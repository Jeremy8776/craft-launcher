//! The part of a tool's card that is about its model: which connected model does the job, and a
//! small sample run that proves it does.
//!
//! Choosing never writes anything the model cannot do: the port refuses a model that is not in
//! the tool's list, and the list only holds models that take the tool's inputs. A sample to a
//! hosted service waits for a yes first, and says plainly that it is billed.

use craft_ai_features::Feature;
use craft_ai_model::{Billing, ConnectionId, ModelId, PRESETS};
use egui::{RichText, Ui};

use super::{FeaturesPort, FeaturesUi, Outcome, Sample, ToolState, ToolStatus};
use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::widgets::{self, Kind};

/// What a press in the block asks for, applied after drawing so no borrow is held while it runs.
pub(super) enum Act {
    Choose(Option<(ConnectionId, ModelId)>),
    Test { agreed: bool },
    CancelTest,
    Dismiss,
}

impl FeaturesUi {
    /// A workflow's runtime setting, kept in Advanced rather than beside the main model choice.
    /// Returns true when this tool has a local model whose runtime can be shown.
    pub(super) fn engine_setting(&mut self, ui: &mut Ui, feature: &Feature, tool: &ToolState, port: &mut dyn FeaturesPort) -> bool {
        let Some((_, model)) = &tool.bound else { return false };
        if !tool.choices.iter().any(|c| &c.model == model && c.local) {
            return false;
        }
        ui.horizontal(|ui| {
            if let Some(Act::Choose(choice)) = engine_picker(ui, feature, tool) {
                self.choose(&feature.id, choice, port);
            }
        });
        true
    }
}

/// A workflow's runtime is selected independently of its model. Switching is only enabled
/// when the same model is available on that runtime; the saved binding changes only after the
/// host accepts the choice, so an incompatible option never displaces the working route.
fn engine_picker(ui: &mut Ui, feature: &Feature, tool: &ToolState) -> Option<Act> {
    let (_, model) = tool.bound.as_ref()?;
    let current = tool.choices.iter().find(|c| &c.model == model && c.local)?;

    let mut providers: Vec<String> = Vec::new();
    for choice in tool.choices.iter().filter(|c| c.local) {
        if !providers.iter().any(|provider| provider == &choice.provider) {
            providers.push(choice.provider.clone());
        }
    }
    if let Some(recipe) = feature.locals.iter().find(|r| r.workflow == model.as_str() || r.native.iter().any(|n| n.model == model.as_str())) {
        if recipe.usable && !providers.iter().any(|p| p == "comfyui") {
            providers.push("comfyui".to_owned());
        }
        for route in &recipe.native {
            if !providers.iter().any(|p| p == &route.provider) {
                providers.push(route.provider.clone());
            }
        }
    }
    let has_unavailable_option =
        providers.iter().any(|provider| !tool.choices.iter().any(|c| c.local && c.provider == *provider && c.model == *model));
    let selected = provider_name(&current.provider);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(tl!("Run with"));
        egui::ComboBox::from_id_salt(("tool-runtime", &feature.id)).selected_text(&selected).show_ui(ui, |ui| {
            for provider in &providers {
                let target = tool.choices.iter().find(|c| c.local && c.provider == *provider && c.model == *model);
                let name = provider_name(provider);
                if let Some(target) = target {
                    let is_current = provider == &current.provider;
                    if ui.selectable_label(is_current, &name).clicked() && !is_current {
                        action = Some(Act::Choose(Some((target.connection.clone(), target.model.clone()))));
                    }
                } else {
                    ui.add_enabled_ui(false, |ui| {
                        ui.button(&name).on_hover_text(tl!("Not available here"));
                    });
                }
            }
        });
    });
    if has_unavailable_option {
        ui.label(tl!("This model is not available with every option."));
    }
    action
}

pub(super) fn provider_name(provider: &str) -> String {
    match provider {
        "comfyui" => "ComfyUI".to_owned(),
        "candle" | "burn" => tl!("Inside this app").to_owned(),
        _ => tl!("This computer").to_owned(),
    }
}

pub(super) fn status_line(ui: &mut Ui, tool: &ToolState) {
    let t = Tokens::get(ui.ctx());
    let (text, colour) = match &tool.status {
        ToolStatus::NoChoice => (tl!("No model chosen. Apps will ask, or fall back to their own setting.").to_owned(), t.muted),
        ToolStatus::Ready { local: true, .. } => (tl!("Ready. Runs on this computer.").to_owned(), t.good),
        ToolStatus::Ready { local: false, .. } => (tl!("Ready. Sends to a hosted service.").to_owned(), t.good),
        ToolStatus::Unavailable(reason) => (reason.clone(), t.bad),
    };
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(colour)).wrap());
}

/// The test button and everything that follows from pressing it.
pub(super) fn sample_row(ui: &mut Ui, tool: &ToolState, sample: Option<&Sample>, act: &mut Option<Act>) -> Option<Outcome> {
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    match sample {
        None | Some(Sample::Idle) => {
            let ready = matches!(tool.status, ToolStatus::Ready { .. }) && tool.can_test;
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(ready, |ui| {
                    if widgets::button(ui, tl!("Test it"), Kind::Secondary, 0.0)
                        .on_hover_text(tl!("Runs one small sample to see whether this model really does the job"))
                        .clicked()
                    {
                        *act = Some(Act::Test { agreed: false });
                    }
                });
                if !tool.can_test {
                    ui.label(RichText::new(tl!("No test for this tool yet")).size(12.0).color(t.faint));
                } else if tool.costly && ready {
                    ui.label(RichText::new(tl!("A hosted video sample can cost noticeably more than an image")).size(12.0).color(t.warn));
                }
            });
        }
        Some(Sample::NeedsConsent { connection, destination, items, billing, .. }) => {
            ui.add(
                egui::Label::new(
                    RichText::new(fmt(
                        tl!("This sends a small sample ({items}) to {destination} through {connection}. {cost}"),
                        &[("items", &items.join(", ")), ("destination", destination), ("connection", connection), ("cost", &billing_text(*billing))],
                    ))
                    .color(t.ink),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, tl!("Send the test"), Kind::Primary, 0.0).clicked() {
                    *act = Some(Act::Test { agreed: true });
                }
                if widgets::button(ui, tl!("Cancel"), Kind::Secondary, 0.0).clicked() {
                    *act = Some(Act::Dismiss);
                }
            });
        }
        Some(Sample::Running { .. }) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tl!("Testing…")).color(t.muted));
                if widgets::button(ui, tl!("Cancel"), Kind::Secondary, 0.0).clicked() {
                    *act = Some(Act::CancelTest);
                }
            });
        }
        Some(Sample::Passed { summary, millis, .. }) => {
            ui.add(
                egui::Label::new(
                    RichText::new(fmt(tl!("Works. {summary}, in {secs} s."), &[("summary", summary), ("secs", &seconds(*millis))])).color(t.good),
                )
                .wrap(),
            );
            if widgets::button(ui, tl!("Done"), Kind::Secondary, 0.0).clicked() {
                *act = Some(Act::Dismiss);
            }
        }
        Some(Sample::Failed { message, .. }) => {
            ui.add(egui::Label::new(RichText::new(fmt(tl!("The test failed: {e}"), &[("e", message)])).color(t.bad)).wrap());
            ui.horizontal(|ui| {
                if widgets::button(ui, tl!("Try again"), Kind::Secondary, 0.0).clicked() {
                    *act = Some(Act::Test { agreed: false });
                }
                if widgets::button(ui, tl!("Done"), Kind::Secondary, 0.0).clicked() {
                    *act = Some(Act::Dismiss);
                }
            });
        }
    }
    None
}

pub(super) fn billing_text(billing: Billing) -> String {
    match billing {
        Billing::Free => tl!("It is free.").to_owned(),
        Billing::Metered => tl!("You are billed for it by the service.").to_owned(),
        Billing::SubscriptionAllowance => tl!("It counts against your subscription's allowance.").to_owned(),
        Billing::Unknown => tl!("Whether it costs anything is not known.").to_owned(),
    }
}

/// Tenths of a second, rounded, without floating point.
pub(super) fn seconds(millis: u64) -> String {
    let tenths = millis.saturating_add(50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// One button per hosted option; pressing it opens the connection form for that service.
pub(super) fn hosted_buttons(ui: &mut Ui, feature: &Feature) -> Option<Outcome> {
    let mut outcome = None;
    ui.horizontal_wrapped(|ui| {
        for option in &feature.cloud {
            let Some(preset) = PRESETS.iter().find(|p| p.key == option.preset) else { continue };
            if widgets::button(ui, crate::i18n::t(&option.label), Kind::Secondary, 0.0).on_hover_text(crate::i18n::t(&option.note)).clicked() {
                outcome = Some(Outcome::AddConnection(preset));
            }
        }
    });
    outcome
}

/// A hairline across the available width, between sections.
pub(super) fn hline(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, t.hairline().color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_tenths_of_a_second() {
        assert_eq!(seconds(0), "0.0");
        assert_eq!(seconds(1_249), "1.2");
        assert_eq!(seconds(1_250), "1.3");
        assert_eq!(seconds(60_000), "60.0");
        assert_eq!(seconds(u64::MAX), "18446744073709551.6");
    }
}
