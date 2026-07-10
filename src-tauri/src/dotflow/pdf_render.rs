//! DotFlow — PDF page rasterization via pdfium (step 1 of scanned-PDF OCR). Renders each page to an image so
//! the OCR stage (PaddleOCR/ocrs, added next) can read scanned/image-only PDFs that have no text layer.
//!
//! `pdfium.dll` (Google's PDF renderer, prebuilt from bblanchon/pdfium-binaries) ships next to the exe and is
//! loaded at runtime — pure-Rust bindings, no native compilation. See `docs/dotflow-design/ROADMAP.md`
//! §Document ingestion.

use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;

/// Cap on pages we rasterize in one pass — bounds time/memory for a pathologically long scan.
pub const MAX_PAGES: usize = 60;

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

/// Render each page of the PDF to a `DynamicImage` ~`target_width` px wide (wider = better OCR, slower).
/// Capped at [`MAX_PAGES`].
pub fn render_pages(
    pdf_path: &Path,
    target_width: u16,
) -> Result<Vec<image::DynamicImage>, String> {
    let pdfium = bind()?;
    let path_str = pdf_path.to_str().ok_or("PDF path is not valid UTF-8")?;
    let doc = pdfium
        .load_pdf_from_file(path_str, None)
        .map_err(|e| format!("Couldn't open PDF for rendering: {e}"))?;

    let cfg = PdfRenderConfig::new().set_target_width(target_width as i32);
    let mut out = Vec::new();
    for page in doc.pages().iter().take(MAX_PAGES) {
        let img = page
            .render_with_config(&cfg)
            .map_err(|e| format!("Failed to render a page: {e}"))?
            .as_image()
            .map_err(|e| format!("Failed to convert a rendered page to an image: {e}"))?;
        out.push(img);
    }
    if out.is_empty() {
        return Err("The PDF has no pages to render.".to_string());
    }
    Ok(out)
}
