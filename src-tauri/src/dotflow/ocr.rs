//! DotFlow — OCR for scanned PDFs (step 2). Reads text off rasterized pages with the pure-Rust `ocrs` engine
//! (rten runtime). Two model files — `text-detection.rten` + `text-recognition.rten` — ship next to the exe
//! (downloaded from the ocrs project, like pdfium.dll). CPU-based; a handful of clinical pages is fine.
//! See `docs/dotflow-design/ROADMAP.md` §Document ingestion.

use std::path::PathBuf;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;

/// Locate a model file that ships next to the exe (falling back to CWD for dev runs).
fn model_path(name: &str) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        let p = dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    let cwd = PathBuf::from(name);
    if cwd.exists() {
        return Ok(cwd);
    }
    Err(format!(
        "OCR model '{name}' not found next to the app. Scanned-PDF OCR needs the ocrs models installed."
    ))
}

/// Build the OCR engine from the bundled detection + recognition models. Expensive (loads both models), so
/// callers build it ONCE per document and reuse across pages.
pub fn load_engine() -> Result<OcrEngine, String> {
    let det = Model::load_file(model_path("text-detection.rten")?)
        .map_err(|e| format!("Failed to load OCR detection model: {e}"))?;
    let rec = Model::load_file(model_path("text-recognition.rten")?)
        .map_err(|e| format!("Failed to load OCR recognition model: {e}"))?;
    OcrEngine::new(OcrEngineParams {
        detection_model: Some(det),
        recognition_model: Some(rec),
        ..Default::default()
    })
    .map_err(|e| format!("Failed to initialize OCR engine: {e}"))
}

/// OCR a single rasterized page to plain text.
fn ocr_image(engine: &OcrEngine, img: &image::DynamicImage) -> Result<String, String> {
    let rgb = img.to_rgb8();
    let src = ImageSource::from_bytes(rgb.as_raw(), (rgb.width(), rgb.height()))
        .map_err(|e| format!("OCR image prep failed: {e}"))?;
    let input = engine
        .prepare_input(src)
        .map_err(|e| format!("OCR input prep failed: {e}"))?;
    engine
        .get_text(&input)
        .map_err(|e| format!("OCR text extraction failed: {e}"))
}

/// OCR every rasterized page and join into one document, separating pages with blank lines. Page-tolerant: a
/// page that fails to OCR is skipped (logged) rather than discarding the whole document — only an all-pages
/// failure errors. Appends a short note if any pages were skipped.
pub fn ocr_pages(images: &[image::DynamicImage]) -> Result<String, String> {
    let engine = load_engine()?;
    let mut out = String::new();
    let mut ok_pages = 0usize;
    let mut failed = 0usize;
    for (i, img) in images.iter().enumerate() {
        match ocr_image(&engine, img) {
            Ok(text) => {
                if ok_pages > 0 {
                    out.push_str("\n\n");
                }
                out.push_str(text.trim());
                ok_pages += 1;
            }
            Err(e) => {
                log::warn!("OCR: skipped page {} of {}: {e}", i + 1, images.len());
                failed += 1;
            }
        }
    }
    if ok_pages == 0 {
        return Err(format!(
            "OCR couldn't read any of the {} page(s).",
            images.len()
        ));
    }
    if failed > 0 {
        out.push_str(&format!(
            "\n\n[Note: {failed} of {} page(s) couldn't be read and were skipped.]",
            images.len()
        ));
    }
    Ok(out)
}
