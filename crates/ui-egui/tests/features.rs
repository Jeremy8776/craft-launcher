//! The Set up AI window driven the way a person would: open it, see what can be set up and what
//! each thing switches on, set one up or connect a service. Fake ports stand in for the disk, the
//! network and the keychain; the real ones are covered where they live, in the desktop shell.

use std::cell::RefCell;
use std::rc::Rc;

use artcraft_launcher_core::protocol::{Backend, Capabilities, Command, Event};
use artcraft_launcher_core::{Arch, Os, Platform};
use artcraft_launcher_ui_egui::features::{Install, LocalUse, Sample, Snapshot, ToolChoice, ToolState, ToolStatus};
use artcraft_launcher_ui_egui::i18n::{Lang, set_test_lang};
use artcraft_launcher_ui_egui::theme::ThemeChoice;
use artcraft_launcher_ui_egui::{ConnectionsPort, Failure, FeaturesPort, Host, KeyView, LauncherUi, Row, Settings, TestOutcome, TestTarget};
use craft_ai_features::{Feature, InstalledFile, Inventory, builtin, plans};
use craft_ai_model::{Billing, ConnectionDraft, ConnectionId, Locality, ModelId, PRESETS};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Quiet;
impl Backend for Quiet {
    fn send(&mut self, _: Command) {}
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { manage_apps: false, reveal_label: "Show in Finder", self_update: false }
    }
    fn platform(&self) -> Option<Platform> {
        Some(Platform { os: Os::MacOs, arch: Arch::Arm64 })
    }
    fn now(&self) -> i64 {
        1_791_500_000
    }
}

/// The built-in features, with the local model optionally marked as not runnable by any app yet.
fn catalogue(not_runnable: bool) -> Vec<Feature> {
    let mut features = builtin();
    if not_runnable {
        for recipe in features.iter_mut().flat_map(|f| f.locals.iter_mut()) {
            recipe.usable = false;
        }
    }
    features
}

#[derive(Default)]
struct World {
    /// Pretend no app can run the local model yet, to keep the "planned" wording covered.
    not_runnable: bool,
    /// Narrow the rendered catalogue so a UI test never clicks a control outside the viewport.
    feature_titles: Option<Vec<String>>,
    inventory: Inventory,
    install: Install,
    scans: usize,
    installs: Vec<(String, bool)>,
    model_installs: Vec<(String, String, bool)>,
    local_uses: Vec<String>,
    model_uses: Vec<(String, String)>,
    cancels: usize,
    refreshes: usize,
    tools: Vec<ToolState>,
    sample: Sample,
    chosen: Vec<(String, Option<(ConnectionId, ModelId)>)>,
    choose_refused: Option<String>,
    tests: Vec<(String, bool)>,
    test_cancels: usize,
    dismissals: usize,
    folders: artcraft_launcher_ui_egui::features::FoldersSnapshot,
    folder_calls: Vec<String>,
}

#[derive(Clone, Default)]
struct Features(Rc<RefCell<World>>);

impl Features {
    fn only(titles: &[&str]) -> Self {
        let features = Self::default();
        features.0.borrow_mut().feature_titles = Some(titles.iter().map(|title| (*title).to_owned()).collect());
        features
    }
}

/// Generate images has two models to pick from; nothing else is connected.
fn with_image_choices(f: &Features) {
    let mut w = f.0.borrow_mut();
    w.inventory = bare_comfy();
    w.tools = vec![ToolState {
        feature: "generate-image".into(),
        choices: vec![
            ToolChoice {
                connection: ConnectionId::new("openai").unwrap(),
                model: ModelId::new("gpt-image-1").unwrap(),
                provider: "openai".into(),
                label: "GPT Image 1, OpenAI".into(),
                local: false,
            },
            ToolChoice {
                connection: ConnectionId::new("local").unwrap(),
                model: ModelId::new("flux").unwrap(),
                provider: "comfyui".into(),
                label: "Flux, My ComfyUI".into(),
                local: true,
            },
        ],
        bound: None,
        status: ToolStatus::NoChoice,
        can_test: true,
        costly: false,
    }];
}

fn pickers<'a>(h: &'a Harness<'_, State>) -> Vec<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::ComboBox)
        .filter(|node| {
            !matches!(
                node.accesskit_node().value().as_deref(),
                Some("Suite order" | "This tool" | "All compatible tools" | "Choose specific tools" | "ComfyUI" | "Inside this app" | "OpenAI")
            )
        })
        .collect()
}

fn picker_text(h: &Harness<'_, State>) -> String {
    pickers(h)[0].accesskit_node().value().map(|v| v.to_string()).unwrap_or_default()
}

fn visible_catalogue(world: &World) -> Vec<Feature> {
    let mut features = catalogue(world.not_runnable);
    if let Some(titles) = &world.feature_titles {
        features.retain(|feature| titles.contains(&feature.title));
    }
    features
}

impl FeaturesPort for Features {
    fn features(&self) -> Vec<Feature> {
        visible_catalogue(&self.0.borrow())
    }
    fn rescan(&mut self) {
        self.0.borrow_mut().scans += 1;
    }
    fn refresh_tools(&mut self) {
        self.0.borrow_mut().refreshes += 1;
    }
    fn snapshot(&mut self) -> Snapshot {
        let w = self.0.borrow();
        let features = visible_catalogue(&w);
        Snapshot {
            scanning: false,
            plans: features.iter().flat_map(|f| plans(f, &w.inventory)).collect(),
            tools: w.tools.clone(),
            install: w.install.clone(),
            sample: w.sample.clone(),
            folders: w.folders.clone(),
            local_use: LocalUse::Idle,
        }
    }
    fn choose(&mut self, feature: &str, choice: Option<(ConnectionId, ModelId)>) -> Result<(), String> {
        let mut w = self.0.borrow_mut();
        if let Some(why) = w.choose_refused.clone() {
            return Err(why);
        }
        w.chosen.push((feature.to_owned(), choice.clone()));
        if let Some(tool) = w.tools.iter_mut().find(|t| t.feature == feature) {
            tool.bound = choice.clone();
            tool.status = match &choice {
                Some(_) => ToolStatus::Ready { label: "chosen".into(), local: false },
                None => ToolStatus::NoChoice,
            };
        }
        Ok(())
    }
    fn test(&mut self, feature: &str, agreed: bool) {
        let mut w = self.0.borrow_mut();
        w.tests.push((feature.to_owned(), agreed));
        w.sample = Sample::Running { feature: feature.to_owned() };
    }
    fn cancel_test(&mut self) {
        let mut w = self.0.borrow_mut();
        w.test_cancels += 1;
        w.sample = Sample::Idle;
    }
    fn dismiss_test(&mut self) {
        let mut w = self.0.borrow_mut();
        w.dismissals += 1;
        w.sample = Sample::Idle;
    }
    fn install(&mut self, feature: &str, accepted: bool) -> Result<(), String> {
        let mut w = self.0.borrow_mut();
        w.installs.push((feature.to_owned(), accepted));
        w.install = Install::Running {
            feature: feature.to_owned(),
            recipe: "sam3-segment".into(),
            file: "sam3.safetensors".into(),
            verifying: false,
            done: 1_700_000_000,
            total: 3_449_860_632,
            step: 1,
            of: 1,
        };
        Ok(())
    }
    fn install_recipe(&mut self, feature: &str, recipe: &str, accepted: bool) -> Result<(), String> {
        let mut w = self.0.borrow_mut();
        w.model_installs.push((feature.to_owned(), recipe.to_owned(), accepted));
        w.installs.push((feature.to_owned(), accepted));
        w.install = Install::Running {
            feature: feature.to_owned(),
            recipe: recipe.to_owned(),
            file: "sam3.safetensors".into(),
            verifying: false,
            done: 1_700_000_000,
            total: 3_449_860_632,
            step: 1,
            of: 1,
        };
        Ok(())
    }
    fn cancel_install(&mut self) {
        let mut w = self.0.borrow_mut();
        w.cancels += 1;
        w.install = Install::Idle;
    }
    fn use_local(&mut self, feature: &str) -> Result<(), String> {
        self.0.borrow_mut().local_uses.push(feature.to_owned());
        Ok(())
    }
    fn use_local_recipe(&mut self, feature: &str, recipe: &str) -> Result<(), String> {
        self.0.borrow_mut().model_uses.push((feature.to_owned(), recipe.to_owned()));
        self.0.borrow_mut().local_uses.push(feature.to_owned());
        Ok(())
    }
    fn history(&mut self) -> Result<craft_ai_model::HistoryLog, String> {
        Ok(craft_ai_model::HistoryLog::default())
    }
    fn clear_history(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn export_history(&mut self, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    fn clock(&self) -> artcraft_launcher_ui_egui::features::Clock {
        artcraft_launcher_ui_egui::features::Clock::default()
    }
    fn refresh_folders(&mut self) {
        self.0.borrow_mut().folder_calls.push("refresh".into());
    }
    fn add_folder(&mut self) {
        self.0.borrow_mut().folder_calls.push("add".into());
    }
    fn set_default_folder(&mut self, id: &str) {
        self.0.borrow_mut().folder_calls.push(format!("default:{id}"));
    }
    fn rescan_folder(&mut self, id: &str) {
        self.0.borrow_mut().folder_calls.push(format!("rescan:{id}"));
    }
    fn remove_folder(&mut self, id: &str) {
        self.0.borrow_mut().folder_calls.push(format!("remove:{id}"));
    }
}

/// A connections store the tests can look inside: what was saved and deleted, and the rows shown.
#[derive(Default)]
struct ConnWorld {
    rows: Vec<Row>,
    saved: Vec<ConnectionDraft>,
    deleted: Vec<ConnectionId>,
}

#[derive(Clone, Default)]
struct Connections(Rc<RefCell<ConnWorld>>);

fn row_of(c: &craft_ai_model::Connection) -> Row {
    Row {
        id: c.id.clone(),
        label: c.label.clone(),
        provider: c.provider.clone(),
        endpoint: c.endpoint.clone().unwrap_or_default(),
        local: c.locality == Locality::Local,
        key: if c.auth.secret_ref().is_some() { KeyView::Saved } else { KeyView::NotNeeded },
        models: c.models.len(),
        bindings: 0,
        enabled: true,
        draft: ConnectionDraft::from_connection(c),
    }
}

/// A saved connection made from a service's own preset, as the form would have saved it.
fn saved_preset(key: &str) -> Row {
    let preset = PRESETS.iter().find(|p| p.key == key).unwrap();
    let mut draft = ConnectionDraft::from_preset(preset);
    draft.key = "sk-test".into();
    row_of(&draft.prepare(&[]).unwrap().connection)
}

impl ConnectionsPort for Connections {
    fn rows(&mut self) -> Result<Vec<Row>, String> {
        Ok(self.0.borrow().rows.clone())
    }
    fn save(&mut self, draft: &ConnectionDraft) -> Result<Option<String>, Failure> {
        let prepared = draft.prepare(&[]).map_err(|e| Failure { field: Some(e.field), message: e.message })?;
        let mut w = self.0.borrow_mut();
        w.saved.push(draft.clone());
        w.rows.push(row_of(&prepared.connection));
        Ok(None)
    }
    fn delete(&mut self, id: &ConnectionId) -> Result<(), String> {
        let mut w = self.0.borrow_mut();
        w.deleted.push(id.clone());
        w.rows.retain(|r| &r.id != id);
        Ok(())
    }
    fn set_enabled(&mut self, _: &ConnectionId, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn start_test(&mut self, _: TestTarget) {}
    fn poll_test(&mut self) -> Option<Result<TestOutcome, String>> {
        None
    }
}

struct Shell {
    features: Features,
    connections: Connections,
}

impl Host for Shell {
    fn connections(&mut self) -> Option<&mut dyn ConnectionsPort> {
        Some(&mut self.connections)
    }
    fn features(&mut self) -> Option<&mut dyn FeaturesPort> {
        Some(&mut self.features)
    }
}

type State = (Option<LauncherUi>, Shell);

fn harness(features: Features, connections: Connections) -> Harness<'static, State> {
    set_test_lang(Lang::EN);
    Harness::builder().with_size(egui::vec2(1180.0, 2400.0)).build_ui_state(
        |ui, state: &mut State| {
            let (ui_state, shell) = state;
            let l = ui_state.get_or_insert_with(|| {
                let s = Settings { notify: false, theme: ThemeChoice::Dark, ..Settings::default() };
                LauncherUi::new(ui.ctx(), Box::new(Quiet), s)
            });
            l.logic(&ui.ctx().clone(), shell);
            l.ui(ui, shell);
        },
        (None, Shell { features, connections }),
    )
}

fn settle(h: &mut Harness<'_, State>) {
    for _ in 0..4 {
        h.step();
    }
}

fn bare_comfy() -> Inventory {
    Inventory { comfyui_found: true, ..Inventory::default() }
}

fn with_sam3() -> Inventory {
    Inventory {
        comfyui_found: true,
        files: vec![InstalledFile { folder: "sam3".into(), filename: "sam3.safetensors".into(), size_bytes: 3_449_860_632 }],
        node_folders: vec!["comfyui-sam3".into()],
        ..Inventory::default()
    }
}

fn with_sam21() -> Inventory {
    // The checkpoint and every Florence-2-large detector file, as the catalogue lists them.
    let features = builtin();
    let recipe = features.iter().flat_map(|f| &f.locals).find(|r| r.workflow == "sam21-segment").unwrap();
    Inventory {
        comfyui_found: true,
        files: recipe
            .models
            .iter()
            .map(|m| InstalledFile { folder: m.folder.clone(), filename: m.filename.clone(), size_bytes: m.size_bytes })
            .collect(),
        node_folders: vec!["comfyui-segment-anything-2".into(), "comfyui-florence2".into()],
        ..Inventory::default()
    }
}

fn open_with(features: &Features, connections: &Connections) -> Harness<'static, State> {
    let mut h = harness(features.clone(), connections.clone());
    settle(&mut h);
    h.get_by_label("Set up AI").click();
    settle(&mut h);
    h
}

fn open(features: &Features) -> Harness<'static, State> {
    open_with(features, &Connections::default())
}

fn open_models(features: &Features) -> Harness<'static, State> {
    let mut h = open(features);
    press(&mut h, "Models");
    h
}

fn open_connections(features: &Features, connections: &Connections) -> Harness<'static, State> {
    let mut h = open_with(features, connections);
    press(&mut h, "Connections");
    h
}

fn press(h: &mut Harness<'_, State>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}

// ---- the list ----

#[test]
fn set_up_ai_lists_capabilities_and_setup_paths_first() {
    let f = Features::default();
    f.0.borrow_mut().inventory = bare_comfy();
    let h = open(&f);
    h.query_all_by_label("Set up AI").next().unwrap();
    h.get_by_role_and_label(Role::Button, "Tools");
    h.get_by_label("See which Craft apps use each AI tool and set its model.");
    h.get_by_label("Usage");
    h.get_by_label("Connections");
    h.get_by_label("Models");
    h.get_by_label("Engines");
    for name in ["Select subject", "Remove background", "Remove objects", "Generative fill", "Generate images"] {
        h.get_by_label(name);
    }
    assert!(h.query_all_by_role_and_label(Role::Button, "Configure").count() >= 1);
    assert!(h.query_by_label("Start from").is_none() && h.query_by_label("Add a provider").is_none(), "no provider form to start with");
    assert_eq!(f.0.borrow().scans, 1);
}

#[test]
fn each_tool_names_the_apps_that_may_use_it() {
    let f = Features::default();
    f.0.borrow_mut().inventory = bare_comfy();
    let h = open(&f);
    assert!(h.query_all_by_label("Generative fill").count() >= 1);
    assert!(h.query_all_by_label("Generate images").count() >= 1);
    assert!(h.query_all_by_label_contains("Suggested for PhotoCraft:").count() >= 1, "the app and its current proposal status are named");
}

#[test]
fn a_connected_model_is_offered_without_being_selected_automatically() {
    let f = Features::default();
    f.0.borrow_mut().feature_titles = Some(vec!["Generate images".into()]);
    with_image_choices(&f);
    let mut h = open(&f);
    press(&mut h, "Configure");
    assert_eq!(picker_text(&h), "Automatic (best available)");
    assert!(f.0.borrow().chosen.is_empty(), "refreshing a connection must not write a suite default");
}

#[test]
fn each_local_model_has_its_own_name_and_setup_state() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = bare_comfy();
    let mut h = open(&f);
    press(&mut h, "Models");
    for name in ["SAM 3", "SAM 2.1", "SAM 2", "SAM 1"] {
        h.get_by_label(name);
    }
}

#[test]
fn the_local_model_says_what_this_computer_still_needs() {
    let f = Features::default();
    f.0.borrow_mut().inventory = bare_comfy();
    let h = open_models(&f);
    h.get_by_label("Set up, 3.4 GB");

    let f = Features::default();
    f.0.borrow_mut().not_runnable = true;
    f.0.borrow_mut().inventory = with_sam3();
    let h = open_models(&f);
    h.get_by_label("Files in place");
    assert!(h.query_by_label("Ready").is_none(), "installed files are not a runnable tool");
    assert!(h.query_by_label("Set up, 3.4 GB").is_none());

    let f = Features::default();
    let h = open_models(&f);
    assert!(h.query_all_by_label("The local service for these files was not found.").next().is_some());
}

#[test]
fn finishing_sam3_does_not_claim_sam21_was_downloaded() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = bare_comfy();
    let mut h = open_models(&f);
    press(&mut h, "Set up, 2.0 GB");
    {
        let mut w = f.0.borrow_mut();
        w.inventory = with_sam21();
        w.install = Install::Done { feature: "select-subject".into(), recipe: "sam3-segment".into() };
    }
    settle(&mut h);
    assert!(h.query_by_label("Model files installed. The app tool is not connected yet.").is_none());
}

#[test]
fn completed_download_does_not_claim_the_tool_can_run() {
    let (f, mut h) = open_setup_with(true);
    {
        let mut w = f.0.borrow_mut();
        w.inventory = with_sam3();
        w.install = Install::Done { feature: "select-subject".into(), recipe: "sam3-segment".into() };
    }
    settle(&mut h);
    assert!(h.query_by_label("Installed. The files are ready to test.").is_none());
}

#[test]
fn completed_download_of_a_runnable_model_says_files_are_ready_to_test() {
    let (f, mut h) = open_setup();
    {
        let mut w = f.0.borrow_mut();
        w.inventory = with_sam3();
        w.install = Install::Done { feature: "select-subject".into(), recipe: "sam3-segment".into() };
    }
    settle(&mut h);
    h.get_by_label("Installed. The files are ready to test.");
}

#[test]
fn a_local_model_no_app_can_run_yet_says_so_instead_of_promising_a_tool() {
    let f = Features::default();
    f.0.borrow_mut().not_runnable = true;
    f.0.borrow_mut().inventory = bare_comfy();
    let h = open_models(&f);
    assert!(h.query_all_by_label("Setup only").next().is_some());
    assert_eq!(h.query_all_by_label("Models marked Setup only can be installed, but no Craft app can use them yet.").count(), 1);
}

#[test]
fn a_model_whose_files_are_here_is_not_told_to_download_them() {
    let f = Features::default();
    f.0.borrow_mut().not_runnable = true;
    f.0.borrow_mut().inventory = with_sam3();
    let h = open_models(&f);
    h.get_by_label("Files in place");
    assert!(h.query_all_by_label("Setup only").next().is_some());
    assert_eq!(h.query_all_by_label("Models marked Setup only can be installed, but no Craft app can use them yet.").count(), 1);
    assert!(h.query_by_label("Set up, 3.4 GB").is_none());
}

#[test]
fn existing_local_files_need_an_explicit_use_action_before_registration() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = with_sam3();
    let mut h = open_models(&f);
    assert!(f.0.borrow().local_uses.is_empty(), "opening must not bind a model");
    press(&mut h, "Check and use model");
    assert_eq!(f.0.borrow().local_uses, ["select-subject"]);
}

#[test]
fn ready_second_model_uses_its_own_recipe_when_enabled() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = with_sam21();
    let mut h = open_models(&f);
    press(&mut h, "Check and use model");
    assert_eq!(f.0.borrow().model_uses, [("select-subject".into(), "sam21-segment".into())]);
}

// ---- local setup ----

fn open_setup() -> (Features, Harness<'static, State>) {
    open_setup_with(false)
}

fn open_setup_with(not_runnable: bool) -> (Features, Harness<'static, State>) {
    let f = Features::default();
    f.0.borrow_mut().feature_titles = Some(vec!["Select subject".into()]);
    f.0.borrow_mut().not_runnable = not_runnable;
    f.0.borrow_mut().inventory = bare_comfy();
    let mut h = open_models(&f);
    press(&mut h, "Set up, 3.4 GB");
    (f, h)
}

#[test]
fn opening_sam21_setup_shows_only_its_files_and_size() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = bare_comfy();
    let mut h = open_models(&f);
    press(&mut h, "Set up, 2.0 GB");
    h.get_by_label("Set up SAM 2.1");
    h.get_by_label_contains("sam2.1_hiera_large-fp16.safetensors, 449 MB");
    h.get_by_label_contains("Florence-2-large/model.safetensors, 1.6 GB");
    h.get_by_label_contains("Total 2.0 GB");
    assert!(h.query_by_label_contains("sam3.safetensors, 3.4 GB").is_none());
}

#[test]
fn downloading_sam21_names_its_recipe_to_the_host() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = bare_comfy();
    let mut h = open_models(&f);
    press(&mut h, "Set up, 2.0 GB");
    // The setup page now lists what the model runs, so the button sits below the first screen.
    h.get_by_role_and_label(Role::Button, "Download 2.0 GB").scroll_to_me();
    settle(&mut h);
    press(&mut h, "Download 2.0 GB");
    assert_eq!(f.0.borrow().model_installs, [("select-subject".into(), "sam21-segment".into(), false)]);
}

#[test]
fn setting_up_shows_the_size_the_licence_and_waits_for_agreement() {
    let (f, mut h) = open_setup();
    h.get_by_label("Set up SAM 3");
    h.get_by_label("Will be downloaded");
    h.get_by_label_contains("sam3.safetensors, 3.4 GB, from huggingface.co");
    h.get_by_label("Licence");
    assert!(h.query_all_by_label_contains("community copy").count() >= 1);
    h.get_by_label_contains("Also needs the ComfyUI-SAM3 add-on");
    // Pressing Download before agreeing does nothing.
    h.get_by_label("Download 3.4 GB").click();
    settle(&mut h);
    assert!(f.0.borrow().installs.is_empty());
    h.get_by_label("I have read the licence and accept it").click();
    settle(&mut h);
    h.get_by_label("Download 3.4 GB").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().installs, [("select-subject".to_owned(), true)]);
}

#[test]
fn progress_and_cancel_are_shown_while_downloading() {
    let (f, mut h) = open_setup();
    h.get_by_label("I have read the licence and accept it").click();
    settle(&mut h);
    h.get_by_label("Download 3.4 GB").click();
    settle(&mut h);
    h.get_by_label_contains("Downloading sam3.safetensors: 1.7 GB of 3.4 GB");
    assert!(h.query_by_role(Role::ProgressIndicator).is_some());
    h.get_by_label("Cancel download").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().cancels, 1);
}

#[test]
fn a_failed_download_says_why() {
    let (f, mut h) = open_setup();
    f.0.borrow_mut().install =
        Install::Failed { feature: "select-subject".into(), recipe: "sam3-segment".into(), message: "the source refused the download".into() };
    settle(&mut h);
    h.get_by_label_contains("The download failed: the source refused the download");
}

#[test]
fn back_from_setup_returns_to_the_list_and_looks_at_the_tools_again() {
    let (f, mut h) = open_setup();
    press(&mut h, "Back");
    h.get_by_role_and_label(Role::Button, "Models");
    assert_eq!(f.0.borrow().refreshes, 2);
}

// ---- hosted services ----

#[test]
fn connect_opens_that_services_own_short_form_and_back_returns_to_the_list() {
    let f = Features::only(&["Generate images"]);
    let mut h = open_connections(&f, &Connections::default());
    press(&mut h, "Add connection");
    // Pick a hosted service from the provider list.
    h.query_all_by_role_and_label(Role::Button, "OpenAI").next().unwrap().click();
    settle(&mut h);
    h.get_by_label("Connect OpenAI");
    h.get_by_label("Data sent to this connection leaves this computer.");
    assert!(h.query_by_label("Add a provider").is_none());
    press(&mut h, "Back");
    h.get_by_role_and_label(Role::Button, "Connections");
    assert!(h.query_by_label("Connect OpenAI").is_none());
}

#[test]
fn saving_a_key_connects_the_service_and_the_list_shows_it() {
    let f = Features::only(&["Generate images"]);
    let c = Connections::default();
    let mut h = open_connections(&f, &c);
    press(&mut h, "Add connection");
    h.query_all_by_role_and_label(Role::Button, "OpenAI").next().unwrap().click();
    settle(&mut h);
    h.get_by_role(Role::PasswordInput).focus();
    h.get_by_role(Role::PasswordInput).type_text("sk-test-123");
    settle(&mut h);
    press(&mut h, "Save");
    assert_eq!(c.0.borrow().saved.len(), 1);
    assert_eq!(c.0.borrow().saved[0].key, "sk-test-123");
    h.get_by_role_and_label(Role::Button, "Connections");
    h.get_by_label("Key saved");
    h.get_by_role_and_label(Role::Button, "Edit");
    assert_eq!(f.0.borrow().refreshes, 2, "the tools are read again so the new connection can be chosen");
    assert!(f.0.borrow().chosen.is_empty(), "saving a key does not select a model");
}

#[test]
fn a_connected_service_can_be_edited_and_removed_after_asking() {
    let f = Features::only(&[]);
    let c = Connections::default();
    c.0.borrow_mut().rows = vec![saved_preset("openai")];
    let mut h = open_connections(&f, &c);
    h.get_by_role_and_label(Role::Button, "Connections");
    h.get_by_label("Key saved");
    press(&mut h, "Edit");
    h.get_by_label("Edit connection");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut h);
    press(&mut h, "Remove");
    h.get_by_label("Remove OpenAI?");
    h.get_by_label("Its saved key is deleted too.");
    assert!(c.0.borrow().deleted.is_empty(), "asked first");
    h.get_by_role_and_label(Role::Button, "Remove").click();
    settle(&mut h);
    assert_eq!(c.0.borrow().deleted.len(), 1);
    h.get_by_role_and_label(Role::Button, "Connections");
    assert!(h.query_by_label("Key saved").is_none());
}

#[test]
fn a_connection_that_is_not_one_of_the_listed_services_is_still_shown() {
    let f = Features::default();
    let c = Connections::default();
    c.0.borrow_mut().rows = vec![saved_preset("openai"), saved_preset("ollama")];
    let h = open_connections(&f, &c);
    h.get_by_role_and_label(Role::Button, "Connections");
    h.get_by_label("Ollama");
    assert!(h.query_all_by_label("OpenAI").count() >= 1, "the saved service remains visible");
}

#[test]
fn add_connection_starts_from_the_connections_section_and_cancel_returns_to_it() {
    let f = Features::default();
    let mut h = open_connections(&f, &Connections::default());
    assert!(h.query_by_role_and_label(Role::Button, "Add connection").is_some());
    assert!(h.query_by_role_and_label(Role::Button, "OpenAI").is_none());
    press(&mut h, "Add connection");
    h.get_by_label("Add a provider");
    h.get_by_label("Choose the one you use");
    assert!(h.query_by_role_and_label(Role::Button, "SAM 3 (local)").is_none(), "SAM 3 is a model, not a server address");
    press(&mut h, "Cancel");
    h.get_by_role_and_label(Role::Button, "Connections");
    assert!(h.query_by_label("Add a provider").is_none());
}

#[test]
fn close_closes_the_window_and_opening_again_looks_afresh() {
    let f = Features::default();
    let mut h = open(&f);
    h.get_by_label("Close").click();
    settle(&mut h);
    assert!(h.query_by_label("AI tools").is_none());
    h.get_by_label("Set up AI").click();
    settle(&mut h);
    assert_eq!(f.0.borrow().scans, 2);
    assert_eq!(f.0.borrow().refreshes, 2);
}

#[path = "features/folders.rs"]
mod folder_tests;
#[path = "features/tools.rs"]
mod tools;

#[test]
fn sam21_shows_publisher_terms_and_never_gates_the_model_on_them() {
    let f = Features::only(&["Select subject"]);
    f.0.borrow_mut().inventory = with_sam21();
    let mut h = open_models(&f);
    // Folded until asked for, and the model can already be used: disclosure is not a gate.
    assert!(h.query_by_label("Florence-2-large detector").is_none());
    h.get_by_label("Licences of what this uses").click();
    settle(&mut h);
    h.get_by_label("Florence-2-large detector");
    h.get_by_label("SAM 2.1 checkpoint");
    assert_eq!(h.query_all_by_label("Where the publisher says so").count(), 4);
    h.get_by_label_contains("gives no warranty or indemnity");
    h.get_by_label_contains("You can run another Florence-2 variant in ComfyUI yourself");
    h.get_by_label_contains("downloads it from Hugging Face by itself");
    assert!(h.query_by_label("I have read the licence and accept it").is_none());
    h.get_by_role_and_label(Role::Button, "Check and use model");
}
