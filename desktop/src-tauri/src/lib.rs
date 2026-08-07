//! Tauri shell for ulnclaw: owns the window and manages the `ulnclaw
//! gateway` child process. All agent traffic goes through the gateway's
//! HTTP API from the webview (see ../src/gateway.ts).

use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State};

/// Handle of the managed gateway child process.
struct GatewayProcess(Mutex<Option<u32>>);

/// Locate the `ulnclaw` binary: PATH first, then common install spots.
#[tauri::command]
fn find_ulnclaw_binary() -> Option<String> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = std::path::Path::new(dir).join("ulnclaw");
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Some(home) = home {
        for rel in [".local/bin/ulnclaw", "bin/ulnclaw", ".cargo/bin/ulnclaw"] {
            let candidate = home.join(rel);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
    }
    None
}

/// Default gateway port from ~/.ulnclaw/config.toml ([gateway] port),
/// falling back to 8642.
#[tauri::command]
fn default_gateway_port() -> u16 {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Some(home) = home {
        if let Ok(text) = std::fs::read_to_string(home.join(".ulnclaw").join("config.toml")) {
            if let Ok(doc) = text.parse::<toml_min::Value>() {
                if let Some(port) = doc
                    .get("gateway")
                    .and_then(|g| g.get("port"))
                    .and_then(|p| p.as_integer())
                {
                    return port as u16;
                }
            }
        }
    }
    8642
}

/// Spawn `ulnclaw gateway --port <port>`; returns the child pid.
#[tauri::command]
fn spawn_gateway(state: State<'_, GatewayProcess>, binary: String, port: u16) -> Result<u32, String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(pid) = *guard {
        return Ok(pid); // already managed
    }
    // ULNCLAW_DESKTOP=1 registers the desktop affordance tools and arms
    // the desktop bridge in the child gateway (P231): events stream back
    // over /api/desktop/events and read_terminal round-trips through
    // /api/desktop/read-response.
    let child = std::process::Command::new(&binary)
        .args(["gateway", "--port", &port.to_string()])
        .env("ULNCLAW_DESKTOP", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {binary}: {e}"))?;
    let pid = child.id();
    std::mem::forget(child); // keep it running; stop_gateway reaps by pid
    *guard = Some(pid);
    Ok(pid)
}

/// Stop a managed gateway by pid (SIGTERM on unix).
#[tauri::command]
fn stop_gateway(state: State<'_, GatewayProcess>, pid: u32) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        // SAFETY: kill(2) with a pid we spawned; SIGTERM = 15.
        unsafe { kill(pid as i32, 15) };
    }
    #[cfg(not(unix))]
    {
        let _ = pid; // best-effort: the process exits with the app
    }
    *guard = None;
    Ok(())
}

/// Minimal TOML reader for the port lookup (avoids a toml crate dep in
/// the shell crate).
mod toml_min {
    use std::collections::HashMap;

    #[derive(Debug)]
    pub enum Value {
        Table(HashMap<String, Value>),
        Integer(i64),
        Str(String),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Table(map) => map.get(key),
                _ => None,
            }
        }
        pub fn as_integer(&self) -> Option<i64> {
            match self {
                Value::Integer(v) => Some(*v),
                _ => None,
            }
        }
    }

    /// Parse the subset of TOML we need: `[section]` headers and
    /// `key = value` scalar lines.
    pub fn parse(text: &str) -> Option<Value> {
        let mut root: HashMap<String, Value> = HashMap::new();
        let mut section: Option<String> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    section = Some(name.trim().to_string());
                    continue;
                }
            }
            if let Some((key, value)) = trimmed.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim();
                let parsed = if let Ok(n) = value.parse::<i64>() {
                    Value::Integer(n)
                } else {
                    Value::Str(value.trim_matches('"').to_string())
                };
                match &section {
                    Some(name) => {
                        let entry = root
                            .entry(name.clone())
                            .or_insert_with(|| Value::Table(HashMap::new()));
                        if let Value::Table(map) = entry {
                            map.insert(key, parsed);
                        }
                    }
                    None => {
                        root.insert(key, parsed);
                    }
                }
            }
        }
        Some(Value::Table(root))
    }

    // Convenience for the caller's `text.parse::<Value>()` style.
    impl std::str::FromStr for Value {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            parse(s).ok_or(())
        }
    }
}

/// Show the main window (tray menu + left-click handler share this).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Build the system-tray icon: Show/Quit menu, left-click restores the
/// window. Failure is non-fatal — the app keeps running windowed (e.g.
/// environments without a status-notifier implementation).
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show ulnclaw", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("ulnclaw")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app.handle())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(GatewayProcess(Mutex::new(None)))
        .setup(|app| {
            if let Err(err) = setup_tray(app) {
                eprintln!("ulnclaw desktop: tray unavailable, continuing windowed: {err}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            find_ulnclaw_binary,
            default_gateway_port,
            spawn_gateway,
            stop_gateway
        ])
        .run(tauri::generate_context!())
        .expect("error while running ulnclaw desktop");
}
