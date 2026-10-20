//! The inline Configure panel for a tool. It keeps model, scope and runtime choices together.

use std::collections::BTreeSet;

use egui::{RichText, Ui};

use super::tool::{Act, provider_name, sample_row};
use super::{FeaturesPort, FeaturesUi, Outcome, Sample, Snapshot, ToolState};
use crate::theme::Tokens;
use crate::widgets::{self, Kind};
use craft_ai_model::{ConnectionId, ModelId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DraftChoice {
    Automatic,
    Model(ConnectionId, ModelId),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ApplyTo {
    #[default]
    ThisTool,
    AllCompatible,
    SpecificTools,
}

#[derive(Clone, Debug)]
pub(crate) struct ConfigureState {
    pub(crate) feature: String,
    pub(crate) choice: DraftChoice,
    pub(crate) apply_to: ApplyTo,
    pub(crate) targets: BTreeSet<String>,
}

impl FeaturesUi {
    pub(crate) fn toggle_configure(&mut self, feature: &str, tool: Option<&ToolState>) {
        if self.configure.as_ref().is_some_and(|state| state.feature == feature) {
            self.configure = None;
            return;
        }
        let mut targets = BTreeSet::new();
        targets.insert(feature.to_owned());
        let choice = tool
            .and_then(|tool| tool.bound.as_ref())
            .map_or(DraftChoice::Automatic, |(connection, model)| DraftChoice::Model(connection.clone(), model.clone()));
        self.configure = Some(ConfigureState { feature: feature.to_owned(), choice, apply_to: ApplyTo::ThisTool, targets });
    }

    pub(super) fn configure_panel(
        &mut self,
        ui: &mut Ui,
        feature: &super::Feature,
        snapshot: &Snapshot,
        port: &mut dyn FeaturesPort,
    ) -> Option<Outcome> {
        let mut draft = self.configure.clone().filter(|draft| draft.feature == feature.id)?;
        let t = Tokens::get(ui.ctx());
        let tool = snapshot.tools.iter().find(|tool| tool.feature == feature.id);
        let mut sample_act = None;
        let mut selected = draft.choice.clone();
        let mut apply = false;
        egui::Frame::new().fill(t.sunken).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Model")).color(t.ink_strong));
                let selected_text = match &selected {
                    DraftChoice::Automatic => tl!("Automatic (best available)").to_owned(),
                    DraftChoice::Model(connection, model) => tool
                        .and_then(|tool| tool.choices.iter().find(|choice| &choice.connection == connection && &choice.model == model))
                        .map_or_else(|| tl!("Chosen model unavailable").to_owned(), |choice| choice.label.clone()),
                };
                egui::ComboBox::from_id_salt(("configure-model", &feature.id)).selected_text(selected_text).show_ui(ui, |ui| {
                    if ui.selectable_label(selected == DraftChoice::Automatic, tl!("Automatic (best available)")).clicked() {
                        selected = DraftChoice::Automatic;
                    }
                    if let Some(tool) = tool {
                        for choice in &tool.choices {
                            let next = DraftChoice::Model(choice.connection.clone(), choice.model.clone());
                            let on = selected == next;
                            let location = if choice.local { tl!("Runs on this computer") } else { tl!("Leaves this computer") };
                            let label = format!("{} · {}", choice.label, location);
                            if ui.selectable_label(on, label).clicked() {
                                selected = next;
                            }
                        }
                    }
                });
            });

            if let Some(tool) = tool {
                engine_picker(ui, feature, tool, &mut selected);
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Apply to")).color(t.muted));
                egui::ComboBox::from_id_salt(("configure-scope", &feature.id)).selected_text(scope_label(draft.apply_to)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.apply_to, ApplyTo::ThisTool, tl!("This tool"));
                    ui.selectable_value(&mut draft.apply_to, ApplyTo::AllCompatible, tl!("All compatible tools"));
                    ui.selectable_value(&mut draft.apply_to, ApplyTo::SpecificTools, tl!("Choose specific tools"));
                });
            });

            let compatible = compatible_features(self, snapshot, &selected);
            match draft.apply_to {
                ApplyTo::ThisTool => {
                    draft.targets.clear();
                    draft.targets.insert(feature.id.clone());
                }
                ApplyTo::AllCompatible => {
                    draft.targets = compatible.iter().map(|candidate| candidate.id.clone()).collect();
                    ui.label(
                        RichText::new(crate::i18n::fmt(tl!("This choice works for {n} tools."), &[("n", &compatible.len().to_string())]))
                            .size(12.0)
                            .color(t.muted),
                    );
                }
                ApplyTo::SpecificTools => {
                    for candidate in &self.features {
                        let available = compatible.iter().any(|item| item.id == candidate.id);
                        if !available {
                            continue;
                        }
                        let mut checked = draft.targets.contains(&candidate.id);
                        if ui.checkbox(&mut checked, crate::i18n::t(&candidate.title)).changed() {
                            if checked {
                                draft.targets.insert(candidate.id.clone());
                            } else {
                                draft.targets.remove(&candidate.id);
                            }
                        }
                    }
                    draft.targets.retain(|id| compatible.iter().any(|candidate| &candidate.id == id));
                }
            }

            if let Some(tool) = tool {
                ui.add_space(6.0);
                sample_row(ui, tool, sample_for(snapshot, &feature.id), &mut sample_act);
            }
            ui.add_space(8.0);
            apply = widgets::button(ui, tl!("Apply"), Kind::Primary, 0.0).clicked();
        });
        draft.choice = selected;
        self.configure = Some(draft.clone());
        if let Some(action) = sample_act {
            match action {
                Act::Choose(choice) => self.choose(&feature.id, choice, port),
                Act::Test { agreed } => port.test(&feature.id, agreed),
                Act::CancelTest => port.cancel_test(),
                Act::Dismiss => port.dismiss_test(),
            }
        }
        if apply {
            self.apply_config(&draft, snapshot, port);
            self.configure = None;
        }
        None
    }

    fn apply_config(&mut self, draft: &ConfigureState, snapshot: &Snapshot, port: &mut dyn FeaturesPort) {
        let targets: Vec<String> = match draft.apply_to {
            ApplyTo::ThisTool => vec![draft.feature.clone()],
            ApplyTo::AllCompatible => compatible_features(self, snapshot, &draft.choice).iter().map(|feature| feature.id.clone()).collect(),
            ApplyTo::SpecificTools => draft.targets.iter().cloned().collect(),
        };
        let choice = match &draft.choice {
            DraftChoice::Automatic => None,
            DraftChoice::Model(connection, model) => Some((connection.clone(), model.clone())),
        };
        let mut applied = 0;
        let mut failed = 0;
        let mut failure_message = None;
        for id in targets {
            if !compatible_tool(snapshot, &id, &draft.choice) {
                continue;
            }
            match port.choose(&id, choice.clone()) {
                Ok(()) => applied += 1,
                Err(message) => {
                    failed += 1;
                    failure_message.get_or_insert(message);
                }
            }
        }
        self.notice = if let Some(message) = failure_message {
            Some((message, true))
        } else if failed == 0 {
            Some((crate::i18n::fmt(tl!("Applied to {n} tools."), &[("n", &applied.to_string())]), false))
        } else {
            Some((crate::i18n::fmt(tl!("Applied to {n} tools. Some choices could not be saved."), &[("n", &applied.to_string())]), true))
        };
    }
}

fn scope_label(scope: ApplyTo) -> String {
    crate::i18n::t(match scope {
        ApplyTo::ThisTool => "This tool",
        ApplyTo::AllCompatible => "All compatible tools",
        ApplyTo::SpecificTools => "Choose specific tools",
    })
    .to_owned()
}

fn sample_for<'a>(snapshot: &'a Snapshot, feature: &str) -> Option<&'a Sample> {
    (snapshot.sample.feature() == Some(feature)).then_some(&snapshot.sample)
}

fn compatible_features<'a>(ui: &'a FeaturesUi, snapshot: &Snapshot, choice: &DraftChoice) -> Vec<&'a super::Feature> {
    ui.features.iter().filter(|feature| compatible_tool(snapshot, &feature.id, choice)).collect()
}

fn compatible_tool(snapshot: &Snapshot, feature: &str, choice: &DraftChoice) -> bool {
    match choice {
        DraftChoice::Automatic => snapshot.tools.iter().any(|tool| tool.feature == feature),
        DraftChoice::Model(connection, model) => snapshot.tools.iter().any(|tool| {
            tool.feature == feature && tool.choices.iter().any(|candidate| &candidate.connection == connection && &candidate.model == model)
        }),
    }
}

fn engine_picker(ui: &mut Ui, feature: &super::Feature, tool: &ToolState, selected: &mut DraftChoice) {
    let DraftChoice::Model(connection, model) = selected else {
        ui.label(RichText::new(tl!("Choose a model on this computer to choose an engine.")).size(12.0).color(Tokens::get(ui.ctx()).muted));
        return;
    };
    let model = model.clone();
    let connection = connection.clone();
    let current = tool.choices.iter().find(|choice| choice.local && choice.connection == connection && choice.model == model);
    let Some(current) = current else {
        ui.label(RichText::new(tl!("Choose a model on this computer to choose an engine.")).size(12.0).color(Tokens::get(ui.ctx()).muted));
        return;
    };

    let mut providers = Vec::new();
    for choice in tool.choices.iter().filter(|choice| choice.local) {
        if !providers.contains(&choice.provider) {
            providers.push(choice.provider.clone());
        }
    }
    if let Some(recipe) =
        feature.locals.iter().find(|recipe| recipe.workflow == model.as_str() || recipe.native.iter().any(|route| route.model == model.as_str()))
    {
        if recipe.usable && !providers.iter().any(|provider| provider == "comfyui") {
            providers.push("comfyui".to_owned());
        }
        for route in &recipe.native {
            if !providers.contains(&route.provider) {
                providers.push(route.provider.clone());
            }
        }
    }

    let mut selected_provider = current.provider.clone();
    let mut next_choice = None;
    let mut unavailable = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Engine")).color(Tokens::get(ui.ctx()).ink_strong));
        egui::ComboBox::from_id_salt(("configure-engine", &feature.id)).selected_text(provider_name(&selected_provider)).show_ui(ui, |ui| {
            for provider in &providers {
                let target = tool.choices.iter().find(|choice| choice.local && choice.provider == *provider && choice.model == model);
                let label = provider_name(provider);
                if let Some(target) = target {
                    let on = provider == &selected_provider;
                    if ui.selectable_label(on, &label).clicked() {
                        selected_provider = provider.clone();
                        next_choice = Some(DraftChoice::Model(target.connection.clone(), target.model.clone()));
                    }
                } else {
                    unavailable = true;
                    ui.add_enabled_ui(false, |ui| {
                        let _ = ui.button(&label);
                    });
                }
            }
        });
    });
    if unavailable {
        ui.label(RichText::new(tl!("This model is not available with every engine.")).size(12.0).color(Tokens::get(ui.ctx()).muted));
    }
    if let Some(next) = next_choice {
        *selected = next;
    }
}
