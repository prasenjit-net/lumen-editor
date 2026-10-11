use super::DocumentId;
use std::path::{Path, PathBuf};
pub(crate) struct Document {
    pub(crate) id: DocumentId,
    pub(crate) revision: u64,
    pub(crate) path: Option<PathBuf>,
    pub(crate) name: String,
    pub(crate) language: String,
    pub(crate) text: String,
    pub(crate) dirty: bool,
    saved_text: String,
    #[cfg(feature = "shared-editor")]
    pub(crate) editing: Option<lumen_editor_core::EditorBuffer>,
}

impl Document {
    pub(crate) fn from_path(path: PathBuf, text: String) -> Self {
        Self {
            id: DocumentId::allocate(),
            revision: 0,
            name: display_name(&path),
            language: language_for(&path),
            path: Some(path),
            saved_text: text.clone(),
            text,
            dirty: false,
            #[cfg(feature = "shared-editor")]
            editing: None,
        }
    }

    pub(crate) fn mark_saved(&mut self, path: PathBuf) {
        self.relocate(path);
        self.saved_text.clone_from(&self.text);
        self.dirty = false;
        #[cfg(feature = "shared-editor")]
        if let Some(buffer) = &mut self.editing {
            buffer.mark_saved();
        }
    }

    /// TextEdit remains the single editing/history authority during migration.
    /// Observe its completed edit, including undo/redo, against saved contents.
    pub(crate) fn record_text_change(&mut self) {
        #[cfg(feature = "shared-editor")]
        debug_assert!(
            self.editing.is_none(),
            "Shared documents must synchronize their core, not edit the snapshot"
        );
        self.revision = self.revision.wrapping_add(1);
        self.dirty = self.text != self.saved_text;
    }

    pub(crate) fn relocate(&mut self, path: PathBuf) {
        self.name = display_name(&path);
        self.language = language_for(&path);
        self.path = Some(path);
    }
    pub(crate) fn untitled(number: usize) -> Self {
        Self {
            id: DocumentId::allocate(),
            revision: 0,
            path: None,
            name: format!("untitled-{number}.txt"),
            language: "Plain Text".into(),
            text: String::new(),
            dirty: false,
            saved_text: String::new(),
            #[cfg(feature = "shared-editor")]
            editing: None,
        }
    }

    #[cfg(feature = "shared-editor")]
    pub(crate) fn enable_shared_editing(&mut self) {
        if self.editing.is_none() {
            let mut buffer = lumen_editor_core::EditorBuffer::new(self.saved_text.clone());
            if self.text != self.saved_text {
                buffer.replace_document(&self.text).unwrap();
            }
            self.editing = Some(buffer);
        }
    }

    #[cfg(feature = "shared-editor")]
    pub(crate) fn sync_shared_editing(&mut self) {
        if let Some(buffer) = &self.editing {
            self.text = buffer.text().to_owned();
            self.revision = buffer.revision();
            self.dirty = buffer.is_dirty();
        }
    }
}

fn language_for(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "rs" => "Rust",
        "toml" => "TOML",
        "md" => "Markdown",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "js" | "ts" | "tsx" | "jsx" => "JavaScript",
        "html" => "HTML",
        "css" => "CSS",
        "py" => "Python",
        "go" => "Go",
        "c" | "h" | "cpp" | "hpp" => "C/C++",
        "sh" => "Shell Script",
        _ => "Plain Text",
    }
    .into()
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .into()
}
