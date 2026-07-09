//! DotFlow — document ingestion commands (PDF → text) for the AI chat "attach PDF" feature.
//!
//! Extracts the **text layer** of a PDF locally (pure-Rust `pdf-extract`, fully offline) so the chat can
//! summarize it or answer questions about it. Image-only / scanned PDFs have no text layer and yield an
//! empty result — surfaced as a clear "looks scanned, OCR isn't available yet" message (OCR is a later
//! roadmap step). See `docs/dotflow-design/ROADMAP.md` §Document ingestion.

/// Soft cap on extracted characters so a pathological PDF can't balloon memory or blow past the model's
/// context. ~600k chars ≈ 150k tokens — well within a large-context model, truncated with a note beyond that.
const MAX_CHARS: usize = 600_000;

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

    let text = tauri::async_runtime::spawn_blocking(move || {
        pdf_extract::extract_text(&p).map_err(|e| format!("Couldn't read this PDF: {e}"))
    })
    .await
    .map_err(|e| format!("PDF read task failed: {e}"))??;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(
            "No selectable text found — this looks like a scanned PDF. OCR for scanned documents \
             isn't available yet."
                .to_string(),
        );
    }

    // Truncate a very large document (with a visible note) so it can't overflow the context window.
    if trimmed.chars().count() > MAX_CHARS {
        let mut out: String = trimmed.chars().take(MAX_CHARS).collect();
        out.push_str("\n\n[Document truncated — it was too long to include in full.]");
        return Ok(out);
    }
    Ok(trimmed.to_string())
}
