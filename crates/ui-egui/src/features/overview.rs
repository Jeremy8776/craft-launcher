//! Tool-first overview. Each card starts from a job, then offers only its usable setup paths.
use super::FolderNotice;
use craft_ai_features::{AppStatus, Feature, Route, State};
use craft_ai_model::model_folders::{FolderOrigin, ModelFolder};
use egui::{RichText, Ui};

use super::{FeaturesPort, FeaturesUi, LocalUse, Outcome, Snapshot};
use crate::i18n::fmt;
use crate::theme::Tokens;
use crate::widgets::{self, Kind};

pub(super) fn show(view: &mut FeaturesUi, ui: &mut Ui, snapshot: &Snapshot, port: &mut dyn FeaturesPort) -> Outcome {
    let t = Tokens::get(ui.ctx());
    crate::connections::title(ui, tl!("Tools"), tl!("See which Craft apps use each AI tool and set its model."));
    if let Some((message, error)) = &view.notice {
        ui.label(RichText::new(message).size(12.0).color(if *error { t.bad } else { t.good }));
        ui.add_space(6.0);
    }
    let mut outcome = Outcome::None;
    for feature in view.features.clone() {
        let tool = snapshot.tools.iter().find(|tool| tool.feature == feature.id);
        egui::Frame::new().stroke(t.hairline()).inner_margin(14).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(widgets::heading(crate::i18n::t(&feature.title), 16.0, t.ink_strong));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if view.configure.as_ref().is_some_and(|state| state.feature == feature.id) { tl!("Done") } else { tl!("Configure") };
                    if widgets::button(ui, label, Kind::Secondary, 0.0).clicked() {
                        view.toggle_configure(&feature.id, tool);
                    }
                });
            });
            ui.add_space(3.0);
            ui.add(egui::Label::new(RichText::new(crate::i18n::t(&feature.summary)).size(12.0).color(t.muted)).wrap());
            app_uses(ui, &feature);
            ui.add_space(6.0);
            if let Some(tool) = tool {
                super::tool::status_line(ui, tool);
            } else {
                ui.label(RichText::new(tl!("No model chosen. Apps will ask, or fall back to their own setting.")).size(12.0).color(t.muted));
            }
            if view.configure.as_ref().is_some_and(|state| state.feature == feature.id)
                && let Some(next) = view.configure_panel(ui, &feature, snapshot, port)
            {
                outcome = next;
            }
        });
        ui.add_space(8.0);
    }
    outcome
}

pub(super) fn folders(ui: &mut Ui, snapshot: &Snapshot, port: &mut dyn FeaturesPort) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(12.0);
    widgets::hud(ui, tl!("Folders"), t.faint);
    ui.add_space(6.0);
    egui::Frame::new().stroke(t.hairline()).inner_margin(14).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(
            egui::Label::new(
                RichText::new(tl!("Choose where new model files should go. Existing files stay where they are.")).size(12.0).color(t.muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        if snapshot.folders.busy {
            ui.label(RichText::new(tl!("Updating folders...")).size(12.0).color(t.muted));
        }
        if snapshot.folders.folders.is_empty() && !snapshot.folders.busy {
            let empty = if snapshot.folders.loaded {
                tl!("No folders yet. Add one to choose where model files are saved.")
            } else {
                tl!("Checking model folders...")
            };
            ui.label(RichText::new(empty).size(12.0).color(t.muted));
        } else {
            for (index, folder) in snapshot.folders.folders.iter().enumerate() {
                if index > 0 {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
                folder_row(ui, folder, port, snapshot.folders.busy);
            }
        }
        if let Some(notice) = snapshot.folders.notice {
            let (message, color) = match notice {
                FolderNotice::Added => (tl!("Folder added."), t.good),
                FolderNotice::DefaultChanged => (tl!("Default folder changed."), t.good),
                FolderNotice::Rescanned => (tl!("Folder checked again."), t.good),
                FolderNotice::Removed => (tl!("Folder removed from the list. Its files were not deleted."), t.good),
                FolderNotice::Failed => (tl!("That folder could not be changed. Check that it is available, then try again."), t.bad),
            };
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(message).size(12.0).color(color)).wrap());
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!snapshot.folders.busy, |ui| {
            if widgets::button(ui, tl!("Add a folder"), Kind::Secondary, 0.0).clicked() {
                port.add_folder();
            }
        });
    });
}

fn folder_row(ui: &mut Ui, folder: &ModelFolder, port: &mut dyn FeaturesPort, busy: bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(widgets::heading(&folder.label, 14.0, t.ink_strong));
                if folder.default {
                    widgets::badge(ui, tl!("Default for new downloads"), t.good);
                }
                if folder.origin == FolderOrigin::ComfyUi {
                    widgets::badge(ui, tl!("Read-only"), t.faint);
                }
            });
            ui.label(RichText::new(&folder.path).size(11.0).color(t.muted));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if folder.origin == FolderOrigin::Chosen && widgets::button(ui, tl!("Remove"), Kind::Secondary, 0.0).clicked() {
                    port.remove_folder(&folder.id);
                }
                if widgets::button(ui, tl!("Rescan"), Kind::Secondary, 0.0).clicked() {
                    port.rescan_folder(&folder.id);
                }
                if folder.origin == FolderOrigin::Chosen
                    && !folder.default
                    && widgets::button(ui, tl!("Make default"), Kind::Secondary, 0.0).clicked()
                {
                    port.set_default_folder(&folder.id);
                }
            });
        });
    });
}

fn app_uses(ui: &mut Ui, feature: &Feature) {
    let t = Tokens::get(ui.ctx());
    for usage in &feature.uses {
        let line = match usage.status {
            AppStatus::Proposed => tl!("Suggested for {app}: {tool}"),
            AppStatus::Planned => tl!("Planned for {app}: {tool}"),
            AppStatus::Shipped => tl!("Available in {app}: {tool}"),
        };
        ui.add(
            egui::Label::new(
                RichText::new(fmt(line, &[("app", crate::i18n::t(&usage.app)), ("tool", crate::i18n::t(&usage.tool))])).size(11.0).color(t.faint),
            )
            .wrap(),
        );
    }
}

pub(super) fn setup_options(view: &mut FeaturesUi, ui: &mut Ui, feature: &Feature, snapshot: &Snapshot, port: &mut dyn FeaturesPort) {
    let t = Tokens::get(ui.ctx());
    let options = view.options.clone();
    let local_usable = options.iter().any(|option| option.usable && matches!(&option.route, Route::Local { feature: id, .. } if id == &feature.id));
    for option in options.iter().filter(|option| match &option.route {
        Route::Local { feature: id, .. } | Route::Native { feature: id, .. } => id == &feature.id,
        Route::Service { .. } => false,
    }) {
        match &option.route {
            Route::Local { feature: id, recipe } => {
                let Some(plan) = view.plan_for_recipe(snapshot, id, recipe).cloned() else { continue };
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    widgets::hud(ui, crate::i18n::t(&option.title), t.faint);
                    match plan.state {
                        State::NeedsInstall if plan.download_bytes > 0 => {
                            let label = fmt(tl!("Set up, {size}"), &[("size", &super::text::bytes(plan.download_bytes))]);
                            if widgets::button(ui, &label, Kind::Secondary, 0.0).clicked() {
                                view.begin_setup(id, recipe);
                            }
                        }
                        State::Ready => {
                            widgets::badge(ui, tl!("Files in place"), t.good);
                            if option.usable {
                                match &snapshot.local_use {
                                    LocalUse::Checking { feature, recipe: active } if feature == id && active == recipe => {
                                        ui.spinner();
                                        ui.label(tl!("Checking model files..."));
                                    }
                                    LocalUse::Enabled { feature, recipe: active, bound } if feature == id && active == recipe => {
                                        let text = if *bound {
                                            tl!("Files checked. Model added for this tool.")
                                        } else {
                                            tl!("Files checked. Earlier model choice kept.")
                                        };
                                        ui.label(RichText::new(text).color(t.good));
                                    }
                                    _ => {
                                        if widgets::button(ui, tl!("Check and use model"), Kind::Secondary, 0.0).clicked()
                                            && let Err(message) = port.use_local_recipe(id, recipe)
                                        {
                                            view.notice = Some((message, true));
                                        }
                                    }
                                }
                            }
                        }
                        State::NoRuntime => {
                            ui.label(RichText::new(tl!("The local service for these files was not found.")).size(12.0).color(t.muted));
                        }
                        State::Blocked => {
                            ui.label(RichText::new(super::text::blocker_text(plan.blockers.first())).size(12.0).color(t.bad));
                        }
                        State::NeedsInstall | State::NoLocalRoute => {}
                    }
                    if !option.usable {
                        widgets::badge(ui, tl!("Setup only"), t.warn).on_hover_text(tl!("This model cannot be used by a Craft app yet."));
                    }
                });
                if let Some(recipe) = feature.locals.iter().find(|r| &r.workflow == recipe) {
                    super::licences::folded(ui, recipe);
                }
                if let LocalUse::Failed { feature, recipe: active, message } = &snapshot.local_use
                    && feature == id
                    && active == recipe
                {
                    ui.label(RichText::new(message).color(t.bad));
                }
            }
            Route::Native { .. } if !local_usable => {
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("ArtCraft Launcher cannot use this option yet.")).size(12.0).color(t.warn));
            }
            Route::Native { .. } => {}
            Route::Service { .. } => {}
        }
    }
    // One note per tool, not one warning per model.
    let any_setup_only =
        options.iter().any(|option| !option.usable && matches!(&option.route, Route::Local { feature: id, .. } if id == &feature.id));
    if any_setup_only {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(tl!("Models marked Setup only can be installed, but no Craft app can use them yet.")).size(12.0).color(t.muted),
            )
            .wrap(),
        );
    }
}
