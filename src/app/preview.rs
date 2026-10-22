//! Preview tabs: a read-only rendered view of another document, kept live from
//! the source buffer and tracked by the source's stable document ID.
use super::{LumenApp, images::ImageViewState, view_state::ViewState};
use crate::{
    model::{
        Document, DocumentId, EditorState,
        image_asset::DecodedImage,
        markdown::{self, Block, LinkTarget},
        previewable::AlternateView,
        svg::SvgImage,
    },
    services::image_files,
};
use eframe::egui;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
    time::Duration,
};

/// Local image referenced by a preview, loaded off the UI thread.
pub(crate) enum PreviewImage {
    Loading,
    Ready {
        image: Arc<DecodedImage>,
        texture: Option<egui::TextureHandle>,
    },
    Failed,
}

/// One SVG preview tab: the source parsed for a revision, the latest raster of
/// it, and the viewer's zoom and pan.
pub(crate) struct SvgPreview {
    revision: u64,
    parsed: Result<SvgImage, String>,
    /// Raster scale and pixels; redone when the zoom needs a sharper bitmap.
    raster: Option<(f32, Arc<DecodedImage>)>,
    pub(crate) view: ImageViewState,
}

/// Scale to rasterize an SVG at for `wanted` device pixels per SVG pixel:
/// rounded up to half-octave steps so smooth zooming does not re-render on
/// every frame, while the bitmap is never coarser than the display needs.
pub(crate) fn raster_scale(wanted: f32) -> f32 {
    let wanted = if wanted.is_finite() { wanted } else { 1.0 };
    2f32.powf((wanted.clamp(0.01, 1.0e4).log2() * 2.0).ceil() / 2.0)
}

/// Preview state that is not part of any document.
#[derive(Default)]
pub(crate) struct PreviewState {
    /// Parsed Markdown per source document, valid for the recorded revision.
    parsed: HashMap<DocumentId, (u64, Arc<Vec<Block>>)>,
    /// SVG previews by preview-tab ID.
    svg: HashMap<DocumentId, SvgPreview>,
    pub(crate) images: HashMap<PathBuf, PreviewImage>,
    /// Image paths the last frame asked for, so loading starts on demand.
    wanted: Vec<PathBuf>,
    jobs: Vec<(PathBuf, Receiver<Result<DecodedImage, String>>)>,
    /// Heading anchor to scroll to in the given preview tab.
    pub(crate) anchor_request: Option<(DocumentId, String)>,
}

impl PreviewState {
    /// Blocks of `source`, reparsed only when its revision changed.
    pub(crate) fn blocks(&mut self, source: &Document) -> Arc<Vec<Block>> {
        if let Some((revision, blocks)) = self.parsed.get(&source.id)
            && *revision == source.revision
        {
            return Arc::clone(blocks);
        }
        let blocks = Arc::new(markdown::parse(&source.text));
        self.parsed
            .insert(source.id, (source.revision, Arc::clone(&blocks)));
        blocks
    }

    /// The SVG preview of `source` shown in tab `preview`, reparsed only when
    /// the source revision changed.
    pub(crate) fn svg(&mut self, preview: DocumentId, source: &Document) -> &mut SvgPreview {
        let entry = self.svg.entry(preview).or_insert_with(|| SvgPreview {
            revision: source.revision,
            parsed: SvgImage::parse(&source.text),
            raster: None,
            view: ImageViewState::default(),
        });
        if entry.revision != source.revision {
            entry.revision = source.revision;
            entry.parsed = SvgImage::parse(&source.text);
            entry.raster = None;
        }
        entry
    }

    /// Ask for a local image; returns its current state.
    pub(crate) fn image(&mut self, path: &Path) -> Option<&mut PreviewImage> {
        if !self.images.contains_key(path) && !self.wanted.iter().any(|wanted| wanted == path) {
            self.wanted.push(path.to_path_buf());
        }
        self.images.get_mut(path)
    }
}

impl SvgPreview {
    /// Intrinsic size when the SVG parsed, or why it did not.
    pub(crate) fn size(&self) -> Result<(u32, u32), &str> {
        self.parsed
            .as_ref()
            .map(SvgImage::size)
            .map_err(String::as_str)
    }

    /// The SVG rasterized sharply enough for `wanted` device pixels per SVG
    /// pixel, reusing the previous bitmap while it is still adequate.
    pub(crate) fn raster(&mut self, wanted: f32) -> Result<Arc<DecodedImage>, String> {
        let scale = raster_scale(wanted);
        if let Some((held, image)) = &self.raster
            && *held == scale
        {
            return Ok(Arc::clone(image));
        }
        let image = Arc::new(self.parsed.as_ref().map_err(Clone::clone)?.render(scale)?);
        self.raster = Some((scale, Arc::clone(&image)));
        Ok(image)
    }
}

/// Open the tab showing `source` as `kind`, or focus the one that is already
/// open. Returns whether `source` offers that view at all.
pub(crate) fn open_alternate_view(
    editor: &mut EditorState,
    view: &mut ViewState,
    source: DocumentId,
    kind: AlternateView,
) -> bool {
    let Some(index) = editor.index_of(source) else {
        return false;
    };
    if !editor.files[index].alternate_views().contains(&kind) {
        view.status = AlternateView::unsupported_message().into();
        return false;
    }
    let existing = editor
        .files
        .iter()
        .find(|file| file.view_of == Some(source) && file.view_kind == Some(kind))
        .map(|file| file.id);
    let id = match existing {
        Some(id) => id,
        None => {
            let tab = Document::view_for(&editor.files[index], kind);
            let id = tab.id;
            editor.files.insert(index + 1, tab);
            id
        }
    };
    view.leave_page_tabs();
    if let Some(diff) = &mut view.source_control.diff {
        diff.active = false;
    }
    editor.activate(id);
    view.focus_editor = true;
    view.status = kind.opened_message().into();
    true
}

/// Open the first alternate view of `source` that `wants` accepts.
pub(crate) fn open_first_view(
    editor: &mut EditorState,
    view: &mut ViewState,
    source: DocumentId,
    wants: impl Fn(&AlternateView) -> bool,
    missing: &str,
) {
    let kind = editor.index_of(source).and_then(|index| {
        editor.files[index]
            .alternate_views()
            .into_iter()
            .find(wants)
    });
    match kind {
        Some(kind) => {
            open_alternate_view(editor, view, source, kind);
        }
        None => view.status = missing.into(),
    }
}

/// Where a relative link or image of the document at `source` points.
pub(crate) fn resolve_relative(source: &Path, relative: &str) -> Option<PathBuf> {
    if relative.is_empty() {
        return None;
    }
    let relative = Path::new(relative);
    if relative.is_absolute() {
        return Some(relative.to_path_buf());
    }
    let mut resolved = source.parent()?.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    Some(resolved)
}

impl LumenApp {
    /// Keep preview titles and lifetimes in step with their sources.
    pub(crate) fn maintain_previews(&mut self, ctx: &egui::Context) {
        let titles: HashMap<DocumentId, String> = self
            .editor
            .files
            .iter()
            .map(|file| (file.id, file.name.clone()))
            .collect();
        let orphans: Vec<DocumentId> = self
            .editor
            .files
            .iter()
            .filter(|file| {
                file.view_of
                    .is_some_and(|source| !titles.contains_key(&source))
            })
            .map(|file| file.id)
            .collect();
        for file in &mut self.editor.files {
            if let Some(name) = file.view_of.and_then(|source| titles.get(&source)) {
                let title = file
                    .view_kind
                    .map_or_else(|| name.clone(), |kind| kind.tab_title(name));
                if file.name != title {
                    file.name = title;
                }
            }
        }
        for id in orphans {
            self.editor.close_document(id);
        }
        self.view
            .preview
            .parsed
            .retain(|id, _| titles.contains_key(id));
        self.view
            .preview
            .svg
            .retain(|id, _| titles.contains_key(id));
        self.view
            .visual
            .retain_sources(|id| titles.contains_key(id));
        self.poll_preview_images(ctx);
    }

    fn poll_preview_images(&mut self, ctx: &egui::Context) {
        let state = &mut self.view.preview;
        for path in std::mem::take(&mut state.wanted) {
            if state.images.contains_key(&path) {
                continue;
            }
            state.images.insert(path.clone(), PreviewImage::Loading);
            let receiver = image_files::spawn_load(path.clone());
            state.jobs.push((path, receiver));
        }
        let mut finished = Vec::new();
        state
            .jobs
            .retain(|(path, receiver)| match receiver.try_recv() {
                Ok(result) => {
                    finished.push((path.clone(), result));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    finished.push((path.clone(), Err("Image loading stopped".into())));
                    false
                }
            });
        for (path, result) in finished {
            state.images.insert(
                path,
                match result {
                    Ok(image) => PreviewImage::Ready {
                        image: Arc::new(image),
                        texture: None,
                    },
                    Err(_) => PreviewImage::Failed,
                },
            );
        }
        if !state.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Follow a link clicked in the preview tab `preview`.
    pub(crate) fn open_preview_link(&mut self, preview: DocumentId, destination: &str) {
        let source = self
            .editor
            .files
            .iter()
            .find(|file| file.id == preview)
            .and_then(|file| file.view_of)
            .and_then(|id| self.editor.index_of(id));
        match LinkTarget::classify(destination) {
            LinkTarget::Anchor(anchor) => {
                self.view.preview.anchor_request = Some((preview, anchor));
            }
            LinkTarget::External(url) => {
                self.view.status = match crate::services::open_external(&url) {
                    Ok(()) => format!("Opened {url}"),
                    Err(error) => format!("Could not open {url}: {error}"),
                };
            }
            LinkTarget::File(relative) => {
                let target = source
                    .and_then(|index| self.editor.files[index].path.as_deref())
                    .and_then(|path| resolve_relative(path, &relative));
                match target {
                    Some(path) if path.is_file() => self.open_file(path),
                    Some(path) => {
                        self.view.status = format!("{} does not exist", path.display());
                    }
                    None => self.view.status = "This link has no file target".into(),
                }
            }
        }
    }
}
