//! Debug-only screenshot capture for visual checks and before/after baselines.
//!
//! Set `LUMEN_CAPTURE=/path/to/shot.png` (optionally `LUMEN_CAPTURE_FRAMES=n`, the
//! number of frames to let the layout settle, default 20) and launch Lumen on a
//! disposable folder, e.g. `LUMEN_CAPTURE=/tmp/a.png lumen-editor /tmp/fixture`.
//! The window is captured once, written as a PNG, and the app exits. Never point it
//! at a real workspace or profile: the capture shows whatever is on screen.
use eframe::egui;
use std::path::{Path, PathBuf};

const DEFAULT_FRAMES: u32 = 20;

pub(crate) struct Capture {
    path: PathBuf,
    frames_left: u32,
    requested: bool,
}

impl Capture {
    /// A capture request from the environment, if one is set.
    pub(crate) fn from_env() -> Option<Self> {
        Self::from_values(
            std::env::var_os("LUMEN_CAPTURE")?.into(),
            std::env::var("LUMEN_CAPTURE_FRAMES").ok().as_deref(),
        )
    }

    fn from_values(path: PathBuf, frames: Option<&str>) -> Option<Self> {
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(Self {
            path,
            frames_left: frames
                .and_then(|frames| frames.parse().ok())
                .unwrap_or(DEFAULT_FRAMES),
            requested: false,
        })
    }

    /// Call once per frame: counts frames, asks for the screenshot, then saves it
    /// and closes the window when it arrives.
    pub(crate) fn update(&mut self, ctx: &egui::Context) {
        if !self.requested {
            if self.frames_left > 0 {
                self.frames_left -= 1;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.requested = true;
            }
            ctx.request_repaint();
            return;
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&self.path, &image) {
                Ok(()) => crate::services::logger::info(
                    "capture",
                    &format!("Saved {}", self.path.display()),
                ),
                Err(error) => crate::services::logger::error("capture", &error),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }
}

fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    image::save_buffer(
        path,
        image.as_raw(),
        image.size[0] as u32,
        image.size[1] as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_round_trips_its_size_and_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let image = egui::ColorImage::new([3, 2], vec![egui::Color32::from_rgb(10, 20, 30); 6]);
        save_png(&path, &image).unwrap();
        let read = image::open(&path).unwrap().to_rgba8();
        assert_eq!((read.width(), read.height()), (3, 2));
        assert_eq!(read.get_pixel(1, 1).0, [10, 20, 30, 255]);
    }

    #[test]
    fn environment_values_choose_the_path_and_frame_count() {
        assert!(Capture::from_values("".into(), None).is_none());
        let capture = Capture::from_values("/tmp/a.png".into(), Some("5")).unwrap();
        assert_eq!(capture.frames_left, 5);
        let capture = Capture::from_values("/tmp/a.png".into(), Some("nonsense")).unwrap();
        assert_eq!(capture.frames_left, DEFAULT_FRAMES);
    }

    #[test]
    fn the_request_waits_for_the_settle_frames_then_asks_once() {
        let ctx = egui::Context::default();
        let mut capture = Capture::from_values("/tmp/a.png".into(), Some("2")).unwrap();
        for _ in 0..2 {
            capture.update(&ctx);
            assert!(!capture.requested);
        }
        capture.update(&ctx);
        assert!(capture.requested);
    }
}
