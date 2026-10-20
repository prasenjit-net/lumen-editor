//! `⋯` menu contents for Source Control. Pure data: each entry names a typed
//! `Command` and the reason it is unavailable, so the rules are testable
//! without drawing anything.
use super::widgets::{MenuEntry, MenuItem};
use crate::{
    app::command::Command,
    services::source_control::{GitSnapshot, OperationAction, RepoState},
};

const BUSY: &str = "Another Git operation is running";

/// Actions for the selected repository. The first line names the target so it
/// stays unambiguous in a workspace with several repositories.
pub(crate) fn repository_menu(
    name: &str,
    snapshot: &GitSnapshot,
    idle: bool,
    has_conflicts: bool,
) -> Vec<MenuEntry> {
    let busy = (!idle).then_some(BUSY);
    let no_remote = snapshot
        .remotes
        .is_empty()
        .then_some("No remote is configured");
    let mut entries = vec![
        MenuEntry::Caption(format!("{name}  ·  {}", snapshot.branch)),
        MenuItem::new("Pull", Command::PullSourceControl)
            .unless(busy)
            .entry(),
        MenuItem::new("Push", Command::PushSourceControl)
            .unless(busy)
            .entry(),
        MenuItem::new("Fetch", Command::FetchSourceControl)
            .unless(busy.or(no_remote))
            .entry(),
    ];
    match crate::app::source_control::publish_target(snapshot) {
        Ok((remote, branch)) => entries.push(
            MenuItem::new(
                format!("Publish Branch  ({branch} to {remote})"),
                Command::PublishSourceBranch,
            )
            .unless(busy)
            .entry(),
        ),
        Err(reason) if snapshot.upstream.is_none() => entries.push(
            MenuItem::new("Publish Branch", Command::PublishSourceBranch)
                .unless(Some(reason))
                .entry(),
        ),
        Err(_) => {}
    }
    entries.push(MenuEntry::Separator);
    entries.extend([
        MenuItem::new("Switch or Create Branch…", Command::OpenBranchDialog)
            .unless(busy)
            .entry(),
        MenuItem::new("Stash…", Command::OpenStashDialog)
            .unless(busy)
            .entry(),
        MenuItem::new("Tags…", Command::OpenTagsDialog(None))
            .unless(busy)
            .entry(),
        MenuItem::new("Show History…", Command::OpenHistoryDialog).entry(),
    ]);
    if snapshot.state != RepoState::Clean {
        entries.push(MenuEntry::Separator);
        entries.push(
            MenuItem::new(
                "Continue",
                Command::SourceOperation(OperationAction::Continue),
            )
            .unless(busy.or(has_conflicts.then_some("Resolve and stage every conflict first")))
            .entry(),
        );
        if snapshot.state != RepoState::Merge {
            entries.push(
                MenuItem::new("Skip", Command::SourceOperation(OperationAction::Skip))
                    .unless(busy)
                    .entry(),
            );
        }
        entries.push(
            MenuItem::new("Abort…", Command::SourceOperation(OperationAction::Abort))
                .unless(busy)
                .danger()
                .entry(),
        );
    }
    if snapshot.upstream.is_some() {
        entries.push(MenuEntry::Separator);
        entries.push(
            MenuItem::new("Force Push with Lease…", Command::PlanForcePush)
                .unless(busy)
                .danger()
                .entry(),
        );
    }
    entries
}

/// Actions that concern the whole panel rather than one repository.
pub(crate) fn panel_menu(has_workspace: bool, idle: bool) -> Vec<MenuEntry> {
    let no_folder = (!has_workspace).then_some("Open a folder first");
    vec![
        MenuEntry::Caption("All repositories".into()),
        MenuItem::new("Refresh and Rescan", Command::RefreshSourceControl)
            .unless(no_folder)
            .entry(),
        MenuEntry::Separator,
        MenuItem::new("Clone Repository…", Command::OpenCloneDialog)
            .unless((!idle).then_some(BUSY))
            .entry(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::source_control::{GitHead, GitRemote, GitUpstream};

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item.label.clone()),
                _ => None,
            })
            .collect()
    }

    fn item<'a>(entries: &'a [MenuEntry], label: &str) -> &'a MenuItem {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label.starts_with(label) => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no menu item {label}"))
    }

    fn remote() -> GitRemote {
        GitRemote {
            name: "origin".into(),
            fetch_url: "https://example.com/a.git".into(),
            push_url: None,
        }
    }

    #[test]
    fn the_menu_names_its_repository_first() {
        let snapshot = GitSnapshot {
            branch: "main".into(),
            ..Default::default()
        };
        let entries = repository_menu("lumen", &snapshot, true, false);
        assert_eq!(
            entries[0],
            MenuEntry::Caption("lumen  ·  main".into()),
            "the target repository must lead the menu"
        );
    }

    #[test]
    fn unavailable_items_explain_themselves() {
        let snapshot = GitSnapshot {
            branch: "main".into(),
            head: GitHead::Attached("main".into()),
            ..Default::default()
        };
        let entries = repository_menu("lumen", &snapshot, true, false);
        assert_eq!(
            item(&entries, "Fetch").disabled.as_deref(),
            Some("No remote is configured")
        );
        assert!(item(&entries, "Publish Branch").disabled.is_some());
        assert!(!labels(&entries).iter().any(|l| l.starts_with("Force")));

        let busy = repository_menu("lumen", &snapshot, false, false);
        assert_eq!(
            item(&busy, "Pull").disabled.as_deref(),
            Some("Another Git operation is running")
        );
    }

    #[test]
    fn publish_and_force_push_follow_the_upstream() {
        let mut snapshot = GitSnapshot {
            head: GitHead::Attached("topic".into()),
            remotes: vec![remote()],
            ..Default::default()
        };
        let entries = repository_menu("r", &snapshot, true, false);
        assert!(item(&entries, "Publish Branch").disabled.is_none());
        assert!(!labels(&entries).iter().any(|l| l.starts_with("Force")));

        snapshot.upstream = Some(GitUpstream {
            name: "origin/topic".into(),
            ahead_behind: Some((0, 0)),
        });
        let entries = repository_menu("r", &snapshot, true, false);
        assert!(!labels(&entries).iter().any(|l| l.starts_with("Publish")));
        let force = item(&entries, "Force Push");
        assert!(force.danger && force.disabled.is_none());
        assert!(
            matches!(entries.last(), Some(MenuEntry::Item(last)) if last.danger),
            "dangerous entries come last"
        );
    }

    #[test]
    fn operation_controls_appear_only_while_an_operation_runs() {
        let mut snapshot = GitSnapshot::default();
        let idle = repository_menu("r", &snapshot, true, false);
        assert!(!labels(&idle).iter().any(|l| l == "Continue"));

        snapshot.state = RepoState::Rebase;
        let entries = repository_menu("r", &snapshot, true, true);
        assert!(item(&entries, "Continue").disabled.is_some());
        assert!(labels(&entries).iter().any(|l| l == "Skip"));
        assert!(item(&entries, "Abort").danger);

        snapshot.state = RepoState::Merge;
        let entries = repository_menu("r", &snapshot, true, false);
        assert!(item(&entries, "Continue").disabled.is_none());
        assert!(!labels(&entries).iter().any(|l| l == "Skip"));
    }

    #[test]
    fn the_panel_menu_is_scoped_to_every_repository() {
        let entries = panel_menu(true, true);
        assert_eq!(entries[0], MenuEntry::Caption("All repositories".into()));
        assert!(item(&entries, "Refresh").disabled.is_none());
        assert!(item(&panel_menu(false, true), "Refresh").disabled.is_some());
    }
}
