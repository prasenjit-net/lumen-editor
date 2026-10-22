//! SVG text rasterized for the preview tab, with no egui or filesystem
//! dependency. Parsing is sandboxed: text, system fonts and raster images are
//! compiled out, and external image references are never resolved.
use super::image_asset::{DecodedImage, ImageKind};
use resvg::{tiny_skia, usvg};

/// Longest edge rasterized, so a high zoom cannot allocate an unbounded bitmap.
const MAX_RASTER_EDGE: f32 = 4096.0;
/// Most pixels rasterized in one pass (64 MB of RGBA).
const MAX_RASTER_PIXELS: f32 = 16.0 * 1024.0 * 1024.0;

/// A parsed SVG, rasterizable at any scale.
pub(crate) struct SvgImage {
    tree: usvg::Tree,
    source_len: u64,
}

impl SvgImage {
    /// Parse SVG text. Invalid, empty and zero-sized documents are errors.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Err("The SVG is empty".into());
        }
        let tree = usvg::Tree::from_str(text, &options()).map_err(|error| error.to_string())?;
        let size = tree.size();
        if !(size.width() >= 1.0 && size.height() >= 1.0) {
            return Err("The SVG has no visible size".into());
        }
        Ok(Self {
            tree,
            source_len: text.len() as u64,
        })
    }

    /// Intrinsic size in CSS pixels, at least 1x1.
    pub(crate) fn size(&self) -> (u32, u32) {
        let size = self.tree.size();
        (size.width().ceil() as u32, size.height().ceil() as u32)
    }

    /// Rasterize at `scale` device pixels per SVG pixel, reduced if the bitmap
    /// would exceed the size limits. The result keeps the intrinsic size in
    /// `width`/`height` so the image viewer lays it out at 100% unchanged.
    pub(crate) fn render(&self, scale: f32) -> Result<DecodedImage, String> {
        let (width, height) = self.size();
        let (w, h) = (width as f32, height as f32);
        let limit = (MAX_RASTER_EDGE / w.max(h)).min((MAX_RASTER_PIXELS / (w * h)).sqrt());
        let scale = if scale.is_finite() { scale } else { 1.0 };
        let scale = scale.min(limit).max(f32::MIN_POSITIVE);
        let texture_width = (w * scale).ceil().max(1.0) as u32;
        let texture_height = (h * scale).ceil().max(1.0) as u32;
        let mut pixmap = tiny_skia::Pixmap::new(texture_width, texture_height)
            .ok_or("The SVG is too large to display")?;
        let size = self.tree.size();
        let transform = tiny_skia::Transform::from_scale(
            texture_width as f32 / size.width(),
            texture_height as f32 / size.height(),
        );
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        Ok(DecodedImage {
            kind: ImageKind::Svg,
            width,
            height,
            texture_width,
            texture_height,
            rgba: pixmap.take_demultiplied(),
            file_size: self.source_len,
        })
    }
}

/// Parser options that load nothing from outside the document.
fn options() -> usvg::Options<'static> {
    let mut options = usvg::Options::default();
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
        <rect width="10" height="10" fill="#ff0000"/></svg>"##;

    #[test]
    fn valid_svg_renders_with_transparency_at_its_intrinsic_size() {
        let image = SvgImage::parse(SQUARE).unwrap();
        assert_eq!(image.size(), (20, 10));
        let decoded = image.render(1.0).unwrap();
        assert_eq!((decoded.width, decoded.height), (20, 10));
        assert_eq!((decoded.texture_width, decoded.texture_height), (20, 10));
        assert_eq!(decoded.kind, ImageKind::Svg);
        assert_eq!(&decoded.rgba[..4], &[255, 0, 0, 255]);
        let right = 15 * 4;
        assert_eq!(
            decoded.rgba[right + 3],
            0,
            "outside the rect is transparent"
        );
    }

    #[test]
    fn higher_scales_rasterize_more_pixels_up_to_the_limits() {
        let image = SvgImage::parse(SQUARE).unwrap();
        let sharp = image.render(4.0).unwrap();
        assert_eq!((sharp.texture_width, sharp.texture_height), (80, 40));
        assert_eq!((sharp.width, sharp.height), (20, 10));
        let capped = image.render(1.0e6).unwrap();
        assert!(capped.texture_width as f32 <= MAX_RASTER_EDGE);
        assert!(capped.texture_width as f32 * capped.texture_height as f32 <= MAX_RASTER_PIXELS);
        assert_eq!(
            capped.rgba.len(),
            (capped.texture_width * capped.texture_height * 4) as usize
        );
        assert!(image.render(f32::NAN).is_ok());
    }

    #[test]
    fn invalid_empty_and_zero_sized_svg_report_errors() {
        for text in [
            "",
            "   \n",
            "not xml at all",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"",
            "<html></html>",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"0\" height=\"0\"/>",
        ] {
            assert!(SvgImage::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn external_file_images_are_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("pixel.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255]))
            .save(&png)
            .unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="4"><image width="4" height="4" xlink:href="{}"/></svg>"#,
            png.display()
        );
        let decoded = SvgImage::parse(&svg).unwrap().render(1.0).unwrap();
        assert!(decoded.rgba.chunks(4).all(|pixel| pixel[3] == 0));
    }
}
