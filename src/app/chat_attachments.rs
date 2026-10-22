//! Files attached to a chat message: classification, limits and conversion to
//! provider-neutral input blocks. Attachments are context only; nothing here
//! writes to the attached file.
use super::LumenApp;
use crate::services::FileSystem;
use lumen_agent_core::{SessionId, Support, api::InputBlock};
use std::path::{Path, PathBuf};

/// Attachments allowed on one message.
pub(crate) const MAX_ATTACHMENTS: usize = 10;
/// Largest image sent inline.
pub(crate) const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Largest total of inline image bytes in one message.
pub(crate) const MAX_TOTAL_IMAGE_BYTES: u64 = 15 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub(crate) path: PathBuf,
    /// Set for images, from the file's magic bytes rather than its extension.
    pub(crate) image_type: Option<&'static str>,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttachmentError {
    Duplicate(String),
    TooMany,
    Folder(String),
    Unreadable(String, String),
    ImageTooLarge(String),
    TotalTooLarge,
    Unsupported(String),
}

impl std::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(name) => write!(f, "{name} is already attached"),
            Self::TooMany => write!(f, "At most {MAX_ATTACHMENTS} files can be attached"),
            Self::Folder(name) => write!(f, "{name} is a folder; attach its files instead"),
            Self::Unreadable(name, why) => write!(f, "Could not read {name}: {why}"),
            Self::ImageTooLarge(name) => write!(
                f,
                "{name} is larger than {} MB, the image limit",
                MAX_IMAGE_BYTES / (1024 * 1024)
            ),
            Self::TotalTooLarge => write!(
                f,
                "Images may total at most {} MB per message",
                MAX_TOTAL_IMAGE_BYTES / (1024 * 1024)
            ),
            Self::Unsupported(reason) => f.write_str(reason),
        }
    }
}

impl Attachment {
    pub(crate) fn new(path: PathBuf, image_type: Option<&'static str>, size: u64) -> Self {
        Self {
            path,
            image_type,
            size,
        }
    }

    pub(crate) fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into(),
        )
    }
}

/// The image media type for a file's leading bytes, if it is a supported image.
pub(crate) fn sniff_image(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn has_image_extension(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        ["png", "jpg", "jpeg", "gif", "webp"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

/// Inspect a picked path. Folders and unreadable files are refused here, with a
/// message, so a failed pick never leaves a half-made attachment.
pub(crate) fn classify(fs: &dyn FileSystem, path: &Path) -> Result<Attachment, AttachmentError> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    if fs.is_directory(path) {
        return Err(AttachmentError::Folder(name));
    }
    let size = fs
        .file_len(path)
        .map_err(|e| AttachmentError::Unreadable(name.clone(), e.to_string()))?;
    if size > MAX_IMAGE_BYTES {
        // Too big to sniff cheaply: only an image by name needs the limit.
        return if has_image_extension(path) {
            Err(AttachmentError::ImageTooLarge(name))
        } else {
            Ok(Attachment::new(path.into(), None, size))
        };
    }
    let bytes = fs
        .read_bytes(path)
        .map_err(|e| AttachmentError::Unreadable(name, e.to_string()))?;
    Ok(Attachment::new(path.into(), sniff_image(&bytes), size))
}

/// Refuse what the provider has said it cannot take, with its own reason.
pub(crate) fn check_support(
    attachment: &Attachment,
    files: &Support,
    images: &Support,
) -> Result<(), AttachmentError> {
    let (support, what) = if attachment.image_type.is_some() {
        (images, "images")
    } else {
        (files, "files")
    };
    match support {
        Support::Supported => Ok(()),
        Support::Unsupported { reason } | Support::VersionGated { reason, .. } => Err(
            AttachmentError::Unsupported(format!("This agent cannot take {what}: {reason}")),
        ),
    }
}

/// Input blocks for the attachments: files by reference, images inline. Reads
/// are fresh, so a file moved or changed since it was attached is reported.
pub(crate) fn blocks(
    fs: &dyn FileSystem,
    attachments: &[Attachment],
) -> Result<Vec<InputBlock>, AttachmentError> {
    let mut out = Vec::with_capacity(attachments.len());
    let mut image_total = 0u64;
    for attachment in attachments {
        let name = attachment.name();
        let Some(media_type) = attachment.image_type else {
            if !fs.entry_exists(&attachment.path) {
                return Err(AttachmentError::Unreadable(name, "the file is gone".into()));
            }
            out.push(InputBlock::FileReference {
                path: attachment.path.clone(),
            });
            continue;
        };
        let bytes = fs
            .read_bytes(&attachment.path)
            .map_err(|e| AttachmentError::Unreadable(name.clone(), e.to_string()))?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return Err(AttachmentError::ImageTooLarge(name));
        }
        image_total += bytes.len() as u64;
        if image_total > MAX_TOTAL_IMAGE_BYTES {
            return Err(AttachmentError::TotalTooLarge);
        }
        out.push(InputBlock::Image {
            media_type: media_type.into(),
            bytes,
        });
    }
    Ok(out)
}

impl LumenApp {
    /// Pick files with the native dialog and attach them to the session's draft.
    pub(super) fn chat_attach(&mut self, id: SessionId) {
        let picked = self.dialogs.pick_attachments();
        self.attach_paths(id, picked);
    }

    /// Attach each path; the first refusal is shown and the rest still apply.
    pub(super) fn attach_paths(&mut self, id: SessionId, paths: Vec<PathBuf>) {
        let (files, images) = self
            .view
            .native_agents
            .as_ref()
            .and_then(|hub| hub.live(id))
            .and_then(|live| live.capabilities.as_deref())
            .map_or((Support::Supported, Support::Supported), |caps| {
                (
                    caps.attachments_text.clone(),
                    caps.attachments_image.clone(),
                )
            });
        let mut refusal = None;
        for path in paths {
            let result = classify(&*self.file_system, &path)
                .and_then(|a| check_support(&a, &files, &images).map(|()| a))
                .and_then(|a| self.view.chats.attach(id, a));
            if let Err(error) = result {
                refusal.get_or_insert(error.to_string());
            }
        }
        if let Some(message) = refusal {
            self.view.status = message;
        }
    }

    pub(super) fn chat_detach(&mut self, id: SessionId, path: &Path) {
        self.view.chats.detach(id, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_recognised_by_magic_bytes_not_names() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<svg></svg>"), None);
        assert_eq!(sniff_image(b"fn main() {}"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn unsupported_kinds_are_refused_with_the_providers_reason() {
        let file = Attachment::new("/w/a.rs".into(), None, 1);
        let image = Attachment::new("/w/a.png".into(), Some("image/png"), 1);
        let no = Support::unsupported("not yet");
        assert!(check_support(&file, &Support::Supported, &no).is_ok());
        assert!(check_support(&image, &Support::Supported, &Support::Supported).is_ok());
        let error = check_support(&image, &Support::Supported, &no).unwrap_err();
        assert_eq!(error.to_string(), "This agent cannot take images: not yet");
        assert!(check_support(&file, &no, &Support::Supported).is_err());
    }
}
