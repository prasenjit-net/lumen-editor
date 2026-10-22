//! Preview tabs: a read-only rendered view of another document, kept live from
//! the source buffer and tracked by the source's stable document ID.
use super::{LumenApp, view_state::ViewState};
use crate::{
    model::{
        Document, DocumentId, EditorState,
        image_asset::DecodedImage,
        markdown::{self, Block, LinkTarget},
        previewable::AlternateView,
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

/// Preview state that is not part of any document.
#[derive(Default)]
pub(crate) struct PreviewState {
    /// Parsed Markdown per source document, valid for the recorded revision.
    parsed: HashMap<DocumentId, (u64, Arc<Vec<Block>>)>,
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

    /// Ask for a local image; returns its current state.
    pub(crate) fn image(&mut self, path: &Path) -> Option<&mut PreviewImage> {
        if !self.images.contains_key(path) && !self.wanted.iter().any(|wanted| wanted == path) {
            self.wanted.push(path.to_path_buf());
        }
        self.images.get_mut(path)
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
