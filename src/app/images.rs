//! Image tabs: background loading, reload on disk changes, and per-tab zoom/pan.
use super::LumenApp;
use crate::model::{
    DocumentId,
    image_asset::{DecodedImage, ImageLoad},
};
use crate::services::image_files::{self, FileStamp};
use eframe::egui;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

pub(crate) const MIN_SCALE: f32 = 0.02;
pub(crate) const MAX_SCALE: f32 = 32.0;
const STEPS: [f32; 16] = [
    0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ImageZoom {
    /// Scale to fit the viewport, never enlarging past 100%.
    Fit,
    Scale(f32),
}

/// Zoom, pan and the uploaded texture of one image tab.
pub(crate) struct ImageViewState {
    pub(crate) zoom: ImageZoom,
    /// Pan of the image centre from the viewport centre, in screen points.
    pub(crate) offset: egui::Vec2,
    /// Texture for the pixels it was uploaded from (identified by allocation).
    pub(crate) texture: Option<(usize, egui::TextureHandle)>,
}

impl Default for ImageViewState {
    fn default() -> Self {
        Self {
            zoom: ImageZoom::Fit,
            offset: egui::Vec2::ZERO,
            texture: None,
        }
    }
}

impl ImageViewState {
    /// Scale actually drawn for an image of `size` in a viewport of `available`.
    pub(crate) fn scale(&self, size: egui::Vec2, available: egui::Vec2) -> f32 {
        match self.zoom {
            ImageZoom::Fit => fit_scale(size, available),
            ImageZoom::Scale(scale) => scale,
        }
    }

    pub(crate) fn set_scale(&mut self, scale: f32) {
        self.zoom = ImageZoom::Scale(scale.clamp(MIN_SCALE, MAX_SCALE));
    }

    pub(crate) fn zoom_in(&mut self, current: f32) {
        let next = STEPS
            .iter()
            .copied()
            .find(|step| *step > current * 1.001)
            .unwrap_or(MAX_SCALE);
        self.set_scale(next);
    }

    pub(crate) fn zoom_out(&mut self, current: f32) {
        let next = STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < current * 0.999)
            .unwrap_or(MIN_SCALE);
        self.set_scale(next);
    }

    pub(crate) fn actual_size(&mut self) {
        self.zoom = ImageZoom::Scale(1.0);
        self.offset = egui::Vec2::ZERO;
    }

    pub(crate) fn fit(&mut self) {
        self.zoom = ImageZoom::Fit;
        self.offset = egui::Vec2::ZERO;
    }

    /// Zoom to `scale` keeping the image point under `anchor` (relative to the
    /// viewport centre) where it is.
    pub(crate) fn zoom_at(&mut self, current: f32, scale: f32, anchor: egui::Vec2) {
        let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        let ratio = scale / current;
        self.offset = anchor - (anchor - self.offset) * ratio;
        self.zoom = ImageZoom::Scale(scale);
    }
}

pub(crate) fn fit_scale(size: egui::Vec2, available: egui::Vec2) -> f32 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return 1.0;
    }
    (available.x / size.x)
        .min(available.y / size.y)
        .clamp(MIN_SCALE, 1.0)
}

struct Job {
    document: DocumentId,
    receiver: Receiver<Result<DecodedImage, String>>,
}

/// Pending decodes and the on-disk version each open image tab shows.
#[derive(Default)]
pub(crate) struct ImageLoader {
    jobs: Vec<Job>,
    stamps: HashMap<DocumentId, Option<FileStamp>>,
    checked_at: Option<Instant>,
}

impl ImageLoader {
    /// No decode is running and none is waiting to be picked up.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Make the next poll compare disk stamps instead of waiting out the interval.
    #[cfg(test)]
    pub(crate) fn force_check(&mut self) {
        self.checked_at = None;
    }
}

impl LumenApp {
    pub(crate) fn start_image_load(&mut self, document: DocumentId) {
        let Some(path) = self
            .editor
            .index_of(document)
            .and_then(|index| self.editor.files[index].path.clone())
        else {
            return;
        };
        self.images
            .stamps
            .insert(document, image_files::stamp(&path).ok());
        self.images.jobs.retain(|job| job.document != document);
        self.images.jobs.push(Job {
            document,
            receiver: image_files::spawn_load(path),
        });
    }

    pub(crate) fn poll_images(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.images
            .jobs
            .retain(|job| match job.receiver.try_recv() {
                Ok(result) => {
                    finished.push((job.document, result));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    finished.push((
                        job.document,
                        Err("Image loading stopped unexpectedly".into()),
                    ));
                    false
                }
            });
        for (document, result) in finished {
            let Some(file) = self
                .editor
                .index_of(document)
                .map(|index| &mut self.editor.files[index])
                .filter(|file| file.is_image())
            else {
                continue;
            };
            file.image = Some(match result {
                Ok(image) => ImageLoad::Ready(Arc::new(image)),
                Err(error) => ImageLoad::Failed(error),
            });
        }
        let open: Vec<DocumentId> = self
            .editor
            .files
            .iter()
            .filter(|file| file.is_image())
            .map(|file| file.id)
            .collect();
        self.images.stamps.retain(|id, _| open.contains(id));
        self.images.jobs.retain(|job| open.contains(&job.document));
        self.view.image_views.retain(|id, _| open.contains(id));
        if !self.images.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.reload_changed_images(ctx);
    }

    /// Reload image tabs whose file changed on disk; a vanished file keeps its
    /// pixels on screen and is flagged like a text tab.
    fn reload_changed_images(&mut self, ctx: &egui::Context) {
        if self.images.stamps.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(2));
        let now = Instant::now();
        if self
            .images
            .checked_at
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return;
        }
        self.images.checked_at = Some(now);
        let mut reload = Vec::new();
        for file in self.editor.files.iter_mut().filter(|file| file.is_image()) {
            let Some(path) = &file.path else { continue };
            match image_files::stamp(path) {
                Ok(current) => {
                    file.disk_changed = false;
                    if self.images.stamps.get(&file.id) != Some(&Some(current)) {
                        reload.push(file.id);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !file.disk_changed {
                        self.view.status = format!("{} was deleted or moved on disk", file.name);
                    }
                    file.disk_changed = true;
                }
                Err(_) => {}
            }
        }
        for document in reload {
            self.start_image_load(document);
        }
    }

    /// Replace a failed or unwanted image tab by the file's text.
    pub(crate) fn open_image_as_text(&mut self, document: DocumentId) {
        let Some(index) = self.editor.index_of(document) else {
            return;
        };
        let Some(path) = self.editor.files[index].path.clone() else {
            return;
        };
        match self.file_system.read_text(&path) {
            Ok(text) => {
                self.editor.files[index].open_image_as_text(text);
                self.images.stamps.remove(&document);
                self.images.jobs.retain(|job| job.document != document);
                self.view.image_views.remove(&document);
                self.view.status = "Opened as text".into();
            }
            Err(error) => self.view.status = format!("Could not open as text: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_never_enlarges_and_shrinks_to_the_tighter_edge() {
        let size = egui::vec2(400.0, 200.0);
        assert_eq!(fit_scale(size, egui::vec2(800.0, 800.0)), 1.0);
        assert_eq!(fit_scale(size, egui::vec2(200.0, 800.0)), 0.5);
        assert_eq!(fit_scale(size, egui::vec2(800.0, 50.0)), 0.25);
        assert_eq!(fit_scale(size, egui::vec2(0.0, 0.0)), MIN_SCALE);
    }

    #[test]
    fn zoom_steps_walk_the_ladder_from_any_scale() {
        let mut view = ImageViewState::default();
        view.zoom_in(0.6);
        assert_eq!(view.zoom, ImageZoom::Scale(0.75));
        view.zoom_in(1.0);
        assert_eq!(view.zoom, ImageZoom::Scale(1.5));
        view.zoom_out(1.0);
        assert_eq!(view.zoom, ImageZoom::Scale(0.75));
        view.zoom_out(0.04);
        assert_eq!(view.zoom, ImageZoom::Scale(MIN_SCALE));
        view.zoom_in(MAX_SCALE);
        assert_eq!(view.zoom, ImageZoom::Scale(MAX_SCALE));
    }

    #[test]
    fn zooming_at_a_point_keeps_the_image_point_under_it() {
        let mut view = ImageViewState::default();
        let anchor = egui::vec2(30.0, -10.0);
        view.zoom_at(1.0, 2.0, anchor);
        // The image point under the anchor was (anchor - offset) / scale.
        assert_eq!((anchor - view.offset) / 2.0, anchor);
        assert_eq!(view.zoom, ImageZoom::Scale(2.0));
    }

    #[test]
    fn actual_size_and_fit_reset_the_pan() {
        let mut view = ImageViewState {
            offset: egui::vec2(5.0, 5.0),
            ..Default::default()
        };
        view.actual_size();
        assert_eq!(
            (view.zoom, view.offset),
            (ImageZoom::Scale(1.0), egui::Vec2::ZERO)
        );
        view.offset = egui::vec2(5.0, 5.0);
        view.fit();
        assert_eq!((view.zoom, view.offset), (ImageZoom::Fit, egui::Vec2::ZERO));
    }
}
