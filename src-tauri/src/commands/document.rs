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

/// Progress for a running [`ocr_pdf`], emitted as `ocr-progress` so the chat can show "Reading page 12 of 105"
/// during a long scan. Plain `Serialize`, listened to ad-hoc by the frontend (same as the summarize events).
#[derive(Clone, serde::Serialize)]
struct OcrProgress {
    done: u32,
    total: u32,
}

/// OCR a (scanned) PDF: rasterize each page via pdfium, read text off each, and return the concatenated text.
/// Prefers **Tesseract** (dramatically cleaner on faxed/scanned clinical pages — it read drug names correctly
/// that the bundled `ocrs` engine garbled) and falls back to `ocrs` when Tesseract isn't installed. Streams
/// page-by-page so a 100+ page chart can't exhaust memory, emits `ocr-progress`, and is page-tolerant (a page
/// that fails to read is skipped, not fatal). CPU-bound, so it runs off the async runtime.
#[tauri::command]
#[specta::specta]
pub async fn ocr_pdf(app: tauri::AppHandle, path: String) -> Result<String, String> {
    use tauri::Emitter;

    let p = std::path::PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("File not found: {}", p.display()));
    }
    let app_emit = app.clone();
    let text = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let tess = crate::dotflow::ocr::find_tesseract();
        // Render wider for Tesseract (it wants ~300 DPI); the ocrs engine was tuned around 1600px.
        let target_width: u16 = if tess.is_some() { 2400 } else { 1600 };
        // Build the ocrs engine once, and only when Tesseract isn't available (loading it is expensive).
        let engine = if tess.is_none() {
            Some(crate::dotflow::ocr::load_engine()?)
        } else {
            None
        };
        let pid = std::process::id();

        let pages = crate::dotflow::pdf_render::for_each_page(
            &p,
            target_width,
            crate::dotflow::pdf_render::MAX_PAGES,
            |i, total, img| {
                let _ = app_emit.emit(
                    "ocr-progress",
                    OcrProgress {
                        done: i as u32,
                        total: total as u32,
                    },
                );
                let r = match (&tess, &engine) {
                    (Some(t), _) => {
                        crate::dotflow::ocr::ocr_image_tesseract(t, &img, &format!("{pid}_{i}"))
                    }
                    (None, Some(e)) => crate::dotflow::ocr::ocr_image(e, &img),
                    (None, None) => Err("no OCR engine available".to_string()),
                };
                match r {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("OCR: skipped page {} of {}: {e}", i + 1, total);
                        String::new()
                    }
                }
            },
        )?;

        let ok = pages.iter().filter(|s| !s.trim().is_empty()).count();
        if ok == 0 {
            return Err("OCR couldn't read any text from this document.".to_string());
        }
        let mut joined = pages
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim())
            .collect::<Vec<_>>()
            .join("\n\n");
        let skipped = pages.len() - ok;
        if skipped > 0 {
            joined.push_str(&format!(
                "\n\n[Note: {skipped} of {} page(s) couldn't be read and were skipped.]",
                pages.len()
            ));
        }
        let _ = app_emit.emit(
            "ocr-progress",
            OcrProgress {
                done: pages.len() as u32,
                total: pages.len() as u32,
            },
        );
        Ok(joined)
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
/// REDUCE step can synthesize from facts rather than re-reading the raw text. (`/no_think` is appended to the
/// USER turn, not here — reasoning models like Qwen3/Qwythos only honor the switch in the last user message.)
#[cfg(feature = "local-llm")]
const EXTRACT_SYSTEM: &str = "You are extracting information from ONE PORTION of a longer clinical document, \
     to be combined with the other portions later. Faithfully capture EVERYTHING clinically relevant in this \
     portion, in the order it appears: dates and events (admission, procedures, transfers, consults), the \
     reason for admission and history of the presenting problem, the hospital course and how things changed \
     over time, symptoms, exam and mental-status findings, diagnoses/problems, lab and imaging results, \
     medications (with dose, route, frequency, and start/stop dates if stated), and therapy/functional status. \
     Use ONLY what the text states — do not infer, summarize away, or invent. Preserve numbers, dates, and \
     names verbatim. Be thorough — it is better to include a detail than to drop it. Output only the captured \
     information.";

/// Append `/no_think` to a reasoning model's USER turn so it answers directly instead of emitting a `<think>`
/// pass. Qwen3/Qwythos only honor the switch in the last user message (not the system prompt) — putting it in
/// the system prompt was why Qwythos burned its whole budget thinking and returned an empty summary.
#[cfg(feature = "local-llm")]
fn with_no_think(user: &str) -> String {
    format!("{user}\n\n/no_think")
}

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
    // 4096 (was 2048): a dense clinical page holds more than 2048 tokens of facts, so a small budget TRUNCATED
    // each chunk's extraction — dropping content and leaving the final summary with an incomplete course. A
    // 22k-char (~7k-token) chunk + 4096 output still fits the engine's 16k context with margin. Extraction
    // usually compresses below this, so it only stops early on EOS; it just no longer clips a dense page.
    let out = crate::dotflow::local_llm::generate_chat(
        model_path,
        EXTRACT_SYSTEM,
        &with_no_think(chunk),
        4096,
    )?;
    Ok(crate::commands::ai::strip_reasoning(&out)
        .trim()
        .to_string())
}

/// The clinical SYSTEM prompt for the REDUCE step. Grounds output in the facts, honors the user's requested
/// format (a narrative HPI, a problem/assessment list, etc.), and — when an assessment list is requested —
/// labels each problem Documented vs. Suspected, cites evidence, gives a plan, and flags any ICD-10 code as
/// AI-suggested-verify (a local model is NOT a reliable coder). `{instruction}` is the user's request.
#[cfg(feature = "local-llm")]
fn synth_system(instruction: &str) -> String {
    format!(
        "You are an expert clinical documentation assistant. You are given clinical FACTS extracted, in order, \
         from every part of a patient's record. Produce the document the user requests below, grounded ONLY in \
         these facts.\n\n\
         Rules:\n\
         - Use only what the facts support. Do NOT invent findings, values, medications, diagnoses, or dates. \
         If something the request needs is not in the facts, state that it is not documented rather than \
         guessing.\n\
         - Default to a clear, chronological clinical narrative in complete sentences and paragraphs (not a \
         run-on sentence). Cover the FULL course from start to finish, not just the beginning. Use sections or \
         lists only where the request calls for them.\n\
         - If the request asks for an assessment/problem list: give each problem its own entry; tag it \
         [Documented] when the record states the diagnosis or [Suspected] when you are inferring it from \
         evidence; include the supporting EVIDENCE, a brief PLAN, and — only if the request asks for codes — a \
         SUGGESTED ICD-10 code written as 'ICD-10 (suggested, verify): <code>'. You are not a reliable coder, \
         so never present a code as authoritative.\n\
         - Output the finished document directly: no preamble, no meta-commentary about your process.\n\n\
         Request: {instruction}"
    )
}

/// Run one REDUCE pass on `model`, returning the cleaned (reasoning-stripped) output. `/no_think` rides the
/// user turn. 8192-token budget: a reasoning model that still thinks needs room for the `<think>` pass AND the
/// answer, or it strips to empty; the context caps generation to the room after the prompt regardless.
#[cfg(feature = "local-llm")]
fn synth_once(model: &std::path::Path, system: &str, facts: &str) -> Result<String, String> {
    let out = crate::dotflow::local_llm::generate_chat(model, system, &with_no_think(facts), 8192)?;
    Ok(crate::commands::ai::strip_reasoning(&out)
        .trim()
        .to_string())
}

/// REDUCE: synthesize the ordered `facts` into the user's requested output. Tries `primary` (the capable chat
/// model — better clinical narrative) first; if it returns only reasoning / nothing, falls back to `fallback`
/// (the fast non-reasoning extraction model) so a summary is still produced. Blocking.
#[cfg(feature = "local-llm")]
fn synthesize(
    primary: &std::path::Path,
    fallback: &std::path::Path,
    facts: &str,
    instruction: &str,
) -> Result<String, String> {
    let system = synth_system(instruction);

    let first = synth_once(primary, &system, facts)?;
    if !first.is_empty() {
        return Ok(first);
    }
    // Primary produced only reasoning (or nothing) → fall back to the non-reasoning model, which reliably
    // emits an answer. Only worth it if the fallback is a different model.
    if fallback != primary {
        let second = synth_once(fallback, &system, facts)?;
        if !second.is_empty() {
            return Ok(second);
        }
    }
    Err(
        "The model produced only reasoning and no summary. Pick a small non-reasoning model (e.g. Gemma) \
         as the chat model, or set it as the Transforms model in Settings → Text Cleanup."
            .to_string(),
    )
}

/// The map/reduce core: chunk `text`, MAP each chunk to facts (emitting progress), REDUCE facts to the final
/// answer. If the combined facts are still too big for one synthesis, recurse one level (bounded by `depth`).
/// Blocking — the caller runs it off the async runtime.
#[cfg(feature = "local-llm")]
fn run_summary(
    emit: &dyn Fn(DocSummarizeProgress),
    extract_path: &std::path::Path,
    synth_path: &std::path::Path,
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
        let out = synthesize(synth_path, extract_path, text, instruction)?;
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
        let e = extract_chunk(extract_path, chunk)?;
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
        return run_summary(
            emit,
            extract_path,
            synth_path,
            &combined,
            instruction,
            depth + 1,
        );
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

    let out = synthesize(synth_path, extract_path, &facts, instruction)?;
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
        // MAP (per-chunk fact extraction) is mechanical → use the fast "transform" model (e.g. Gemma) when set.
        // REDUCE (weaving the facts into a coherent HPI/narrative) needs real capability → use the chat model
        // (e.g. Qwythos). Each falls back to the other so a single configured model still works.
        let transform_model = settings.model_for_task("transform");
        let chat_model = settings.local_llm_model_path.trim().to_string();
        let extract_model = if transform_model.is_empty() {
            chat_model.clone()
        } else {
            transform_model
        };
        let synth_model = if chat_model.is_empty() {
            extract_model.clone()
        } else {
            chat_model
        };
        if extract_model.is_empty() {
            return Err(
                "No local model selected — pick one in the chat model dropdown.".to_string(),
            );
        }
        let extract_path = std::path::PathBuf::from(&extract_model);
        let synth_path = std::path::PathBuf::from(&synth_model);
        if !extract_path.exists() {
            return Err(format!("Model file not found: {extract_model}"));
        }
        if !synth_path.exists() {
            return Err(format!("Model file not found: {synth_model}"));
        }

        let app_emit = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let emit = |p: DocSummarizeProgress| {
                let _ = app_emit.emit("doc-summarize-progress", p);
            };
            run_summary(&emit, &extract_path, &synth_path, &text, &instruction, 0)
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
