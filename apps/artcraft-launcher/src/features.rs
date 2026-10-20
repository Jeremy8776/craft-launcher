//! The desktop shell's side of the AI tools screen: finds ComfyUI, works out what each tool still
//! needs, and downloads model files, behind the UI's [`FeaturesPort`].
//!
//! Scanning and downloading each run on their own thread; the screen polls [`snapshot`] every
//! frame. Nothing here starts ComfyUI, installs a node pack or changes an existing file. A
//! download is written beside its final name and only renamed in once its size and SHA-256 match
//! (see `craft-ai-comfy-env`), and licences that need accepting are checked here as well as in the
//! screen, so a caller that skips the checkbox is refused.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::features_folders::DesktopFolders;
use crate::features_tools::Tools;
use crate::history_port::DesktopHistory;
use artcraft_launcher_ui_egui::FeaturesPort;
use artcraft_launcher_ui_egui::features::{Clock, Install, LocalUse, Snapshot};
use craft_ai_client::{AiClient, ToolDesk};
use craft_ai_comfy_env::{ComfyInstall, DownloadConfig, Downloader, Phase, scan, verify_file};
use craft_ai_features::{Feature, Inventory, State, plans};
use craft_ai_model::{ConnectionId, ErrorKind, ModelId};
use craft_ai_native::{ConsentFile, shared::SharedStore};

type Locator = Arc<dyn Fn() -> Option<ComfyInstall> + Send + Sync>;

#[derive(Default)]
struct Shared {
    scanning: bool,
    install_running: bool,
    plans: Vec<craft_ai_features::Plan>,
    comfy: Option<ComfyInstall>,
    install: Install,
    local_use: LocalUse,
}

pub struct DesktopFeatures {
    features: Vec<Feature>,
    locate: Locator,
    config: DownloadConfig,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    tools: Tools,
    folders: DesktopFolders,
    history: DesktopHistory,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A thread that panicked while holding the lock must not take the screen down with it.
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl DesktopFeatures {
    /// The built-in features, looking for ComfyUI Desktop the way it records itself.
    pub fn new() -> Self {
        Self::with(craft_ai_features::builtin(), Arc::new(ComfyInstall::discover), DownloadConfig::default(), Tools::new(standard_desk()))
    }

    pub(crate) fn with(features: Vec<Feature>, locate: Locator, config: DownloadConfig, tools: Tools) -> Self {
        Self {
            features,
            locate,
            config,
            shared: Arc::default(),
            cancel: Arc::default(),
            tools,
            folders: DesktopFolders::new(),
            history: DesktopHistory::new(),
        }
    }

    /// Scans on the calling thread and stores the result.
    fn scan_into(features: &[Feature], locate: &Locator, shared: &Mutex<Shared>) {
        let comfy = locate();
        let inventory = comfy.as_ref().map_or_else(Inventory::default, scan);
        let plans = features.iter().flat_map(|f| plans(f, &inventory)).collect();
        let mut guard = lock(shared);
        guard.plans = plans;
        guard.comfy = comfy;
        guard.scanning = false;
    }
}

impl FeaturesPort for DesktopFeatures {
    fn features(&self) -> Vec<Feature> {
        self.features.clone()
    }

    fn rescan(&mut self) {
        {
            let mut guard = lock(&self.shared);
            if guard.scanning {
                return;
            }
            guard.scanning = true;
        }
        let (features, locate, shared) = (self.features.clone(), Arc::clone(&self.locate), Arc::clone(&self.shared));
        std::thread::spawn(move || Self::scan_into(&features, &locate, &shared));
    }

    fn snapshot(&mut self) -> Snapshot {
        let guard = lock(&self.shared);
        let (tools, sample) = self.tools.snapshot();
        let folders = self.folders.snapshot();
        Snapshot {
            scanning: guard.scanning,
            plans: guard.plans.clone(),
            tools,
            install: guard.install.clone(),
            sample,
            folders,
            local_use: guard.local_use.clone(),
        }
    }

    fn refresh_tools(&mut self) {
        self.tools.refresh(&self.features);
    }

    fn refresh_folders(&mut self) {
        self.folders.refresh();
    }

    fn add_folder(&mut self) {
        self.folders.add_folder();
    }

    fn set_default_folder(&mut self, id: &str) {
        self.folders.set_default_folder(id);
    }

    fn rescan_folder(&mut self, id: &str) {
        self.folders.rescan_folder(id);
    }

    fn remove_folder(&mut self, id: &str) {
        self.folders.remove_folder(id);
    }

    fn choose(&mut self, feature: &str, choice: Option<(ConnectionId, ModelId)>) -> Result<(), String> {
        self.tools.choose(&self.features, feature, choice)
    }

    fn test(&mut self, feature: &str, agreed: bool) {
        self.tools.test(&self.features, feature, agreed);
    }

    fn cancel_test(&mut self) {
        self.tools.cancel();
    }

    fn dismiss_test(&mut self) {
        self.tools.dismiss();
    }

    fn history(&mut self) -> Result<craft_ai_model::HistoryLog, String> {
        self.history.read()
    }

    fn clear_history(&mut self) -> Result<(), String> {
        self.history.clear()
    }

    fn export_history(&mut self, csv: &str) -> Result<String, String> {
        self.history.export(csv)
    }

    fn clock(&self) -> Clock {
        self.history.clock()
    }

    fn install(&mut self, feature: &str, accepted: bool) -> Result<(), String> {
        let recipe = self.features.iter().find(|f| f.id == feature).and_then(|f| f.locals.first()).ok_or("That tool is not known.")?.workflow.clone();
        self.install_recipe(feature, &recipe, accepted)
    }

    fn install_recipe(&mut self, feature: &str, recipe: &str, accepted: bool) -> Result<(), String> {
        let (files, comfy) = {
            let mut guard = lock(&self.shared);
            if guard.install_running {
                return Err("A download is already running.".to_owned());
            }
            let plan = guard.plans.iter().find(|p| p.feature == feature && p.recipe == recipe).ok_or("That model is not known.")?;
            if plan.state != State::NeedsInstall || plan.missing_files.is_empty() {
                return Err("There is nothing to download for this tool.".to_owned());
            }
            if !plan.licences.is_empty() && !accepted {
                return Err("Accept the licence first.".to_owned());
            }
            let files = plan.missing_files.clone();
            let comfy = guard.comfy.clone().ok_or("ComfyUI was not found.")?;
            guard.install_running = true;
            guard.install = Install::Idle;
            (files, comfy)
        };
        self.cancel.store(false, Ordering::Relaxed);
        let ctx = Job {
            feature: feature.to_owned(),
            recipe: recipe.to_owned(),
            files,
            comfy,
            downloader: Downloader::new(self.config.clone()),
            cancel: Arc::clone(&self.cancel),
            shared: Arc::clone(&self.shared),
            features: self.features.clone(),
            locate: Arc::clone(&self.locate),
        };
        std::thread::spawn(move || ctx.run());
        Ok(())
    }

    fn use_local(&mut self, id: &str) -> Result<(), String> {
        let recipe = self.features.iter().find(|f| f.id == id).and_then(|f| f.locals.first()).ok_or("That tool is not known.")?.workflow.clone();
        self.use_local_recipe(id, &recipe)
    }

    fn use_local_recipe(&mut self, id: &str, recipe_id: &str) -> Result<(), String> {
        let (feature, comfy) = {
            let mut guard = lock(&self.shared);
            if matches!(guard.local_use, LocalUse::Checking { .. }) {
                return Err("A model is already being checked.".to_owned());
            }
            let plan = guard.plans.iter().find(|p| p.feature == id && p.recipe == recipe_id).ok_or("That model is not known.")?;
            if plan.state != State::Ready {
                return Err("The model files are not in place yet.".to_owned());
            }
            let feature = self
                .features
                .iter()
                .find(|f| f.id == id)
                .filter(|f| f.locals.iter().any(|r| r.workflow == recipe_id && r.usable))
                .cloned()
                .ok_or("This model cannot be used by an app yet.")?;
            let comfy = guard.comfy.clone().ok_or("The local service was not found.")?;
            guard.local_use = LocalUse::Checking { feature: id.to_owned(), recipe: recipe_id.to_owned() };
            (feature, comfy)
        };
        let shared = Arc::clone(&self.shared);
        let tools = self.tools.clone();
        let features = self.features.clone();
        let recipe = recipe_id.to_owned();
        std::thread::spawn(move || {
            let result = check_local_files(&feature, &recipe, &comfy).and_then(|()| tools.enable_local_recipe(&features, &feature, &recipe));
            lock(&shared).local_use = match result {
                Ok(enabled) => LocalUse::Enabled { feature: feature.id, recipe, bound: enabled.bound },
                Err(message) => LocalUse::Failed { feature: feature.id, recipe, message },
            };
        });
        Ok(())
    }

    fn cancel_install(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The launcher's own client: the shared connection store, keys from the environment then the OS
/// keychain, acting as the app `launcher`, with the launcher's agreements kept in its own file.
fn standard_desk() -> Result<ToolDesk, String> {
    let app = ToolDesk::app().map_err(|e| e.message)?;
    let shared = SharedStore::at_default_location().map_err(|e| e.to_string())?;
    let client = AiClient::standard(app.clone()).map_err(|e| e.message)?;
    Ok(ToolDesk::new(client, shared, ConsentFile::at_default_location(&app)))
}

/// A scan reads metadata only. Hash the actual bytes before registering an existing model.
fn check_local_files(feature: &Feature, recipe_id: &str, comfy: &ComfyInstall) -> Result<(), String> {
    let recipe = feature.locals.iter().find(|r| r.workflow == recipe_id).ok_or("That model is not known.")?;
    for file in &recipe.models {
        let matched = comfy.model_dirs.iter().filter(|(kind, _)| kind.eq_ignore_ascii_case(&file.folder)).any(|(_, dir)| {
            let path = dir.join(&file.filename);
            path.is_file() && !path.is_symlink() && verify_file(&path, file, &AtomicBool::new(false), &mut |_| {}).is_ok()
        });
        if !matched {
            return Err(format!("{} did not pass the file check. Check the model files and try again.", file.filename));
        }
    }
    Ok(())
}

struct Job {
    feature: String,
    recipe: String,
    files: Vec<craft_ai_features::ModelFile>,
    comfy: ComfyInstall,
    downloader: Downloader,
    cancel: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    features: Vec<Feature>,
    locate: Locator,
}

impl Job {
    fn run(self) {
        let result = self.download_all();
        // Whatever happened, look again: files that did arrive change what is still needed.
        DesktopFeatures::scan_into(&self.features, &self.locate, &self.shared);
        let mut guard = lock(&self.shared);
        guard.install_running = false;
        guard.install = match result {
            Ok(()) => Install::Done { feature: self.feature, recipe: self.recipe },
            Err((ErrorKind::Cancelled, _)) => Install::Idle,
            Err((_, message)) => Install::Failed { feature: self.feature, recipe: self.recipe, message },
        };
    }

    fn download_all(&self) -> Result<(), (ErrorKind, String)> {
        let of = self.files.len();
        for (index, file) in self.files.iter().enumerate() {
            let dir = self.comfy.models_dir_for(&file.folder).ok_or((ErrorKind::InvalidRequest, "The model folder name is not valid.".to_owned()))?;
            let shared = Arc::clone(&self.shared);
            let (feature, recipe, name) = (self.feature.clone(), self.recipe.clone(), file.filename.clone());
            self.downloader
                .fetch(file, &dir, None, &self.cancel, &mut |p| {
                    lock(&shared).install = Install::Running {
                        feature: feature.clone(),
                        recipe: recipe.clone(),
                        file: name.clone(),
                        verifying: p.phase == Phase::Verifying,
                        done: p.done,
                        total: p.total,
                        step: index + 1,
                        of,
                    };
                })
                .map_err(|e| (e.kind, e.message))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
