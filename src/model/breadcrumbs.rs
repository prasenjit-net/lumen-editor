//! Path breadcrumbs for the editor header, without any UI types.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Crumb {
    pub(crate) label: String,
    /// Directory or file this crumb stands for; `None` for an unsaved document.
    pub(crate) path: Option<PathBuf>,
    pub(crate) is_file: bool,
}

/// Workspace folder, each directory down to the file, then the file itself.
/// A file outside the workspace (or with no workspace) gets only its own name,
/// because a relative path would be meaningless.
pub(crate) fn crumbs(workspace: Option<&Path>, file: Option<&Path>, name: &str) -> Vec<Crumb> {
    let Some(file) = file else {
        return vec![Crumb {
            label: name.to_owned(),
            path: None,
            is_file: true,
        }];
    };
    let relative = workspace.and_then(|root| file.strip_prefix(root).ok());
    let (Some(root), Some(relative)) = (workspace, relative) else {
        return vec![Crumb {
            label: name.to_owned(),
            path: Some(file.to_path_buf()),
            is_file: true,
        }];
    };
    let mut result = vec![Crumb {
        label: root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into(),
        ),
        path: Some(root.to_path_buf()),
        is_file: false,
    }];
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        result.push(Crumb {
            label: component.as_os_str().to_string_lossy().into_owned(),
            path: Some(current.clone()),
            is_file: index + 1 == components.len(),
        });
    }
    result
}

/// Index of the first crumb to draw. The last crumb always stays; earlier ones
/// are dropped from the front until the rest, plus a leading ellipsis when
/// anything was dropped, fit in `available`.
pub(crate) fn visible_start(
    widths: &[f32],
    separator: f32,
    ellipsis: f32,
    available: f32,
) -> usize {
    let Some(last) = widths.len().checked_sub(1) else {
        return 0;
    };
    let mut start = last;
    let mut used = widths[last];
    while start > 0 {
        let candidate = used + separator + widths[start - 1];
        let reserve = if start - 1 > 0 {
            separator + ellipsis
        } else {
            0.0
        };
        if candidate + reserve > available {
            break;
        }
        used = candidate;
        start -= 1;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(crumbs: &[Crumb]) -> Vec<&str> {
        crumbs.iter().map(|crumb| crumb.label.as_str()).collect()
    }

    #[test]
    fn files_inside_the_workspace_show_each_directory() {
        let result = crumbs(
            Some(Path::new("/w/proj")),
            Some(Path::new("/w/proj/src/ui/a.rs")),
            "a.rs",
        );
        assert_eq!(labels(&result), ["proj", "src", "ui", "a.rs"]);
        assert_eq!(result[1].path.as_deref(), Some(Path::new("/w/proj/src")));
        assert!(!result[2].is_file && result[3].is_file);
    }

    #[test]
    fn untitled_and_outside_files_get_only_their_name() {
        let untitled = crumbs(Some(Path::new("/w")), None, "untitled-1.txt");
        assert_eq!(labels(&untitled), ["untitled-1.txt"]);
        assert!(untitled[0].path.is_none());
        let outside = crumbs(Some(Path::new("/w")), Some(Path::new("/x/y.rs")), "y.rs");
        assert_eq!(labels(&outside), ["y.rs"]);
        let no_workspace = crumbs(None, Some(Path::new("/x/y.rs")), "y.rs");
        assert_eq!(labels(&no_workspace), ["y.rs"]);
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside_the_workspace() {
        let result = crumbs(
            Some(Path::new("/w/proj")),
            Some(Path::new("/w/proj-old/a.rs")),
            "a.rs",
        );
        assert_eq!(labels(&result), ["a.rs"]);
    }

    #[test]
    fn narrow_headers_drop_leading_crumbs_but_never_the_file() {
        let widths = [40.0, 30.0, 30.0, 50.0];
        assert_eq!(visible_start(&widths, 10.0, 12.0, 500.0), 0, "all fit");
        // Two crumbs (90) plus the ellipsis (22) need 112; a third would not fit.
        assert_eq!(visible_start(&widths, 10.0, 12.0, 120.0), 2);
        assert_eq!(
            visible_start(&widths, 10.0, 12.0, 100.0),
            3,
            "no room for the ellipsis"
        );
        assert_eq!(
            visible_start(&widths, 10.0, 12.0, 10.0),
            3,
            "file always stays"
        );
        assert_eq!(visible_start(&[], 10.0, 12.0, 100.0), 0);
        assert_eq!(visible_start(&[25.0], 10.0, 12.0, 5.0), 0);
    }
}
