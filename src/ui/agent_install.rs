//! What the agent panel shows when a provider's program is not installed: where
//! to get it, and a way to look again once it is.
use super::theme;
use crate::{app::command::Command, icon};
use eframe::egui::{self, RichText};
use lumen_agent_core::ProviderId;

/// The provider's official setup instructions.
pub(crate) fn install_url(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "https://docs.claude.com/en/docs/claude-code/setup",
        ProviderId::Codex => "https://github.com/openai/codex#installing-and-running-codex-cli",
    }
}

/// A command that installs it, shown as a convenience next to the link.
fn install_command(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Claude => "npm install -g @anthropic-ai/claude-code",
        ProviderId::Codex => "npm install -g @openai/codex",
    }
}

pub(super) fn not_installed(
    ui: &mut egui::Ui,
    provider: ProviderId,
    program: &str,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) {
    ui.add_space(super::widgets::empty_state_top(ui).min(48.0));
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icon::AGENT)
                .size(26.0)
                .color(super::chat::provider_color(colors, provider)),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{} is not installed", provider.label()))
                .size(13.0)
                .color(colors.text),
        );
        ui.label(
            RichText::new(format!("Lumen could not find “{program}” on your PATH."))
                .size(11.5)
                .color(colors.muted),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(install_command(provider))
                .monospace()
                .size(11.5)
                .color(colors.text),
        );
        ui.add_space(8.0);
        let open = egui::Button::new(
            RichText::new("Open install instructions")
                .size(12.0)
                .color(colors.on_accent),
        )
        .fill(colors.accent)
        .corner_radius(3);
        if ui.add(open).on_hover_text(install_url(provider)).clicked() {
            commands.push(Command::AgentOpenInstallPage(provider));
        }
        ui.add_space(4.0);
        if ui
            .button(RichText::new("I installed it, check again").size(12.0))
            .clicked()
        {
            commands.push(Command::AgentRecheckInstall(provider));
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("A custom location can be set in Settings (agent.native).")
                .size(10.5)
                .color(colors.muted),
        );
    });
}
