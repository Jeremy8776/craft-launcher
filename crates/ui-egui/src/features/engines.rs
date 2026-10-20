//! Engine choices for a tool's already selected local model.

use egui::{RichText, Ui};

use super::{FeaturesPort, FeaturesUi, Snapshot};
use crate::theme::Tokens;
use crate::widgets;

impl FeaturesUi {
    pub(super) fn engines_body(&mut self, ui: &mut Ui, snapshot: &Snapshot, port: &mut dyn FeaturesPort) {
        let t = Tokens::get(ui.ctx());
        crate::connections::title(ui, tl!("Engines"), tl!("Choose where each local model runs."));
        for feature in self.features.clone() {
            let Some(tool) = snapshot.tools.iter().find(|tool| tool.feature == feature.id) else { continue };
            egui::Frame::new().stroke(t.hairline()).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(widgets::heading(crate::i18n::t(&feature.title), 15.0, t.ink_strong));
                let bound_local =
                    tool.bound.as_ref().is_some_and(|(_, model)| tool.choices.iter().any(|choice| &choice.model == model && choice.local));
                if bound_local && self.engine_setting(ui, &feature, tool, port) {
                    return;
                }
                let has_local = tool.choices.iter().any(|choice| choice.local);
                let message = if has_local {
                    tl!("Choose a local model in Tools to set its engine.")
                } else {
                    tl!("No local model is available for this tool yet.")
                };
                ui.label(RichText::new(message).size(12.0).color(t.muted));
            });
            ui.add_space(8.0);
        }
    }
}
