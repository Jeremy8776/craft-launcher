//! The terms of everything a model's workflow runs, as each publisher states them.
//!
//! This is disclosure only. It repeats what the publishers say, links to where they say it, and
//! says plainly that ArtCraft has not checked it, that it is not legal advice, and that no
//! warranty or indemnity comes with it. It never blocks a download or a run: whether a model
//! suits a person's use, including commercial use, is their decision.
use craft_ai_features::LocalRecipe;
use egui::{RichText, Ui};

use crate::theme::Tokens;
use crate::widgets;

/// Lists the recipe's components. Draws nothing when the recipe has none.
pub(super) fn show(ui: &mut Ui, recipe: &LocalRecipe) {
    if recipe.components.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            RichText::new(tl!(
                "This is what each publisher says about its terms. ArtCraft has not checked it, it is not legal advice, and ArtCraft gives no warranty or indemnity for any model, add-on or result. Whether it suits your use, including commercial use, is your decision."
            ))
            .size(12.0)
            .color(t.muted),
        )
        .wrap(),
    );
    for component in &recipe.components {
        ui.add_space(6.0);
        ui.label(RichText::new(crate::i18n::t(&component.label)).color(t.ink_strong));
        ui.add(egui::Label::new(RichText::new(crate::i18n::t(&component.role)).size(12.0).color(t.muted)).wrap());
        ui.add(egui::Label::new(RichText::new(crate::i18n::t(&component.licence.name)).size(12.0).color(t.ink)).wrap());
        if let Some(note) = &component.licence.note {
            ui.add(egui::Label::new(RichText::new(crate::i18n::t(note)).size(12.0).color(t.muted)).wrap());
        }
        if let Some(url) = &component.licence.url {
            widgets::hud_link(ui, tl!("Where the publisher says so"), url);
        }
    }
}

/// The same list folded under a heading, for a model row.
pub(super) fn folded(ui: &mut Ui, recipe: &LocalRecipe) {
    if recipe.components.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(RichText::new(tl!("Licences of what this uses")).size(12.0))
        .id_salt(("licences", &recipe.workflow))
        .default_open(false)
        .show(ui, |ui| show(ui, recipe));
}
