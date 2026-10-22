//! Image files shown in editor tabs: format detection and decoding, with no
//! egui or filesystem dependency.
use image::{DynamicImage, ImageDecoder, ImageReader, imageops::FilterType};
use std::{io::Cursor, path::Path, sync::Arc};

/// Largest texture edge uploaded to the GPU; bigger images are downscaled.
pub(crate) const MAX_TEXTURE_EDGE: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
    Ico,
    Tiff,
    /// Rasterized from SVG text; never opened from a file as an image tab.
    Svg,
}

impl ImageKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::WebP => "WebP",
            Self::Bmp => "BMP",
            Self::Ico => "ICO",
            Self::Tiff => "TIFF",
            Self::Svg => "SVG",
        }
    }

    fn from_extension(extension: &str) -> Option<Self> {
        Some(match extension.to_ascii_lowercase().as_str() {
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "gif" => Self::Gif,
            "webp" => Self::WebP,
            "bmp" => Self::Bmp,
            "ico" => Self::Ico,
            "tif" | "tiff" => Self::Tiff,
            _ => return None,
        })
    }

    fn from_magic(bytes: &[u8]) -> Option<Self> {
        Some(if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Self::Png
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Self::Jpeg
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Self::Gif
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Self::WebP
        } else if bytes.starts_with(b"BM") {
            Self::Bmp
        } else if bytes.starts_with(&[0, 0, 1, 0]) {
            Self::Ico
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            Self::Tiff
        } else {
            return None;
        })
    }

    fn format(self) -> Option<image::ImageFormat> {
        Some(match self {
            Self::Png => image::ImageFormat::Png,
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Gif => image::ImageFormat::Gif,
            Self::WebP => image::ImageFormat::WebP,
            Self::Bmp => image::ImageFormat::Bmp,
            Self::Ico => image::ImageFormat::Ico,
            Self::Tiff => image::ImageFormat::Tiff,
            Self::Svg => return None,
        })
    }
}

/// Whether the extension says this path should open as an image.
pub(crate) fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(ImageKind::from_extension)
        .is_some()
}

/// Decoded pixels ready for texture upload.
pub(crate) struct DecodedImage {
    pub(crate) kind: ImageKind,
    /// Dimensions of the file, before any texture downscaling.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Dimensions of `rgba`; smaller than `width`x`height` when downscaled.
    pub(crate) texture_width: u32,
    pub(crate) texture_height: u32,
    pub(crate) rgba: Vec<u8>,
    pub(crate) file_size: u64,
}

/// Decode `bytes`, confirming the format from magic bytes rather than the
/// extension. Never panics on corrupt data.
pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedImage, String> {
    let kind = ImageKind::from_magic(bytes).ok_or("Not a recognised image file")?;
    let format = kind.format().ok_or("Not a recognised image file")?;
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|error| error.to_string())?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err("The image has no pixels".into());
    }
    if width.max(height) > MAX_TEXTURE_EDGE {
        image = image.resize(MAX_TEXTURE_EDGE, MAX_TEXTURE_EDGE, FilterType::Triangle);
    }
    Ok(DecodedImage {
        kind,
        width,
        height,
        texture_width: image.width(),
        texture_height: image.height(),
        rgba: image.into_rgba8().into_raw(),
        file_size: bytes.len() as u64,
    })
}

/// "1920 × 1080   245 KB   PNG" for the status bar.
pub(crate) fn summary(image: &DecodedImage) -> String {
    format!(
        "{} × {}   {}   {}",
        image.width,
        image.height,
        format_size(image.file_size),
        image.kind.label()
    )
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{:.1} MB", value / (KB * KB))
    }
}

/// Where an image tab's pixels stand.
pub(crate) enum ImageLoad {
    Loading,
    Ready(Arc<DecodedImage>),
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(format: image::ImageFormat, alpha: bool) -> Vec<u8> {
        let image = if alpha {
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                3,
                2,
                image::Rgba([10, 20, 30, 128]),
            ))
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30])))
        };
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn extensions_select_image_paths_case_insensitively() {
        for name in [
            "a.png", "a.JPG", "a.jpeg", "a.gif", "a.webp", "a.bmp", "a.ico", "a.tif", "a.TIFF",
        ] {
            assert!(is_image_path(Path::new(name)), "{name}");
        }
        for name in ["a.svg", "a.rs", "png", "a.png.txt"] {
            assert!(!is_image_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn every_supported_format_decodes_with_its_magic_bytes() {
        let cases = [
            (image::ImageFormat::Png, ImageKind::Png, true),
            (image::ImageFormat::Jpeg, ImageKind::Jpeg, false),
            (image::ImageFormat::Gif, ImageKind::Gif, true),
            (image::ImageFormat::Bmp, ImageKind::Bmp, true),
            (image::ImageFormat::Tiff, ImageKind::Tiff, true),
            (image::ImageFormat::Ico, ImageKind::Ico, true),
            (image::ImageFormat::WebP, ImageKind::WebP, true),
        ];
        for (format, kind, alpha) in cases {
            let bytes = encode(format, alpha);
            let decoded = decode(&bytes).unwrap_or_else(|error| panic!("{kind:?}: {error}"));
            assert_eq!(decoded.kind, kind);
            assert_eq!((decoded.width, decoded.height), (3, 2));
            assert_eq!(decoded.rgba.len(), 3 * 2 * 4);
            assert_eq!(decoded.file_size, bytes.len() as u64);
        }
    }

    #[test]
    fn sizes_read_in_the_largest_whole_unit() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn alpha_is_preserved() {
        let decoded = decode(&encode(image::ImageFormat::Png, true)).unwrap();
        assert_eq!(&decoded.rgba[..4], &[10, 20, 30, 128]);
    }

    #[test]
    fn corrupt_and_unknown_data_report_errors_instead_of_panicking() {
        assert!(decode(b"").is_err());
        assert!(decode(b"plain text, not an image").is_err());
        let mut truncated = encode(image::ImageFormat::Png, false);
        truncated.truncate(20);
        assert!(decode(&truncated).is_err());
        let mut jpeg = encode(image::ImageFormat::Jpeg, false);
        jpeg.truncate(30);
        assert!(decode(&jpeg).is_err());
    }

    #[test]
    fn oversized_images_are_downscaled_for_upload_but_report_real_size() {
        let image = DynamicImage::ImageLuma8(image::GrayImage::new(MAX_TEXTURE_EDGE + 10, 4));
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        let decoded = decode(out.get_ref()).unwrap();
        assert_eq!((decoded.width, decoded.height), (MAX_TEXTURE_EDGE + 10, 4));
        assert_eq!(decoded.texture_width, MAX_TEXTURE_EDGE);
        assert!(decoded.texture_height >= 1);
        assert_eq!(
            decoded.rgba.len(),
            (decoded.texture_width * decoded.texture_height * 4) as usize
        );
    }
}
