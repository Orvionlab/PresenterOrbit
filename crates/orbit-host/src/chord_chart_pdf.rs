use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

const MAX_PAGES: usize = 64;
const RASTER_EDGE: f32 = 2048.0;
const MAX_TOTAL_PIXEL_BYTES: u64 = 128 * 1024 * 1024;

pub fn pdf_pages(bytes: &[u8]) -> Result<Vec<image::DynamicImage>, String> {
    let mut total = 0u64;
    let mut images = Vec::new();
    for_each_pdf_page(
        bytes,
        "PDF chord chart",
        MAX_PAGES,
        |width, height| {
            let scale = RASTER_EDGE / width.max(height);
            let size = (
                (width * scale).round().max(1.0) as u32,
                (height * scale).round().max(1.0) as u32,
            );
            total += u64::from(size.0) * u64::from(size.1) * 4;
            if total > MAX_TOTAL_PIXEL_BYTES {
                return Err("PDF chord chart raster pages exceed 128 MiB.".into());
            }
            Ok(size)
        },
        |_, _, image| {
            images.push(image::DynamicImage::ImageRgba8(image));
            Ok(())
        },
    )?;
    Ok(images)
}

/// Rasterizes each page of a PDF over white at the size `fit` chooses, handing `(index, page count, image)` to
/// `each` one page at a time so long documents are never held in memory together. `kind` names the document in
/// error messages.
pub fn for_each_pdf_page(
    bytes: &[u8],
    kind: &str,
    max_pages: usize,
    mut fit: impl FnMut(f32, f32) -> Result<(u32, u32), String> + Send,
    mut each: impl FnMut(usize, usize, image::RgbaImage) -> Result<(), String> + Send,
) -> Result<usize, String> {
    // hayro recurses deeply on complex content streams, so it runs on a thread with a large stack.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("pdf-pages".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, || {
                rasterize(bytes, kind, max_pages, &mut fit, &mut each)
            })
            .map_err(|e| format!("{kind} worker could not start: {e}"))?
            .join()
            .map_err(|_| format!("{kind} renderer failed on this document."))?
    })
}

fn rasterize(
    bytes: &[u8],
    kind: &str,
    max_pages: usize,
    fit: &mut dyn FnMut(f32, f32) -> Result<(u32, u32), String>,
    each: &mut dyn FnMut(usize, usize, image::RgbaImage) -> Result<(), String>,
) -> Result<usize, String> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|error| match error {
        LoadPdfError::Decryption(_) => {
            format!("Encrypted or password-protected {kind}s are not supported.")
        }
        LoadPdfError::Invalid => format!("{kind} is corrupt or unreadable."),
    })?;
    let pages = pdf.pages();
    if pages.is_empty() || pages.len() > max_pages {
        return Err(format!("{kind}s must contain 1 to {max_pages} pages."));
    }
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    for (index, page) in pages.iter().enumerate() {
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err("PDF page dimensions are invalid.".into());
        }
        let (target_width, target_height) = fit(width, height)?;
        let pixmap = render(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: target_width as f32 / width,
                y_scale: target_height as f32 / height,
                bg_color: WHITE,
            },
        );
        let image = image::RgbaImage::from_raw(
            u32::from(pixmap.width()),
            u32::from(pixmap.height()),
            pixmap.data_as_u8_slice().to_vec(),
        )
        .ok_or("PDF page raster has an unexpected size.")?;
        each(index, pages.len(), image)?;
    }
    Ok(pages.len())
}
