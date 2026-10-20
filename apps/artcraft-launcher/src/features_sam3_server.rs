//! Starts the existing ComfyUI Desktop installation headlessly for the launcher sample.
//! The selected address must be plain IPv4 loopback; never launch against somebody else's server.
#[cfg(any(test, target_os = "windows"))]
use std::ffi::OsString;
use std::net::{SocketAddrV4, TcpStream};
#[cfg(any(test, target_os = "windows"))]
use std::path::Path;
#[cfg(target_os = "windows")]
use std::path::PathBuf;
use std::time::Duration;
#[cfg(target_os = "windows")]
use std::time::Instant;

use craft_ai_comfyui::Sam3Session;

#[cfg(target_os = "windows")]
const HEADLESS_FLAGS: u32 = 0x0800_0000 | 0x0000_0200; // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP

fn loopback_port(url: &str) -> Option<u16> {
    let port = url.strip_prefix("http://127.0.0.1:")?;
    match port.parse::<u16>() {
        Ok(0) | Err(_) => None,
        Ok(port) => Some(port),
    }
}

pub(super) fn ensure_running(url: &str) -> Result<(), String> {
    let port = loopback_port(url).ok_or("Select subject requires a plain IPv4 loopback ComfyUI endpoint")?;
    let session = Sam3Session::new(url).map_err(|e| e.message)?;
    let address = SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, port);
    if TcpStream::connect_timeout(&address.into(), Duration::from_secs(2)).is_ok() {
        return session.verify_server().map_err(|e| format!("ComfyUI port is in use: {}", e.message));
    }
    #[cfg(not(target_os = "windows"))]
    return Err("Start ComfyUI on the selected address to test Select subject".to_owned());
    #[cfg(target_os = "windows")]
    {
        let (python, program, base, extra) = desktop_paths()?;
        let args = headless_args(&base, &program, &extra, port);
        use std::os::windows::process::CommandExt;
        let mut child = std::process::Command::new(&python)
            .args(args)
            .current_dir(&program)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(HEADLESS_FLAGS)
            .spawn()
            .map_err(|e| format!("Could not start your existing ComfyUI: {e}"))?;
        // Desktop's Python launcher and custom-node discovery can exceed a minute on a cold start.
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            if session.verify_server().is_ok() {
                return Ok(());
            }
            if child.try_wait().map_err(|e| format!("Could not check ComfyUI startup: {e}"))?.is_some() {
                return Err("Your existing ComfyUI closed during startup. Open ComfyUI Desktop to inspect its logs".to_owned());
            }
            std::thread::sleep(Duration::from_millis(350));
        }
        Err("Your existing ComfyUI did not become ready within 180 seconds".to_owned())
    }
}

#[cfg(target_os = "windows")]
fn desktop_paths() -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let install = craft_ai_comfy_env::ComfyInstall::discover().ok_or("The existing ComfyUI Desktop settings were not found")?;
    let roaming = PathBuf::from(std::env::var_os("APPDATA").ok_or("Windows APPDATA is unavailable")?).join("ComfyUI");
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("Windows LOCALAPPDATA is unavailable")?);
    let base = install.base;
    let program = local.join("Programs/ComfyUI/resources/ComfyUI");
    let extra = roaming.join("extra_models_config.yaml");
    let python = base.join(".venv/Scripts/python.exe");
    if !base.join("models/sam3").is_dir()
        || !install.custom_nodes.join("comfyui-sam3").is_dir()
        || !extra.is_file()
        || !python.is_file()
        || !program.join("main.py").is_file()
        || !program.join("web_custom_versions/desktop_app").is_dir()
    {
        return Err("Your existing ComfyUI Desktop or SAM 3 installation is incomplete".to_owned());
    }
    Ok((python, program, base, extra))
}

#[cfg(any(test, target_os = "windows"))]
fn headless_args(base: &Path, program: &Path, extra: &Path, port: u16) -> Vec<OsString> {
    let database = format!("sqlite:///{}", base.join("user/comfyui.db").to_string_lossy().replace('\\', "/"));
    vec![
        program.join("main.py").into_os_string(),
        "--user-directory".into(),
        base.join("user").into_os_string(),
        "--input-directory".into(),
        base.join("input").into_os_string(),
        "--output-directory".into(),
        base.join("output").into_os_string(),
        "--front-end-root".into(),
        program.join("web_custom_versions/desktop_app").into_os_string(),
        "--base-directory".into(),
        base.as_os_str().to_owned(),
        "--database-url".into(),
        database.into(),
        "--extra-model-paths-config".into(),
        extra.as_os_str().to_owned(),
        "--disable-auto-launch".into(),
        "--listen".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string().into(),
        "--enable-manager".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_ipv4_loopback_is_started() {
        assert_eq!(loopback_port("http://127.0.0.1:9100"), Some(9100));
        for other in ["http://localhost:9100", "http://[::1]:9100", "http://127.0.0.1:9100/path", "http://127.0.0.1:0"] {
            assert_eq!(loopback_port(other), None, "{other}");
        }
    }

    #[test]
    fn headless_start_uses_the_existing_desktop_resources_and_selected_port() {
        let args =
            headless_args(Path::new("C:/my ComfyUI"), Path::new("C:/Programs/ComfyUI/resources/ComfyUI"), Path::new("C:/ComfyUI/extra.yaml"), 9100);
        assert!(args.windows(2).any(|pair| pair == ["--listen", "127.0.0.1"]));
        assert!(args.windows(2).any(|pair| pair == ["--port", "9100"]));
        assert!(args.contains(&"--disable-auto-launch".into()));
        assert!(args.windows(2).any(|pair| pair[0] == "--extra-model-paths-config" && pair[1] == "C:/ComfyUI/extra.yaml"));
    }
}
