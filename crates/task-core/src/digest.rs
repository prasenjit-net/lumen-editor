//! A stable fingerprint of what a definition would execute. Review/trust decisions
//! are remembered against it, so editing a label or presentation keeps trust but
//! changing a command, argument, directory, environment or dependency does not.

use crate::{Platform, TaskDef};

/// FNV-1a over a canonical text form. Not cryptographic: it detects edits, it does
/// not resist an attacker crafting collisions, and it is stable across releases
/// (unlike `DefaultHasher`).
pub fn execution_digest(task: &TaskDef) -> String {
    let mut text = String::new();
    let mut field = |name: &str, value: String| {
        text.push_str(name);
        text.push('=');
        text.push_str(&value.len().to_string());
        text.push(':');
        text.push_str(&value);
        text.push('\n');
    };
    field("kind", task.kind.as_str().into());
    field("command", task.command.clone().unwrap_or_default());
    field("args", task.args.join("\u{1f}"));
    field("cwd", task.cwd.clone().unwrap_or_default());
    field("env", pairs(task.env.iter()));
    field("unset", task.unset_env.join("\u{1f}"));
    field(
        "transport",
        task.transport
            .map(|t| t.as_str().to_string())
            .unwrap_or_default(),
    );
    field("depends", task.depends_on.join("\u{1f}"));
    field(
        "order",
        task.depends_order
            .map(|o| o.as_str().to_string())
            .unwrap_or_default(),
    );
    field(
        "background",
        match &task.background {
            Some(b) => b.ready_pattern.clone(),
            None => String::new(),
        },
    );
    for platform in Platform::ALL {
        let over = task.platform_override(platform);
        field(
            platform.key(),
            format!(
                "{}|{}|{}|{}",
                over.command.clone().unwrap_or_default(),
                over.args
                    .as_ref()
                    .map(|a| a.join("\u{1f}"))
                    .unwrap_or_default(),
                over.cwd.clone().unwrap_or_default(),
                pairs(over.env.iter())
            ),
        );
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn pairs<'a>(items: impl Iterator<Item = (&'a String, &'a String)>) -> String {
    items
        .map(|(name, value)| format!("{name}\u{1e}{value}"))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Presentation, TaskKind};

    fn task() -> TaskDef {
        let mut task = TaskDef::new("build", "Build", TaskKind::Process);
        task.command = Some("cargo".into());
        task.args = vec!["build".into()];
        task
    }

    #[test]
    fn presentation_and_labels_do_not_change_the_digest_but_execution_does() {
        let base = execution_digest(&task());
        assert_eq!(base.len(), 16);
        let mut other = task();
        other.label = "Other".into();
        other.detail = Some("x".into());
        other.group = Some("build".into());
        other.is_default = true;
        other.presentation = Presentation {
            focus: Some(true),
            ..Default::default()
        };
        assert_eq!(execution_digest(&other), base);
        let changed = |edit: &dyn Fn(&mut TaskDef)| {
            let mut t = task();
            edit(&mut t);
            execution_digest(&t) != base
        };
        assert!(changed(&|t| t.command = Some("make".into())));
        assert!(changed(&|t| t.args.push("--release".into())));
        assert!(changed(&|t| t.cwd = Some("sub".into())));
        assert!(changed(&|t| {
            t.env.insert("A".into(), "1".into());
        }));
        assert!(changed(&|t| t.depends_on = vec!["x".into()]));
        assert!(changed(&|t| t.windows.command = Some("nmake".into())));
        assert!(changed(&|t| t.kind = TaskKind::Shell));
    }

    #[test]
    fn field_boundaries_cannot_be_forged_by_moving_text() {
        let mut a = task();
        a.args = vec!["a b".into()];
        let mut b = task();
        b.args = vec!["a".into(), "b".into()];
        assert_ne!(execution_digest(&a), execution_digest(&b));
        let mut c = task();
        c.command = Some("cargobuild".into());
        c.args.clear();
        assert_ne!(execution_digest(&c), execution_digest(&task()));
    }
}
