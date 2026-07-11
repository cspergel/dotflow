//! DotFlow — PDF page rasterization via pdfium (step 1 of scanned-PDF OCR). Renders each page to an image so
//! the OCR stage (PaddleOCR/ocrs, added next) can read scanned/image-only PDFs that have no text layer.
//!
//! `pdfium.dll` (Google's PDF renderer, prebuilt from bblanchon/pdfium-binaries) ships next to the exe and is
//! loaded at runtime — pure-Rust bindings, no native compilation. See `docs/dotflow-design/ROADMAP.md`
//! §Document ingestion.

use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;

/// Cap on pages we rasterize in one pass — bounds time for a pathologically long scan. Raised from 60: real
/// clinical charts run past 100 pages (a 105-page chart had pages 61+ silently dropped at 60). Streaming
/// (one page image resident at a time, see [`for_each_page`]) keeps memory flat regardless of this cap.
pub const MAX_PAGES: usize = 400;

/// Bind to the bundled `pdfium.dll` (next to the exe), falling back to a system-installed copy.
fn bind() -> Result<Pdfium, String> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        let lib = dir.join("pdfium.dll");
        if let Ok(b) = Pdfium::bind_to_library(&lib) {
            return Ok(Pdfium::new(b));
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| format!("Could not load pdfium.dll (is it next to the app?): {e}"))
}

/// Render up to `max_pages` pages one at a time, each ~`target_width` px wide (wider = better OCR, slower),
/// calling `f(index, total, image)` per page and collecting the results. STREAMING: only one page image is
/// resident at a time, so a 100+ page scan at OCR DPI can't exhaust RAM (holding them all would). `total` is
/// the number of pages that will actually be processed (min of the doc's page count and `max_pages`). A render
/// failure on a page is fatal (returns `Err`); OCR-level tolerance is the caller's job inside `f`.
pub fn for_each_page<T>(
    pdf_path: &Path,
    target_width: u16,
    max_pages: usize,
    mut f: impl FnMut(usize, usize, image::DynamicImage) -> T,
) -> Result<Vec<T>, String> {
    let pdfium = bind()?;
    let path_str = pdf_path.to_str().ok_or("PDF path is not valid UTF-8")?;
    let doc = pdfium
        .load_pdf_from_file(path_str, None)
        .map_err(|e| format!("Couldn't open PDF for rendering: {e}"))?;

    let cfg = PdfRenderConfig::new().set_target_width(target_width as i32);
    let pages = doc.pages();
    let total = (pages.len() as usize).min(max_pages);
    if total == 0 {
        return Err("The PDF has no pages to render.".to_string());
    }

    let mut out = Vec::with_capacity(total);
    for (i, page) in pages.iter().take(total).enumerate() {
        let img = page
            .render_with_config(&cfg)
            .map_err(|e| format!("Failed to render page {}: {e}", i + 1))?
            .as_image()
            .map_err(|e| format!("Failed to convert page {} to an image: {e}", i + 1))?;
        out.push(f(i, total, img));
    }
    Ok(out)
}
