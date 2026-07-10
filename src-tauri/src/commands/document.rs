//! DotFlow — document ingestion commands (PDF → text) for the AI chat "attach PDF" feature.
//!
//! Extracts the **text layer** of a PDF locally (pure-Rust `pdf-extract`, fully offline) so the chat can
//! summarize it or answer questions about it. Image-only / scanned PDFs have no text layer and yield an
//! empty result — surfaced as a clear "looks scanned, OCR isn't available yet" message (OCR is a later
//! roadmap step). See `docs/dotflow-design/ROADMAP.md` §Document ingestion.

/// Soft cap on extracted characters so a pathological PDF can't balloon memory or blow past the model's
/// context. ~600k chars ≈ 150k tokens — well within a large-context model, truncated with a note beyond that.
const MAX_CHARS: usize = 600_000;

/// OCR a (scanned) PDF: rasterize each page via pdfium, then read text off each with the ocrs engine, and
/// return the concatenated text. CPU-bound (render + OCR) so it runs off the async runtime. Returns a clear
/// error if the OCR models aren't installed. The trimmed result is empty-checked by the caller.
#[tauri::command]
#[specta::specta]
pub async fn ocr_pdf(path: String) -> Result<String, String> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("File not found: {}", p.display()));
    }
    let text = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let images = crate::dotflow::pdf_render::render_pages(&p, 1600)?;
        crate::dotflow::ocr::ocr_pages(&images)
    })
    .await
    .map_err(|e| format!("OCR task failed: {e}"))??;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("OCR found no readable text in this document.".to_string());
    }
    Ok(trimmed.to_string())
}

/// Extract the text of a PDF at `path`. Runs the (CPU-bound) parse on a blocking thread. Returns the trimmed
/// text, or a user-facing error: file-missing, not-a-PDF, parse failure, or an empty result (scanned PDF).
#[tauri::command]
#[specta::specta]
pub async fn read_pdf_text(path: String) -> Result<String, String> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("File not found: {}", p.display()));
    }
    let is_pdf = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        == Some(true);
    if !is_pdf {
        return Err("Not a PDF file.".to_string());
    }

    let p_text = p.clone();
    let text = tauri::async_runtime::spawn_blocking(move || {
        pdf_extract::extract_text(&p_text).map_err(|e| format!("Couldn't read this PDF: {e}"))
    })
    .await
    .map_err(|e| format!("PDF read task failed: {e}"))??;

    let trimmed = text.trim();
    let chars = trimmed.chars().count();
    if trimmed.is_empty() {
        return Err(
            "No selectable text found — this is a scanned PDF (images of text). Reading scanned \
             documents needs OCR, which isn't available in DotFlow yet."
                .to_string(),
        );
    }

    // Detect a scanned/image PDF that carries only scraps of embedded text (page numbers, form labels):
    // very low text density per page. Without this, we'd "summarize" a few hundred characters of noise.
    let page_count = tauri::async_runtime::spawn_blocking(move || {
        lopdf::Document::load(&p)
            .ok()
            .map(|d| d.get_pages().len())
            .unwrap_or(0)
    })
    .await
    .unwrap_or(0);
    if page_count >= 3 && chars < page_count * 120 {
        return Err(format!(
            "This looks like a scanned document — only {chars} characters of text across {page_count} \
             pages (~{} per page). Reading scanned PDFs needs OCR, which isn't available in DotFlow yet.",
            chars / page_count.max(1)
        ));
    }

    // Truncate a very large document (with a visible note) so it can't overflow the context window.
    if chars > MAX_CHARS {
        let mut out: String = trimmed.chars().take(MAX_CHARS).collect();
        out.push_str("\n\n[Document truncated — it was too long to include in full.]");
        return Ok(out);
    }
    Ok(trimmed.to_string())
}
