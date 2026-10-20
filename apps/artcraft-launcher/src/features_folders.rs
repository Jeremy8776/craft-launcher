//! Desktop adapter for the shared model-folder list and native folder chooser.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use artcraft_launcher_ui_egui::features::{FolderNotice, FoldersSnapshot};
use craft_ai_client::FolderDesk;
use craft_ai_native::shared::SharedStore;

type FolderPicker = Box<dyn Fn() -> Option<PathBuf>>;
type FolderWork = Box<dyn FnOnce(&FolderDesk) -> bool + Send>;

pub(crate) struct DesktopFolders {
    desk: Option<Arc<FolderDesk>>,
    choose: FolderPicker,
    state: Arc<Mutex<FoldersSnapshot>>,
    worker: Option<JoinHandle<()>>,
}

fn lock(state: &Mutex<FoldersSnapshot>) -> MutexGuard<'_, FoldersSnapshot> {
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl DesktopFolders {
    pub(crate) fn new() -> Self {
        let desk = SharedStore::at_default_location().ok().map(FolderDesk::new);
        let choose: FolderPicker = Box::new(|| {
            let title = artcraft_launcher_ui_egui::i18n::t("Choose a folder for model files").to_string();
            rfd::FileDialog::new().set_title(title).pick_folder()
        });
        Self::with(desk, choose)
    }

    fn with(desk: Option<FolderDesk>, choose: FolderPicker) -> Self {
        Self { desk: desk.map(Arc::new), choose, state: Arc::default(), worker: None }
    }

    pub(crate) fn snapshot(&mut self) -> FoldersSnapshot {
        self.reap_worker();
        lock(&self.state).clone()
    }

    pub(crate) fn refresh(&mut self) {
        self.start(None, Box::new(|_| true));
    }

    pub(crate) fn add_folder(&mut self) {
        if self.desk.is_none() {
            self.fail();
            return;
        }
        let Some(path) = (self.choose)() else { return };
        self.add_selected_folder(path);
    }

    fn add_selected_folder(&mut self, path: PathBuf) {
        self.start(Some(FolderNotice::Added), Box::new(move |desk| desk.add(&path, None).is_ok()));
    }

    pub(crate) fn set_default_folder(&mut self, id: &str) {
        let id = id.to_owned();
        self.start(Some(FolderNotice::DefaultChanged), Box::new(move |desk| desk.set_default(&id).is_ok()));
    }

    pub(crate) fn rescan_folder(&mut self, id: &str) {
        let id = id.to_owned();
        self.start(Some(FolderNotice::Rescanned), Box::new(move |desk| desk.rescan(&id).is_ok()));
    }

    pub(crate) fn remove_folder(&mut self, id: &str) {
        let id = id.to_owned();
        self.start(Some(FolderNotice::Removed), Box::new(move |desk| desk.remove(&id).is_ok()));
    }

    fn start(&mut self, success: Option<FolderNotice>, work: FolderWork) {
        self.reap_worker();
        let Some(desk) = self.desk.as_ref().map(Arc::clone) else {
            self.fail();
            return;
        };
        {
            let mut state = lock(&self.state);
            if state.busy {
                return;
            }
            state.busy = true;
            state.notice = None;
        }
        let state = Arc::clone(&self.state);
        self.worker = Some(std::thread::spawn(move || {
            let operation_ok = work(&desk);
            let listed = desk.list();
            let list_ok = listed.is_ok();
            let mut state = lock(&state);
            if let Ok(folders) = listed {
                state.folders = folders;
            }
            state.loaded = true;
            state.busy = false;
            state.notice = if operation_ok && list_ok { success } else { Some(FolderNotice::Failed) };
        }));
    }

    fn fail(&mut self) {
        let mut state = lock(&self.state);
        state.loaded = true;
        state.busy = false;
        state.notice = Some(FolderNotice::Failed);
    }

    fn reap_worker(&mut self) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) && self.worker.take().is_some_and(|worker| worker.join().is_err()) {
            self.fail();
        }
    }

    #[cfg(test)]
    fn wait_for_worker(&mut self) {
        if self.worker.take().is_some_and(|worker| worker.join().is_err()) {
            self.fail();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::DesktopFolders;
    use craft_ai_client::FolderDesk;
    use craft_ai_native::shared::SharedStore;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn desktop(path: &Path, choose: impl Fn() -> Option<PathBuf> + 'static) -> DesktopFolders {
        let store = SharedStore::new(path.join("shared.json"));
        DesktopFolders::with(Some(FolderDesk::new(store)), Box::new(choose))
    }

    #[test]
    fn adding_a_chosen_folder_makes_it_the_first_shared_download_destination() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("My Models");
        fs::create_dir(&folder).unwrap();
        let selected = folder.clone();
        let mut folders = desktop(temp.path(), move || Some(selected.clone()));

        folders.add_folder();
        folders.wait_for_worker();

        let snapshot = folders.snapshot();
        assert!(snapshot.loaded);
        assert_eq!(snapshot.folders.len(), 1);
        assert_eq!(snapshot.folders[0].path, folder.to_string_lossy());
        assert!(snapshot.folders[0].default);
        assert!(folder.is_dir());
    }

    #[test]
    fn refresh_default_and_rescan_use_the_shared_folder_desk() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("First");
        let second = temp.path().join("Second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let mut folders = desktop(temp.path(), || None);
        folders.add_selected_folder(first.clone());
        folders.wait_for_worker();
        folders.add_selected_folder(second.clone());
        folders.wait_for_worker();

        let mut reloaded = desktop(temp.path(), || None);
        reloaded.refresh();
        reloaded.wait_for_worker();
        let snapshot = reloaded.snapshot();
        assert_eq!(snapshot.folders.len(), 2);
        let first_id = snapshot.folders.iter().find(|folder| folder.path == first.to_string_lossy()).unwrap().id.clone();
        let second_id = snapshot.folders.iter().find(|folder| folder.path == second.to_string_lossy()).unwrap().id.clone();

        reloaded.set_default_folder(&first_id);
        reloaded.wait_for_worker();
        let snapshot = reloaded.snapshot();
        assert!(snapshot.folders.iter().find(|folder| folder.id == first_id).unwrap().default);
        assert!(!snapshot.folders.iter().find(|folder| folder.id == second_id).unwrap().default);
        assert_eq!(snapshot.notice, Some(super::FolderNotice::DefaultChanged));

        reloaded.rescan_folder(&first_id);
        reloaded.wait_for_worker();
        assert_eq!(reloaded.snapshot().notice, Some(super::FolderNotice::Rescanned));
    }

    #[test]
    fn removing_a_chosen_folder_forgets_it_without_deleting_its_files() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("My Models");
        fs::create_dir(&folder).unwrap();
        let marker = folder.join("keep.txt");
        fs::write(&marker, "keep").unwrap();
        let mut folders = desktop(temp.path(), || None);
        folders.add_selected_folder(folder.clone());
        folders.wait_for_worker();
        let id = folders.snapshot().folders[0].id.clone();

        folders.remove_folder(&id);
        folders.wait_for_worker();

        let snapshot = folders.snapshot();
        assert!(snapshot.loaded);
        assert!(snapshot.folders.is_empty());
        assert!(marker.exists());
    }
}
