//! The Set up AI window. The Home page starts from AI tools; Advanced keeps less-used settings.
//!
//! Connecting a service makes its models available. A person chooses the model for each tool;
//! refreshing a connection never changes a choice.

use std::time::Duration;

use craft_ai_features::Route;
use craft_ai_model::{ConnectionId, PRESETS, Preset};
use egui::{RichText, Ui, vec2};

use super::{FeaturesPort, FeaturesUi, Install, Outcome, Page, Sample, Snapshot};
use crate::app::Host;
use crate::connections::{ConnectionsUi, KeyView, Row, Screen as ConnScreen};
use crate::theme::Tokens;
use crate::widgets::{self, Kind};

/// Room kept under a pinned page's scrolling body for its footer (rule, notice, buttons).
const PINNED_FOOTER_H: f32 = 110.0;

/// What a press on the home page asks for.
pub(super) enum Act {
    None,
    Edit(ConnectionId),
    Remove(ConnectionId),
}

/// A saved connection that is the hosted service `preset`, if there is one.
fn connected<'a>(rows: &'a [Row], preset: &Preset) -> Option<&'a Row> {
    let same = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    rows.iter().find(|r| r.provider == preset.provider && same(&r.endpoint, preset.endpoint))
}

/// Whether a connection can be used right now.
fn working(row: &Row) -> bool {
    row.enabled && matches!(row.key, KeyView::Saved | KeyView::NotNeeded)
}

pub(super) fn needs_poll(snapshot: &super::Snapshot, connections_testing: bool) -> bool {
    snapshot.scanning
        || matches!(snapshot.install, Install::Running { .. })
        || matches!(snapshot.sample, Sample::Running { .. })
        || snapshot.folders.busy
        || connections_testing
}

#[cfg(any())]
impl FeaturesUi {
    /// Draws the window. Returns true when the person closed it.
    pub fn show_setup(&mut self, ui: &mut Ui, host: &mut dyn Host, conns: &mut ConnectionsUi) -> bool {
        let snapshot = {
            let Some(port) = host.features() else { return true };
            self.ensure_loaded(port);
            port.snapshot()
        };
        if let Some(port) = host.connections() {
            conns.ensure_loaded(port);
        }
        let busy = needs_poll(&snapshot, conns.is_testing());
        if busy {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        let t = Tokens::get(ui.ctx());
        let screen = ui.ctx().content_rect();
        // The history is a table and a chart: it gets a wider, taller window than the setup cards.
        let on_history = self.page == Page::History;
        let on_advanced = self.page == Page::Advanced;
        let width = (if on_history { 920.0f32 } else { 760.0 }).min(screen.width() - 80.0);
        let height = if on_history { (screen.height() * 0.84).clamp(360.0, 880.0) } else { (screen.height() * 0.74).clamp(320.0, 640.0) };
        ui.set_width(width);

        let mut close = false;
        let mut advanced = false;
        let mut history = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(widgets::heading(tl!("Set up AI"), 22.0, t.ink_strong));
                widgets::hud(ui, tl!("For every Craft app"), t.faint);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                close = widgets::button(ui, tl!("Close"), Kind::Secondary, 90.0).clicked();
                if self.page == Page::Home {
                    advanced = widgets::button(ui, tl!("Advanced"), Kind::Secondary, 0.0)
                        .on_hover_text(tl!("Add another server, or choose which model each tool uses"))
                        .clicked();
                    history = widgets::button(ui, tl!("History"), Kind::Secondary, 0.0)
                        .on_hover_text(tl!("Every AI job your Craft apps have run, and what it cost"))
                        .clicked();
                }
            });
        });
        ui.add_space(14.0);
        if advanced {
            self.page = Page::Advanced;
        }
        if history && let Some(port) = host.features() {
            self.open_history(port);
        }

        if on_history || on_advanced {
            // These pages size to what they show, up to the height limit, so a short list does
            // not leave an empty gap under its buttons. The footer sits outside the scroll area,
            // so it stays in view however long the page.
            egui::ScrollArea::vertical().id_salt("setup-ai").max_height(height - PINNED_FOOTER_H).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(width - 16.0);
                self.page_contents(ui, host, conns, &snapshot);
            });
            ui.add_space(14.0);
            super::tool::hline(ui);
            ui.add_space(12.0);
            if on_history && let Some(port) = host.features() {
                self.history_footer(ui, port);
            } else if on_advanced {
                self.advanced_footer(ui);
            }
        } else {
            ui.allocate_ui_with_layout(vec2(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("setup-ai").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(width - 16.0);
                    self.page_contents(ui, host, conns, &snapshot);
                });
            });
        }
        close
    }

    fn page_contents(&mut self, ui: &mut Ui, host: &mut dyn Host, conns: &mut ConnectionsUi, snapshot: &Snapshot) {
        match self.page.clone() {
            Page::Home => {
                let Some(port) = host.features() else { return };
                let (outcome, act) = self.home(ui, snapshot, conns, port);
                self.apply(act, conns);
                match outcome {
                    Outcome::AddConnection(preset) => self.begin_connect(conns, Some(preset)),
                    Outcome::AddServer => self.begin_connect(conns, None),
                    Outcome::None | Outcome::Close => {}
                }
            }
            Page::Local(id, recipe) => {
                let Some(port) = host.features() else { return };
                if let Outcome::AddConnection(preset) = self.setup(ui, &id, &recipe, snapshot, port) {
                    self.begin_connect(conns, Some(preset));
                }
            }
            Page::Connect => {
                if matches!(conns.screen, ConnScreen::List) {
                    // The form was saved, cancelled or the removal confirmed: back to the list.
                    conns.open();
                    self.back();
                } else if let Some(port) = host.connections() {
                    conns.show(ui, port);
                }
            }
            Page::History => self.history_body(ui),
            Page::Advanced => {
                let Some(port) = host.features() else { return };
                let (outcome, act) = self.advanced_body(ui, snapshot, port, &conns.rows);
                self.apply(act, conns);
                match outcome {
                    Outcome::AddConnection(preset) => self.begin_connect(conns, Some(preset)),
                    Outcome::AddServer => self.begin_connect(conns, None),
                    Outcome::None | Outcome::Close => {}
                }
            }
        }
    }

    /// Opens the connection form for a service, or the list of every service when `None`.
    fn begin_connect(&mut self, conns: &mut ConnectionsUi, preset: Option<&'static Preset>) {
        match preset {
            Some(preset) => conns.begin_add(preset),
            None => conns.begin_pick(),
        }
        self.page = Page::Connect;
        self.notice = None;
    }

    fn apply(&mut self, act: Act, conns: &mut ConnectionsUi) {
        match act {
            Act::None => {}
            Act::Edit(id) => {
                conns.begin_edit(&id);
                self.page = Page::Connect;
            }
            Act::Remove(id) => {
                conns.begin_remove(&id);
                self.page = Page::Connect;
            }
        }
    }

    fn home(&mut self, ui: &mut Ui, snapshot: &Snapshot, conns: &ConnectionsUi, port: &mut dyn FeaturesPort) -> (Outcome, Act) {
        super::overview::show(self, ui, snapshot, conns, port)
    }

    /// The saved connections that are not one of the hosted services the list has a card for.
    pub(super) fn unmatched<'a>(&self, rows: &'a [Row]) -> Vec<&'a Row> {
        rows.iter().filter(|r| !self.is_listed(r)).collect()
    }

    /// Whether a saved connection is one of the hosted services the list already shows a card for.
    fn is_listed(&self, row: &Row) -> bool {
        self.options.iter().any(|o| match &o.route {
            Route::Service { preset } => PRESETS.iter().find(|p| p.key == preset).is_some_and(|p| connected(std::slice::from_ref(row), p).is_some()),
            Route::Local { .. } | Route::Native { .. } => false,
        })
    }
}

/// Each saved server as a row, with what to do about it. Returns what was pressed, if anything.
pub(super) fn server_rows(ui: &mut Ui, rows: &[&Row]) -> Option<Act> {
    let mut act = None;
    for row in rows {
        ui.push_id(row.id.as_str(), |ui| {
            if let Some(next) = other_row(ui, row) {
                act = Some(next);
            }
        });
        ui.add_space(6.0);
    }
    act
}

/// A saved connection that is not one of the listed services (a local server, a custom address):
/// its name, whether it works, where its data goes, and how many models it offers.
fn other_row(ui: &mut Ui, row: &Row) -> Option<Act> {
    let t = Tokens::get(ui.ctx());
    let mut act = None;
    egui::Frame::new().stroke(t.hairline()).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(widgets::heading(&row.label, 15.0, if row.enabled { t.ink_strong } else { t.faint }));
                    if working(row) {
                        widgets::badge(ui, tl!("Connected"), t.good);
                    } else if !row.enabled {
                        widgets::badge(ui, tl!("Turned off"), t.faint);
                    } else {
                        widgets::badge(ui, tl!("Key missing"), t.bad);
                    }
                });
                let place = if row.local { tl!("Runs on this computer") } else { tl!("Leaves this computer") };
                let models = if row.models == 0 {
                    tl!("No models found yet").to_owned()
                } else {
                    crate::i18n::tn(row.models as u64, "{n} model", "{n} models")
                };
                ui.label(RichText::new(format!("{place} · {models}")).size(12.0).color(if row.local { t.muted } else { t.warn }));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, tl!("Remove"), Kind::Secondary, 0.0).clicked() {
                    act = Some(Act::Remove(row.id.clone()));
                }
                if widgets::button(ui, tl!("Edit"), Kind::Secondary, 0.0).clicked() {
                    act = Some(Act::Edit(row.id.clone()));
                }
            });
        });
    });
    act
}
