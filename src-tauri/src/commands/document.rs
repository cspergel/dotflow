//! DotFlow — document ingestion commands (PDF → text) for the AI chat "attach PDF" feature.
//!
//! Extracts the **text layer** of a PDF locally (pure-Rust `pdf-extract`, fully offline) so the chat can
//! summarize it or answer questions about it. Image-only / scanned PDFs have no text layer and yield an
//! empty result — surfaced as a clear "looks scanned, OCR isn't available yet" message (OCR is a later
//! roadmap step). See `docs/dotflow-design/ROADMAP.md` §Document ingestion.

/// Soft cap on extracted characters so a pathological PDF can't balloon memory or blow past the model's
/// context. ~600k chars ≈ 150k tokens — well within a large-context model, truncated with a note beyond that.
const MAX_CHARS: usize = 600_000;

/// Progress for a running [`summarize_document`], emitted as `doc-summarize-progress` so the chat UI can show
/// "Reading part 2/5…" while the map/reduce runs. Plain `Serialize` (not a specta event) — the frontend listens
/// ad-hoc, same as the chat-token events.
#[derive(Clone, serde::Serialize)]
struct DocSummarizeProgress {
    /// Steps completed so far (0..=total).
    done: u32,
    /// Total steps: one per chunk read + one final synthesis.
    total: u32,
    /// Human-readable current step, e.g. "Reading part 2 of 5" or "Writing the summary".
    stage: String,
}

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

/// Run pdf-extract's text extraction CRASH-SAFELY. It can deeply recurse on malformed/complex PDFs (nested
/// fonts / XObjects) and overflow the default small thread stack — an UNCATCHABLE process abort that would
/// take down the whole app. Isolate it on a dedicated 256 MB-stack thread and catch panics, so a bad PDF
/// returns a graceful error instead. (Observed: a text PDF with `Helvetica-Bold` / `.notdef` glyphs crashed.)
fn extract_pdf_text_safe(path: std::path::PathBuf) -> Result<String, String> {
    let handle = std::thread::Builder::new()
        .name("pdf-extract".into())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pdf_extract::extract_text(&path)
            }))
        })
        .map_err(|e| format!("Failed to start the PDF reader: {e}"))?;
    match handle.join() {
        Ok(Ok(Ok(text))) => Ok(text),
        Ok(Ok(Err(e))) => Err(format!("Couldn't read this PDF: {e}")),
        Ok(Err(_)) => Err(
            "This PDF couldn't be read — its internal structure broke the text parser. Re-export/print it \
             to a fresh PDF, or if it's scanned use the OCR option."
                .to_string(),
        ),
        Err(_) => Err("The PDF reader failed unexpectedly.".to_string()),
    }
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
    let text = tauri::async_runtime::spawn_blocking(move || extract_pdf_text_safe(p_text))
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

/// Split `text` into chunks of at most ~`max_chars`, breaking at line boundaries so a chunk never cuts a line
/// in half (clinical records are line-oriented — one finding/med/result per line). A single line longer than
/// `max_chars` is hard-split on a char boundary as a last resort. Every character of the input appears in
/// exactly one chunk, in order — the map/reduce summary depends on losing nothing.
fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        // Starting a new chunk keeps whole lines together when the current one would overflow.
        if !cur.is_empty() && cur.len() + line.len() > max_chars {
            chunks.push(std::mem::take(&mut cur));
        }
        if line.len() > max_chars {
            // A single over-long line (e.g. a PDF that lost its newlines) — flush, then hard-split it.
            if !cur.is_empty() {
                chunks.push(std::mem::take(&mut cur));
            }
            let mut rest = line;
            while rest.len() > max_chars {
                let mut idx = max_chars;
                while idx > 0 && !rest.is_char_boundary(idx) {
                    idx -= 1;
                }
                if idx == 0 {
                    // Pathological: a single char wider than max_chars is impossible, but guard anyway.
                    idx = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                }
                chunks.push(rest[..idx].to_string());
                rest = &rest[idx..];
            }
            cur.push_str(rest);
        } else {
            cur.push_str(line);
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// The SYSTEM prompt for the MAP step: pull every fact out of ONE portion of the document, faithfully, so the
/// REDUCE step can synthesize from facts rather than re-reading the raw text. `/no_think` keeps a reasoning
/// model from burning the budget thinking (its output is stripped either way).
#[cfg(feature = "local-llm")]
const EXTRACT_SYSTEM: &str = "You are extracting information from ONE PORTION of a longer document, to be \
     combined with the other portions later. Faithfully list every fact, finding, event, date, name, \
     medication (with dose, route, and frequency if stated), diagnosis, lab/imaging result, and instruction \
     that appears in this portion. Use ONLY what the text states — do not infer, omit, or invent. Preserve \
     numbers and dates verbatim. Be thorough and specific. Output only the list of facts.\n\n/no_think";

/// Character budget per MAP chunk. ~22k chars ≈ 6–8k tokens for dense clinical text (numbers/abbreviations
/// tokenize smaller than prose), leaving comfortable room under the engine's 16k context for the extraction
/// output. Deliberately conservative — a chunk that overflows context would error mid-run.
#[cfg(feature = "local-llm")]
const CHUNK_CHARS: usize = 22_000;

/// If the combined MAP extracts are still larger than this, run another REDUCE level over them (extract-of-
/// extracts) before the final synthesis, so an arbitrarily large document still fits. The synthesis prompt is
/// these facts + a ~4k-token answer, and the engine caps context at 16k — so the facts must stay well under
/// that. ~24k chars of dense extracted facts ≈ 8–9k tokens, leaving comfortable room for the 4k-token answer.
#[cfg(feature = "local-llm")]
const REDUCE_THRESHOLD: usize = 24_000;

/// MAP one chunk → its extracted facts (reasoning stripped). Blocking (runs the local model).
#[cfg(feature = "local-llm")]
fn extract_chunk(model_path: &std::path::Path, chunk: &str) -> Result<String, String> {
    let out = crate::dotflow::local_llm::generate_chat(model_path, EXTRACT_SYSTEM, chunk, 2048)?;
    Ok(crate::commands::ai::strip_reasoning(&out)
        .trim()
        .to_string())
}

/// REDUCE: synthesize the ordered `facts` into the user's requested output (`instruction`). Blocking.
#[cfg(feature = "local-llm")]
fn synthesize(
    model_path: &std::path::Path,
    facts: &str,
    instruction: &str,
) -> Result<String, String> {
    let system = format!(
        "You are given FACTS extracted, in order, from every part of a document. Using ONLY these facts — \
         do not invent, assume, or add anything not present — complete the user's request below. Write a \
         single coherent result, not a list of the parts. Output the result directly with no preamble and no \
         explanation of your process.\n\nRequest: {instruction}\n\n/no_think"
    );
    // 8192 (was 4096): a reasoning model (e.g. Qwythos) that ignores `/no_think` spends tokens thinking before
    // the answer; a small budget gets fully consumed by an unclosed <think>, which strips to empty. A bigger
    // budget lets the reasoning pass AND the answer fit. The context caps generation to the room after the
    // prompt anyway, so this only stops early on EOS. (A non-reasoning transform model like Gemma is still the
    // fast, reliable choice — see the empty-result guidance below.)
    let out = crate::dotflow::local_llm::generate_chat(model_path, &system, facts, 8192)?;
    let cleaned = crate::commands::ai::strip_reasoning(&out)
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return Err(
            "The model produced only reasoning and no summary. This happens with reasoning models \
             (e.g. Qwythos) on long documents. Set a small non-reasoning model (e.g. Gemma) as the \
             Transforms model in Settings → Text Cleanup — it's faster and reliable for summaries."
                .to_string(),
        );
    }
    Ok(cleaned)
}

/// The map/reduce core: chunk `text`, MAP each chunk to facts (emitting progress), REDUCE facts to the final
/// answer. If the combined facts are still too big for one synthesis, recurse one level (bounded by `depth`).
/// Blocking — the caller runs it off the async runtime.
#[cfg(feature = "local-llm")]
fn run_summary(
    emit: &dyn Fn(DocSummarizeProgress),
    model_path: &std::path::Path,
    text: &str,
    instruction: &str,
    depth: u32,
) -> Result<String, String> {
    let chunks = chunk_text(text, CHUNK_CHARS);

    // Small enough to summarize in one shot — no MAP needed.
    if chunks.len() <= 1 {
        emit(DocSummarizeProgress {
            done: 0,
            total: 1,
            stage: "Writing the summary".to_string(),
        });
        let out = synthesize(model_path, text, instruction)?;
        emit(DocSummarizeProgress {
            done: 1,
            total: 1,
            stage: "Done".to_string(),
        });
        return Ok(out);
    }

    let total = chunks.len() as u32 + 1; // one step per chunk + the final synthesis
    let mut extracts: Vec<String> = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        emit(DocSummarizeProgress {
            done: i as u32,
            total,
            stage: format!("Reading part {} of {}", i + 1, chunks.len()),
        });
        let e = extract_chunk(model_path, chunk)?;
        if !e.is_empty() {
            extracts.push(e);
        }
    }
    if extracts.is_empty() {
        return Err("Couldn't extract anything to summarize from this document.".to_string());
    }

    emit(DocSummarizeProgress {
        done: chunks.len() as u32,
        total,
        stage: "Writing the summary".to_string(),
    });
    let combined = extracts.join("\n\n");

    // Still too large for one synthesis pass → reduce again (bounded, so it can't loop forever if a model
    // fails to compress). At the depth cap, synthesize from a truncated set rather than recursing further.
    if combined.len() > REDUCE_THRESHOLD && depth < 2 {
        return run_summary(emit, model_path, &combined, instruction, depth + 1);
    }
    // At the depth cap a still-oversized fact set is truncated (with a note) rather than overflowing the
    // context window — this keeps synthesis from erroring on a pathologically large document.
    let facts: String = if combined.len() > REDUCE_THRESHOLD {
        let mut t: String = combined.chars().take(REDUCE_THRESHOLD).collect();
        t.push_str("\n\n[Some detail omitted to fit the model's context.]");
        t
    } else {
        combined
    };

    let out = synthesize(model_path, &facts, instruction)?;
    emit(DocSummarizeProgress {
        done: total,
        total,
        stage: "Done".to_string(),
    });
    Ok(out)
}

/// Summarize a document too large for the chat context window, via local map/reduce: split into chunks, extract
/// the facts from each (MAP), then synthesize the user's requested output from all the facts (REDUCE). This is
/// how DotFlow turns a whole hospital chart into, e.g., a comprehensive HPI without ever exceeding the stable
/// 16k in-process context. Fully offline. Emits `doc-summarize-progress` as it works.
///
/// `instruction` is what to produce (the user's chat message, e.g. "summarize into a comprehensive HPI for a
/// skilled-nursing admission note"). Uses the per-task "transform" model when set (a fast non-reasoning model
/// like Gemma is ideal here) else the default chat model.
#[tauri::command]
#[specta::specta]
pub async fn summarize_document(
    app: tauri::AppHandle,
    text: String,
    instruction: String,
) -> Result<String, String> {
    #[cfg(not(feature = "local-llm"))]
    {
        let _ = (&app, &text, &instruction);
        return Err("This build was compiled without local model support.".to_string());
    }

    #[cfg(feature = "local-llm")]
    {
        use tauri::Emitter;

        if text.trim().is_empty() {
            return Err("There's no document text to summarize.".to_string());
        }
        let instruction = instruction.trim().to_string();
        let instruction = if instruction.is_empty() {
            "Summarize this document comprehensively, covering all of its sections.".to_string()
        } else {
            instruction
        };

        let settings = crate::settings::get_settings(&app);
        let model = settings.model_for_task("transform");
        if model.is_empty() {
            return Err(
                "No local model selected — pick one in the chat model dropdown.".to_string(),
            );
        }
        let model_path = std::path::PathBuf::from(&model);
        if !model_path.exists() {
            return Err(format!("Model file not found: {model}"));
        }

        let app_emit = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let emit = |p: DocSummarizeProgress| {
                let _ = app_emit.emit("doc-summarize-progress", p);
            };
            run_summary(&emit, &model_path, &text, &instruction, 0)
        })
        .await
        .map_err(|e| format!("summarize task failed: {e}"))?;

        result
    }
}

#[cfg(test)]
mod tests {
    use super::chunk_text;

    /// The load-bearing guarantee: chunking loses nothing. Concatenating the chunks must reproduce the input
    /// exactly (order + every character), or the summary would silently drop content.
    #[test]
    fn chunks_concatenate_back_to_the_original() {
        let text = "line one\nline two\nline three\nline four\n";
        let chunks = chunk_text(text, 12);
        assert!(chunks.len() > 1, "small max should force multiple chunks");
        assert_eq!(
            chunks.concat(),
            text,
            "no character may be lost or reordered"
        );
    }

    /// Chunks break at line boundaries: no chunk (except a forced hard-split) may end mid-line. With a max that
    /// fits ~1 line, each chunk should be a whole line.
    #[test]
    fn breaks_on_line_boundaries_not_mid_line() {
        let text = "aaaa\nbbbb\ncccc\n";
        let chunks = chunk_text(text, 6); // "aaaa\n" = 5 <= 6, adding "bbbb\n" would exceed
        assert_eq!(chunks, vec!["aaaa\n", "bbbb\n", "cccc\n"]);
    }

    /// A single line longer than the max is hard-split (can't keep it whole) but still loses nothing, and each
    /// piece stays within the max.
    #[test]
    fn hard_splits_an_overlong_line_without_loss() {
        let text = "abcdefghijklmnop"; // 16 chars, no newline
        let chunks = chunk_text(text, 5);
        assert_eq!(chunks.concat(), text);
        assert!(
            chunks.iter().all(|c| c.len() <= 5),
            "every hard-split piece must be within the max: {chunks:?}"
        );
    }

    /// A multi-byte char must never be split mid-codepoint (would corrupt UTF-8). Hard-split must land on a
    /// char boundary.
    #[test]
    fn hard_split_respects_utf8_char_boundaries() {
        let text = "héllo wörld ☃ снеговик"; // mixed 1–3 byte chars, no newline
        let chunks = chunk_text(text, 4);
        assert_eq!(chunks.concat(), text, "no loss");
        // If any chunk boundary landed mid-codepoint, .concat() would still match, so assert each chunk is
        // itself valid UTF-8 by round-tripping through bytes (String is always valid, so check reconstruct).
        for c in &chunks {
            assert!(std::str::from_utf8(c.as_bytes()).is_ok());
        }
    }
}
