use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use craft_ai_features::{AppStatus, AppUse, Licence, LocalRecipe, ModelFile, Source};
use craft_ai_native::secrets::MemoryStore;
use sha2::{Digest, Sha256};

use super::*;

fn body() -> Vec<u8> {
    (0..700_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect()
}

/// Serves `data` to every request, in 50 KB pieces with a pause between them when `slow`.
fn serve(data: Vec<u8>, status: u16, slow: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let data = data.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                for chunk in data.chunks(50_000) {
                    if stream.write_all(chunk).is_err() {
                        return;
                    }
                    if slow {
                        std::thread::sleep(Duration::from_millis(60));
                    }
                }
            });
        }
    });
    format!("http://127.0.0.1:{port}/m")
}

fn feature(data: &[u8], url: &str, licence: bool) -> Feature {
    let digest: String = Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
    Feature {
        id: "t".into(),
        title: "Test".into(),
        summary: "Test".into(),
        capability: craft_ai_model::CapabilityId::parse("image.segment").unwrap(),
        inputs: vec![],
        uses: vec![AppUse { app: "A".into(), tool: "B".into(), status: AppStatus::Proposed }],
        locals: vec![LocalRecipe {
            label: "Test model".into(),
            usable: true,
            workflow: "w".into(),
            models: vec![ModelFile {
                folder: "sam3".into(),
                filename: "m.safetensors".into(),
                size_bytes: data.len() as u64,
                sha256: digest,
                source: Source::Url { url: url.into() },
                licence: Licence { name: "L".into(), url: None, accept_required: licence, note: None },
            }],
            packs: vec![],
            min_vram_mb: None,
            native: Vec::new(),
            components: vec![],
        }],
        cloud: vec![],
    }
}

fn comfy(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::create_dir_all(dir.join("custom_nodes")).unwrap();
    dir.to_owned()
}

/// Looks for ComfyUI afresh every time, as the real locator does, so a folder a download creates
/// is seen by the scan that follows it.
fn port(feature: Feature, install: Option<std::path::PathBuf>) -> DesktopFeatures {
    let config = DownloadConfig { allow_loopback: true, retry_delay: Duration::ZERO, ..DownloadConfig::default() };
    DesktopFeatures::with(
        vec![feature],
        Arc::new(move || install.as_deref().and_then(ComfyInstall::at)),
        config,
        Tools::new(Err("no store in this test".into())),
    )
}

fn settled(port: &mut DesktopFeatures) -> Snapshot {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let snap = port.snapshot();
        if !snap.scanning && !snap.plans.is_empty() && !lock(&port.shared).install_running {
            return snap;
        }
        assert!(Instant::now() < end, "timed out: {snap:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn state(snap: &Snapshot) -> State {
    snap.plans[0].state.clone()
}

#[test]
fn a_scan_turns_the_install_into_a_plan() {
    let data = body();
    let dir = tempfile::tempdir().unwrap();
    let mut p = port(feature(&data, "http://127.0.0.1:1/m", false), Some(comfy(dir.path())));
    p.rescan();
    p.rescan(); // a second request while one runs is harmless
    let snap = settled(&mut p);
    assert_eq!(state(&snap), State::NeedsInstall);
    assert_eq!(snap.plans[0].download_bytes, data.len() as u64);
}

#[test]
fn scan_keeps_separate_plans_for_each_model() {
    let data = body();
    let dir = tempfile::tempdir().unwrap();
    let mut first = feature(&data, "http://127.0.0.1:1/first", false);
    let mut second = first.locals[0].clone();
    second.workflow = "w2".into();
    second.models[0].filename = "other.safetensors".into();
    first.locals.push(second);
    let mut p = port(first, Some(comfy(dir.path())));
    p.rescan();
    let snap = settled(&mut p);
    assert_eq!(snap.plans.iter().map(|plan| plan.recipe.as_str()).collect::<Vec<_>>(), ["w", "w2"]);
    assert_eq!(snap.plans[1].missing_files[0].filename, "other.safetensors");
}

#[test]
fn installing_second_model_downloads_only_its_own_file() {
    let data = body();
    let url = serve(data.clone(), 200, false);
    let dir = tempfile::tempdir().unwrap();
    let mut feature = feature(&data, "http://127.0.0.1:1/first", false);
    let mut second = feature.locals[0].clone();
    second.workflow = "w2".into();
    second.models[0].filename = "other.safetensors".into();
    second.models[0].source = Source::Url { url };
    feature.locals.push(second);
    let mut port = port(feature, Some(comfy(dir.path())));
    port.rescan();
    settled(&mut port);
    port.install_recipe("t", "w2", true).unwrap();
    let result = settled(&mut port);
    assert_eq!(result.plans[1].state, State::Ready);
    assert_eq!(result.plans[0].state, State::NeedsInstall);
    assert_eq!(std::fs::read(dir.path().join("models/sam3/other.safetensors")).unwrap(), data);
    assert!(!dir.path().join("models/sam3/m.safetensors").exists());
}

#[test]
fn without_comfyui_there_is_nothing_to_install_into() {
    let data = body();
    let mut p = port(feature(&data, "http://127.0.0.1:1/m", false), None);
    p.rescan();
    assert_eq!(state(&settled(&mut p)), State::NoRuntime);
    assert!(p.install("t", true).unwrap_err().contains("nothing to download"));
}

#[test]
fn a_licence_must_be_accepted_and_the_shell_checks_it_itself() {
    let data = body();
    let dir = tempfile::tempdir().unwrap();
    let mut p = port(feature(&data, "http://127.0.0.1:1/m", true), Some(comfy(dir.path())));
    p.rescan();
    settled(&mut p);
    assert!(p.install("t", false).unwrap_err().contains("licence"));
    assert!(p.install("nope", true).is_err());
    assert!(dir.path().join("models").read_dir().unwrap().next().is_none(), "nothing was started");
}

#[test]
fn a_download_lands_in_the_models_folder_and_the_tool_becomes_ready() {
    let data = body();
    let url = serve(data.clone(), 200, false);
    let dir = tempfile::tempdir().unwrap();
    let mut p = port(feature(&data, &url, true), Some(comfy(dir.path())));
    p.rescan();
    settled(&mut p);
    p.install("t", true).unwrap();
    let snap = settled(&mut p);
    assert_eq!(snap.install, Install::Done { feature: "t".into(), recipe: "w".into() });
    assert_eq!(state(&snap), State::Ready);
    assert_eq!(std::fs::read(dir.path().join("models/sam3/m.safetensors")).unwrap(), data);
}

#[test]
fn a_failure_is_reported_and_leaves_the_tool_as_it_was() {
    let data = body();
    let url = serve(b"gone".to_vec(), 404, false);
    let dir = tempfile::tempdir().unwrap();
    let mut p = port(feature(&data, &url, false), Some(comfy(dir.path())));
    p.rescan();
    settled(&mut p);
    p.install("t", false).unwrap();
    let snap = settled(&mut p);
    assert!(matches!(&snap.install, Install::Failed { message, .. } if message.contains("no longer")), "{:?}", snap.install);
    assert_eq!(state(&snap), State::NeedsInstall);
    assert!(!dir.path().join("models/sam3/m.safetensors").exists());
}

#[test]
fn cancelling_stops_the_download_and_a_second_start_is_refused_while_it_runs() {
    let data = body();
    let url = serve(data.clone(), 200, true);
    let dir = tempfile::tempdir().unwrap();
    let mut p = port(feature(&data, &url, false), Some(comfy(dir.path())));
    p.rescan();
    settled(&mut p);
    p.install("t", false).unwrap();
    assert!(p.install("t", false).unwrap_err().contains("already running"));
    p.cancel_install();
    let snap = settled(&mut p);
    assert_eq!(snap.install, Install::Idle);
    assert_eq!(state(&snap), State::NeedsInstall);
    assert!(!dir.path().join("models/sam3/m.safetensors").exists());
}

#[test]
fn an_existing_model_is_checked_before_a_suite_binding_is_written() {
    let data = body();
    let dir = tempfile::tempdir().unwrap();
    let base = comfy(dir.path());
    std::fs::create_dir_all(base.join("models/sam3")).unwrap();
    std::fs::write(base.join("models/sam3/m.safetensors"), &data).unwrap();
    let store = SharedStore::new(dir.path().join("connections.json"));
    let client = AiClient::new(ToolDesk::app().unwrap(), store.clone(), Arc::new(MemoryStore::new()));
    let desk = ToolDesk::new(client, store.clone(), None);
    let tiny = feature(&data, "http://127.0.0.1:1/m", false);
    let mut feature = craft_ai_features::builtin().into_iter().find(|f| f.id == "select-subject").unwrap();
    feature.locals.truncate(1);
    feature.locals[0].models = tiny.locals[0].models.clone();
    feature.locals[0].packs.clear();
    let config = DownloadConfig::default();
    let mut port = DesktopFeatures::with(vec![feature], Arc::new(move || ComfyInstall::at(&base)), config, Tools::new(Ok(desk)));
    port.rescan();
    assert_eq!(state(&settled(&mut port)), State::Ready);
    assert!(store.load().unwrap().defaults.is_empty());
    port.use_local("select-subject").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let artcraft_launcher_ui_egui::features::LocalUse::Enabled { feature, recipe, bound } = port.snapshot().local_use {
            assert_eq!(recipe, "sam3-segment");
            assert_eq!(feature, "select-subject");
            assert!(bound);
            break;
        }
        assert!(Instant::now() < end, "model check did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let saved = store.load().unwrap();
    assert_eq!(saved.defaults.len(), 1);
    assert_eq!(saved.connections[0].provider, "comfyui");
    assert_eq!(saved.connections[0].endpoint.as_deref(), Some("http://127.0.0.1:8000"));
    assert_eq!(saved.defaults[0].model.as_str(), "sam3-segment");
}

#[test]
fn checking_second_model_registers_only_the_named_recipe() {
    let data = body();
    let dir = tempfile::tempdir().unwrap();
    let base = comfy(dir.path());
    std::fs::create_dir_all(base.join("models/sam3")).unwrap();
    std::fs::write(base.join("models/sam3/m2.safetensors"), &data).unwrap();
    let store = SharedStore::new(dir.path().join("connections.json"));
    let client = AiClient::new(ToolDesk::app().unwrap(), store.clone(), Arc::new(MemoryStore::new()));
    let desk = ToolDesk::new(client, store.clone(), None);
    let mut subject = craft_ai_features::builtin().into_iter().find(|f| f.id == "select-subject").unwrap();
    subject.locals.truncate(2);
    subject.locals[0].models = feature(&data, "http://127.0.0.1:1/first", false).locals[0].models.clone();
    subject.locals[0].packs.clear();
    subject.locals[1].models = feature(&data, "http://127.0.0.1:1/second", false).locals[0].models.clone();
    subject.locals[1].models[0].filename = "m2.safetensors".into();
    subject.locals[1].models[0].folder = "sam3".into();
    subject.locals[1].packs.clear();
    subject.locals[1].usable = true;
    let mut port = DesktopFeatures::with(vec![subject], Arc::new(move || ComfyInstall::at(&base)), DownloadConfig::default(), Tools::new(Ok(desk)));
    port.rescan();
    let snapshot = settled(&mut port);
    assert_eq!(snapshot.plans[0].state, State::NeedsInstall);
    assert_eq!(snapshot.plans[1].state, State::Ready);
    assert!(store.load().unwrap().defaults.is_empty());
    port.use_local_recipe("select-subject", "sam21-segment").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if matches!(port.snapshot().local_use, artcraft_launcher_ui_egui::features::LocalUse::Enabled { .. }) {
            break;
        }
        assert!(Instant::now() < end, "model check did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let saved = store.load().unwrap();
    assert_eq!(saved.defaults[0].model.as_str(), "sam21-segment");
    assert_eq!(saved.connections[0].models[0].id.as_str(), "sam21-segment");
}

#[test]
fn same_size_corrupt_file_is_not_registered_as_a_model() {
    let data = body();
    let mut corrupted = data.clone();
    corrupted[0] ^= 1;
    let dir = tempfile::tempdir().unwrap();
    let base = comfy(dir.path());
    std::fs::create_dir_all(base.join("models/sam3")).unwrap();
    std::fs::write(base.join("models/sam3/m.safetensors"), corrupted).unwrap();
    let mut port = port(feature(&data, "http://127.0.0.1:1/m", false), Some(base));
    port.rescan();
    assert_eq!(state(&settled(&mut port)), State::Ready, "the metadata scan alone cannot detect this");
    port.use_local("t").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        match port.snapshot().local_use {
            artcraft_launcher_ui_egui::features::LocalUse::Failed { message, .. } => {
                assert!(message.contains("did not pass the file check"), "{message}");
                break;
            }
            _ => assert!(Instant::now() < end, "model check did not finish"),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
