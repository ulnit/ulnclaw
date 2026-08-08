//! Tauri shell for ulnclaw: owns the window and manages the `ulnclaw
//! gateway` child process. All agent traffic goes through the gateway's
//! HTTP API from the webview (see ../src/gateway.ts).

use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};

/// Handle of the managed gateway child process.
struct GatewayProcess(Mutex<Option<u32>>);

/// P535: last normal (unmaximized) window geometry, persisted to
/// `~/.ulnclaw/desktop-window.json` on close and restored on start.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WindowState {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    #[serde(default)]
    maximized: bool,
}

/// Managed copy of the last normal geometry (updated on resize/move).
struct WindowGeometry(Mutex<Option<WindowState>>);

fn window_state_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|home| home.join(".ulnclaw").join("desktop-window.json"))
}

fn load_window_state() -> Option<WindowState> {
    let path = window_state_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let saved: WindowState = serde_json::from_str(&text).ok()?;
    // Ignore corrupt/implausible values; the config defaults apply.
    if saved.width < 200 || saved.height < 150 {
        return None;
    }
    Some(saved)
}

fn persist_window_state(state: &WindowState) {
    let Some(path) = window_state_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(state) {
        let _ = std::fs::write(path, json);
    }
}

/// P555: single-instance pidfile (`~/.ulnclaw/desktop.pid`) so a second
/// shell launch can't double-launch the gateway child. Mirrors the
/// gateway.pid guard: the record carries the `/proc/<pid>/stat` start
/// time as a PID-reuse token.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DesktopPidRecord {
    pid: u32,
    #[serde(default)]
    started_at: Option<u64>,
}

fn desktop_pidfile_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|home| home.join(".ulnclaw").join("desktop.pid"))
}

/// Field 22 (`starttime`) of `/proc/<pid>/stat`, anchored on the last
/// `)` because comm may contain spaces (same math as gateway_pidfile).
fn desktop_process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 2..)?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// True iff `pid` is a live process — `/proc` state check, zombies dead.
/// Platforms without `/proc` degrade to "not alive", disabling the guard.
fn desktop_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            let close = match stat.rfind(')') {
                Some(close) => close,
                None => return true,
            };
            match stat.get(close + 2..).and_then(|rest| rest.chars().next()) {
                Some(state) => !matches!(state, 'Z' | 'X' | 'x'),
                None => true,
            }
        }
        Err(_) => false,
    }
}

/// P555: claim the single-instance slot. Returns false when another
/// shell recorded in the pidfile is still alive (start token matches).
fn acquire_single_instance() -> bool {
    let Some(path) = desktop_pidfile_path() else {
        return true;
    };
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(record) = serde_json::from_str::<DesktopPidRecord>(data.trim()) {
            if record.pid != 0 && desktop_pid_alive(record.pid) {
                let pid_reused = match (record.started_at, desktop_process_start_time(record.pid)) {
                    (Some(recorded), Some(current)) => recorded != current,
                    _ => false,
                };
                if !pid_reused {
                    eprintln!(
                        "ulnclaw desktop: already running (pid {}); refusing a second instance.",
                        record.pid
                    );
                    return false;
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pid = std::process::id();
    let record = DesktopPidRecord {
        pid,
        started_at: desktop_process_start_time(pid),
    };
    if let Ok(json) = serde_json::to_string(&record) {
        let _ = std::fs::write(&path, json + "\n");
    }
    true
}

/// P555: drop the pidfile on exit — but only when it still names this
/// process, never a record a newer instance wrote over ours.
fn release_single_instance() {
    let Some(path) = desktop_pidfile_path() else {
        return;
    };
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(record) = serde_json::from_str::<DesktopPidRecord>(data.trim()) {
            if record.pid == std::process::id() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

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

/// P540: native app menu — File › New Session (CmdOrCtrl+N) emits an
/// event the webview turns into the new-session flow; Edit carries the
/// standard clipboard roles so the webview keeps them on every OS.
fn setup_app_menu(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let new_session = MenuItem::with_id(app, "new_session", "New Session", true, Some("CmdOrCtrl+N"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, Some("CmdOrCtrl+Q"))?;
    let file = Submenu::with_items(app, "File", true, &[&new_session, &quit])?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let menu = Menu::with_items(app, &[&file, &edit])?;
    app.set_menu(menu)?;
    app.on_menu_event(|handle, event| match event.id.as_ref() {
        "new_session" => {
            let _ = handle.emit("ulnclaw://menu-new-session", ());
        }
        "quit" => handle.exit(0),
        _ => {}
    });
    Ok(())
}

/// Build the system-tray icon: Show/Quit menu, left-click restores the
/// window. Failure is non-fatal — the app keeps running windowed (e.g.
/// environments without a status-notifier implementation).
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show ulnclaw", true, None::<&str>)?;
    let new_session =
        MenuItem::with_id(app, "new_session", "New Session", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &new_session, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("ulnclaw")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            // P573: mirror the File-menu item (P540) — surface the window,
            // then let the webview listener create the session.
            "new_session" => {
                show_main_window(app);
                let _ = app.emit("ulnclaw://menu-new-session", ());
            }
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
    // P555: a second launch while the first is alive exits immediately.
    if !acquire_single_instance() {
        return;
    }
    let app = tauri::Builder::default()
        .manage(GatewayProcess(Mutex::new(None)))
        .manage(WindowGeometry(Mutex::new(None)))
        .setup(|app| {
            if let Err(err) = setup_tray(app) {
                eprintln!("ulnclaw desktop: tray unavailable, continuing windowed: {err}");
            }
            if let Err(err) = setup_app_menu(app) {
                eprintln!("ulnclaw desktop: app menu unavailable: {err}");
            }
            // P535: restore the persisted window geometry.
            if let Some(window) = app.get_webview_window("main") {
                if let Some(saved) = load_window_state() {
                    let _ = window.set_size(tauri::PhysicalSize::new(
                        saved.width,
                        saved.height,
                    ));
                    let _ = window.set_position(tauri::PhysicalPosition::new(
                        saved.x,
                        saved.y,
                    ));
                    if saved.maximized {
                        let _ = window.maximize();
                    }
                    if let Some(geometry) = window.try_state::<WindowGeometry>() {
                        if let Ok(mut guard) = geometry.0.lock() {
                            *guard = Some(WindowState {
                                maximized: false,
                                ..saved
                            });
                        }
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // P535: track the normal geometry live; write it on close.
            if window.label() != "main" {
                return;
            }
            match event {
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) => {
                    let Ok(maximized) = window.is_maximized() else {
                        return;
                    };
                    if maximized {
                        return;
                    }
                    let Ok(size) = window.outer_size() else {
                        return;
                    };
                    let Ok(position) = window.outer_position() else {
                        return;
                    };
                    if let Some(geometry) = window.try_state::<WindowGeometry>() {
                        if let Ok(mut guard) = geometry.0.lock() {
                            *guard = Some(WindowState {
                                width: size.width,
                                height: size.height,
                                x: position.x,
                                y: position.y,
                                maximized: false,
                            });
                        }
                    }
                }
                tauri::WindowEvent::CloseRequested => {
                    let maximized = window.is_maximized().unwrap_or(false);
                    if let Some(geometry) = window.try_state::<WindowGeometry>() {
                        if let Ok(guard) = geometry.0.lock() {
                            if let Some(normal) = guard.as_ref() {
                                let mut saved = normal.clone();
                                saved.maximized = maximized;
                                persist_window_state(&saved);
                            }
                        }
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            find_ulnclaw_binary,
            default_gateway_port,
            spawn_gateway,
            stop_gateway
        ])
        .build(tauri::generate_context!())
        .expect("error while building ulnclaw desktop");
    app.run(|_handle, event| {
        // P555: release the single-instance slot on real exit.
        if let tauri::RunEvent::Exit = event {
            release_single_instance();
        }
    });
}
