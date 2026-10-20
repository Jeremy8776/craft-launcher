//! The Set up AI window, with a fixed settings rail and a scrollable section body.

use std::time::Duration;

use egui::{RichText, Ui, vec2};

use super::{FeaturesUi, Install, Outcome, Page, Sample, Snapshot};
use crate::app::Host;
use crate::connections::ConnectionsUi;
use crate::theme::Tokens;
use crate::widgets::{self, Kind};

const FOOTER_H: f32 = 105.0;
const RAIL_W: f32 = 158.0;

pub(super) fn needs_poll(snapshot: &Snapshot, connections_testing: bool) -> bool {
    snapshot.scanning
        || matches!(snapshot.install, Install::Running { .. })
        || matches!(snapshot.sample, Sample::Running { .. })
        || snapshot.folders.busy
        || connections_testing
}

impl FeaturesUi {
    /// Draws the Set up AI window. Returns true when the person closed it.
    pub fn show_setup(&mut self, ui: &mut Ui, host: &mut dyn Host, conns: &mut ConnectionsUi) -> bool {
        let snapshot = {
            let Some(port) = host.features() else { return true };
            self.ensure_loaded(port);
            port.snapshot()
        };
        if let Some(port) = host.connections() {
            conns.ensure_loaded(port);
        }
        if needs_poll(&snapshot, conns.is_testing()) {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        let t = Tokens::get(ui.ctx());
        let screen = ui.ctx().content_rect();
        let width = 900.0f32.min(screen.width() - 48.0).max(420.0);
        let height = (screen.height() * 0.82).clamp(360.0, 900.0);
        ui.set_width(width);
        let mut close = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(widgets::heading(tl!("Set up AI"), 22.0, t.ink_strong));
                ui.label(RichText::new(tl!("For every Craft app")).size(12.0).color(t.faint));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                close = widgets::button(ui, tl!("Close"), Kind::Secondary, 86.0).clicked();
            });
        });
        ui.add_space(12.0);

        let current = self.page.clone();
        let current_section = if matches!(current, Page::Local(..)) { Page::Models } else { current.clone() };
        let mut next_page = None;
        ui.allocate_ui_with_layout(vec2(width, height), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_width(width);
            ui.vertical(|ui| {
                ui.set_width(RAIL_W);
                for (label, page) in section_items() {
                    let selected = next_page.as_ref().unwrap_or(&current_section) == &page;
                    if ui.selectable_label(selected, label).clicked() {
                        next_page = Some(page);
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let active = next_page.as_ref().unwrap_or(&current).clone();
                let body_width = (width - RAIL_W - 30.0).max(260.0);
                ui.set_width(body_width);
                if active == Page::History {
                    egui::ScrollArea::vertical().id_salt("setup-ai-section").max_height(height - FOOTER_H).auto_shrink([false, true]).show(
                        ui,
                        |ui| {
                            ui.set_width(body_width - 8.0);
                            self.page_contents(ui, host, conns, &snapshot, &active);
                        },
                    );
                    ui.add_space(10.0);
                    super::tool::hline(ui);
                    ui.add_space(8.0);
                    if let Some(port) = host.features() {
                        self.history_footer(ui, port);
                    }
                } else {
                    egui::ScrollArea::vertical().id_salt("setup-ai-section").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                        ui.set_width(body_width - 8.0);
                        self.page_contents(ui, host, conns, &snapshot, &active);
                    });
                }
            });
        });
        if let Some(page) = next_page {
            if page == Page::History
                && current != Page::History
                && let Some(port) = host.features()
            {
                self.open_history(port);
            }
            self.page = page;
        }
        close
    }

    fn page_contents(&mut self, ui: &mut Ui, host: &mut dyn Host, conns: &mut ConnectionsUi, snapshot: &Snapshot, page: &Page) {
        match page {
            Page::Home => {
                if let Some(port) = host.features() {
                    match super::overview::show(self, ui, snapshot, port) {
                        Outcome::AddConnection(preset) => self.begin_connect(conns, Some(preset)),
                        Outcome::AddServer => self.begin_connect(conns, None),
                        Outcome::None | Outcome::Close => {}
                    }
                }
            }
            Page::Local(feature, recipe) => {
                if let Some(port) = host.features()
                    && let Outcome::AddConnection(preset) = self.setup(ui, feature, recipe, snapshot, port)
                {
                    self.begin_connect(conns, Some(preset));
                }
            }
            Page::Connections => {
                if let Some(port) = host.connections() {
                    let width = ui.available_width();
                    conns.show_in_setup(ui, port, width);
                    if conns.take_changed() {
                        self.refresh_pending = true;
                    }
                }
            }
            Page::Models => {
                if let Some(port) = host.features() {
                    let rows = conns.rows.clone();
                    self.models_body(ui, snapshot, &rows, port);
                }
            }
            Page::Engines => {
                if let Some(port) = host.features() {
                    self.engines_body(ui, snapshot, port);
                }
            }
            Page::History => self.history_body(ui),
        }
    }

    fn begin_connect(&mut self, conns: &mut ConnectionsUi, preset: Option<&'static craft_ai_model::Preset>) {
        match preset {
            Some(preset) => conns.begin_add(preset),
            None => conns.begin_pick(),
        }
        self.page = Page::Connections;
        self.notice = None;
    }
}

fn section_items() -> [(String, Page); 5] {
    [
        (tl!("Tools").to_owned(), Page::Home),
        (tl!("Usage").to_owned(), Page::History),
        (tl!("Connections").to_owned(), Page::Connections),
        (tl!("Models").to_owned(), Page::Models),
        (tl!("Engines").to_owned(), Page::Engines),
    ]
}
