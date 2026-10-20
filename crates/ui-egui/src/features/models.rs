//! The Models section: inventory from saved connections and shared model-file folders.

use egui::{RichText, Ui};

use super::{FeaturesPort, FeaturesUi, Snapshot};
use crate::connections::Row;
use crate::theme::Tokens;
use crate::widgets;

impl FeaturesUi {
    pub(super) fn models_body(&mut self, ui: &mut Ui, snapshot: &Snapshot, rows: &[Row], port: &mut dyn FeaturesPort) -> super::Outcome {
        let t = Tokens::get(ui.ctx());
        crate::connections::title(ui, tl!("Models"), tl!("Models available from your connections"));
        let model_count: usize = rows.iter().map(|row| row.draft.models.len()).sum();
        if model_count == 0 {
            egui::Frame::new().stroke(t.hairline()).inner_margin(14).show(ui, |ui| {
                ui.label(RichText::new(tl!("Models appear here after a connection has found them.")).color(t.muted));
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("Add a connection or refresh one to find its models.")).size(12.0).color(t.faint));
            });
        } else {
            for row in rows {
                for model in &row.draft.models {
                    egui::Frame::new().stroke(t.hairline()).inner_margin(12).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(widgets::heading(&model.label, 15.0, t.ink_strong));
                        ui.horizontal_wrapped(|ui| {
                            widgets::badge(ui, &row.label, if row.local { t.good } else { t.warn });
                            ui.label(
                                RichText::new(if row.local { tl!("Runs on this computer") } else { tl!("Leaves this computer") })
                                    .size(12.0)
                                    .color(t.muted),
                            );
                        });
                        ui.label(RichText::new(model.id.as_str()).size(11.0).color(t.faint));
                    });
                    ui.add_space(6.0);
                }
            }
        }
        ui.add_space(16.0);
        ui.label(widgets::heading(tl!("Models to set up"), 16.0, t.ink_strong));
        for feature in self.features.clone() {
            egui::Frame::new().stroke(t.hairline()).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(widgets::heading(crate::i18n::t(&feature.title), 14.0, t.ink_strong));
                super::overview::setup_options(self, ui, &feature, snapshot, port);
            });
        }
        super::overview::folders(ui, snapshot, port);
        super::Outcome::None
    }
}
