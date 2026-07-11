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

/// Locate a Tesseract executable, preferred over the bundled `ocrs` engine because it reads faxed/scanned
/// clinical pages far more accurately (verified: `ocrs` garbled drug names that Tesseract read cleanly).
/// Order: `DOTFLOW_TESSERACT` env → common install paths → a bare `tesseract` on PATH (probed with
/// `--version`). Returns `None` when no working Tesseract is found, so the caller falls back to `ocrs`.
pub fn find_tesseract() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("DOTFLOW_TESSERACT") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    for c in [
        r"C:\Program Files\Tesseract-OCR\tesseract.exe",
        r"C:\Program Files (x86)\Tesseract-OCR\tesseract.exe",
        "/usr/bin/tesseract",
        "/usr/local/bin/tesseract",
        "/opt/homebrew/bin/tesseract",
    ] {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to a bare name resolved via PATH — only if it actually runs.
    let bare = PathBuf::from("tesseract");
    if tesseract_runs(&bare) {
        return Some(bare);
    }
    None
}

/// True if `tesseract --version` runs successfully (used to confirm a PATH-resolved binary really exists).
fn tesseract_runs(tess: &std::path::Path) -> bool {
    let mut cmd = std::process::Command::new(tess);
    cmd.arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    no_window(&mut cmd);
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// On Windows, suppress the console window that would otherwise flash for each Tesseract invocation.
#[cfg(windows)]
fn no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn no_window(_cmd: &mut std::process::Command) {}

/// OCR a single rasterized page with Tesseract: write it to a temp PNG, run `tesseract <png> stdout`, and
/// return the recognized text. `tag` makes the temp filename unique per page/process so concurrent/looped
/// calls don't collide. The temp file is always cleaned up.
pub fn ocr_image_tesseract(
    tess: &std::path::Path,
    img: &image::DynamicImage,
    tag: &str,
) -> Result<String, String> {
    let tmp = std::env::temp_dir().join(format!("dotflow_ocr_{tag}.png"));
    img.save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| format!("OCR temp image write failed: {e}"))?;

    let mut cmd = std::process::Command::new(tess);
    cmd.arg(&tmp)
        .arg("stdout")
        .args(["-l", "eng", "--psm", "3"]);
    no_window(&mut cmd);
    let result = cmd.output();
    let _ = std::fs::remove_file(&tmp);

    let out = result.map_err(|e| format!("Failed to run Tesseract: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Tesseract failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// OCR a single rasterized page to plain text.
pub fn ocr_image(engine: &OcrEngine, img: &image::DynamicImage) -> Result<String, String> {
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

// (Page iteration + page-tolerance now live in `commands::document::ocr_pdf`, which streams pages via
// `pdf_render::for_each_page` and picks Tesseract vs. this `ocrs` engine per page.)
