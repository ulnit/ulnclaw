//! Tauri shell for ulnclaw: owns the window and manages the `ulnclaw
//! gateway` child process. All agent traffic goes through the gateway's
//! HTTP API from the webview (see ../src/gateway.ts).

use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

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
    /// P778: fullscreen state is restored alongside the geometry.
    #[serde(default)]
    fullscreen: bool,
    /// P782: webview zoom level (1.2-base exponent, 0.0 = actual size).
    #[serde(default)]
    zoom_level: f64,
}

/// Managed copy of the last normal geometry (updated on resize/move).
struct WindowGeometry(Mutex<Option<WindowState>>);

/// P783: quit guard — how many turns the webview reports in flight
/// (0 = safe to exit) and the user's confirmed-quit latch.
struct ActiveWork(Mutex<u32>);
struct QuitConfirmed(Mutex<bool>);

/// P790: the tray icon id, so commands can update its tooltip live.
struct TrayId(Mutex<Option<String>>);

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

/// P777: About info for the Help menu — shell version, detected
/// `ulnclaw` binary and the gateway port the shell targets.
#[tauri::command]
fn desktop_about() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "binary": find_ulnclaw_binary(),
        "port": default_gateway_port(),
    })
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

/// P781: whether the OS launches ulnclaw at login.
#[tauri::command]
fn desktop_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// P781: toggle launch-at-login through the OS autostart facility
/// (LaunchAgent on macOS, autostart .desktop entry on Linux, registry
/// run key on Windows).
#[tauri::command]
fn desktop_autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
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

/// P782: webview zoom (hermes zoom parity). Levels are exponents of a
/// 1.2 base — Chromium's native zoom step; 0.0 is actual size (100%).
const ZOOM_BASE: f64 = 1.2;
const ZOOM_MIN_LEVEL: f64 = -9.0;
const ZOOM_MAX_LEVEL: f64 = 9.0;

/// Managed live zoom level (persisted on close via the window state).
struct ZoomLevel(Mutex<f64>);

fn clamp_zoom_level(level: f64) -> f64 {
    if !level.is_finite() {
        return 0.0;
    }
    level.clamp(ZOOM_MIN_LEVEL, ZOOM_MAX_LEVEL)
}

fn zoom_percent(level: f64) -> i64 {
    (ZOOM_BASE.powf(clamp_zoom_level(level)) * 100.0).round() as i64
}

/// P782: shift the zoom by whole levels (or reset to actual size),
/// apply it to the webview, then tell the page the new percent so it
/// can surface a brief HUD note.
fn adjust_zoom(app: &tauri::AppHandle, delta: f64, reset: bool) {
    let next = {
        let zoom = app.state::<ZoomLevel>();
        let Ok(mut guard) = zoom.0.lock() else {
            return;
        };
        let next = if reset {
            0.0
        } else {
            clamp_zoom_level(*guard + delta)
        };
        *guard = next;
        next
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_zoom(ZOOM_BASE.powf(next));
    }
    let _ = app.emit(
        "ulnclaw://zoom-changed",
        serde_json::json!({ "percent": zoom_percent(next) }),
    );
}

/// P783: quit unless a turn is in flight; otherwise surface the window
/// and ask the page to confirm (it calls `desktop_confirm_quit`).
fn request_quit(app: &tauri::AppHandle) {
    let active = app
        .try_state::<ActiveWork>()
        .and_then(|work| work.0.lock().ok().map(|guard| *guard))
        .unwrap_or(0);
    let confirmed = app
        .try_state::<QuitConfirmed>()
        .and_then(|flag| flag.0.lock().ok().map(|guard| *guard))
        .unwrap_or(false);
    if active == 0 || confirmed {
        app.exit(0);
        return;
    }
    show_main_window(app);
    let _ = app.emit(
        "ulnclaw://quit-blocked",
        serde_json::json!({ "count": active }),
    );
}

/// P783: the webview reports how many turns are in flight (0 = idle).
#[tauri::command]
fn desktop_set_active_work(state: State<'_, ActiveWork>, count: u32) {
    if let Ok(mut guard) = state.0.lock() {
        *guard = count;
    }
}

/// P789: expose the P555 liveness probe to the webview so it can tell
/// a crashed managed gateway from a slow one (zombies count as dead).
#[tauri::command]
fn desktop_gateway_pid_alive(pid: u32) -> bool {
    desktop_pid_alive(pid)
}

/// P790: live tray tooltip — the webview reports gateway health.
#[tauri::command]
fn desktop_set_tray_tooltip(app: tauri::AppHandle, tooltip: String) {
    let id = app
        .try_state::<TrayId>()
        .and_then(|state| state.0.lock().ok().and_then(|guard| guard.clone()));
    if let Some(id) = id {
        if let Some(tray) = app.tray_by_id(id.as_str()) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
}

/// P783: the user confirmed quitting with work in flight — latch and exit.
#[tauri::command]
fn desktop_confirm_quit(app: tauri::AppHandle, confirmed: State<'_, QuitConfirmed>) {
    if let Ok(mut guard) = confirmed.0.lock() {
        *guard = true;
    }
    app.exit(0);
}

/// P785: deep links that cold-launched this instance — the page flushes
/// them once its listener is mounted (live arrivals stream over the
/// `ulnclaw://deep-link` event).
#[tauri::command]
fn desktop_deep_links_pending(
    deep_link: State<'_, tauri_plugin_deep_link::DeepLink<tauri::Wry>>,
) -> Vec<String> {
    deep_link
        .get_current()
        .ok()
        .flatten()
        .map(|urls| urls.into_iter().map(|url| url.to_string()).collect())
        .unwrap_or_default()
}

/// P787: open a session in its own window — a full shell seeded with
/// `?session=<id>` (hermes session-windows parity, lean). Refocusing an
/// already-open window for the same session wins over a duplicate.
#[tauri::command]
fn desktop_open_session_window(app: tauri::AppHandle, session_id: String) -> Result<(), String> {
    let slug: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    if slug.is_empty() {
        return Err("empty session id".to_string());
    }
    let label = format!("session-{slug}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    let url = tauri::WebviewUrl::App(
        format!("index.html?session={}", slug_of(&session_id)).into(),
    );
    tauri::WebviewWindowBuilder::new(&app, label, url)
        .title("ulnclaw")
        .inner_size(1020.0, 700.0)
        .min_inner_size(640.0, 420.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// P787: url-encode the session id for the popout query string.
fn slug_of(session_id: &str) -> String {
    session_id
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// P780: the global quick-entry accelerator — hermes' default
/// (`CommandOrControl+Shift+Space`), mapped per platform by the plugin.
fn quick_entry_shortcut() -> Shortcut {
    "CmdOrCtrl+Shift+Space"
        .parse()
        .expect("static quick-entry shortcut parses")
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
    // P777: View menu — reload + fullscreen toggle mirror the webview
    // shortcuts so the native affordances work before the page loads.
    let reload = MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?;
    let fullscreen = MenuItem::with_id(
        app,
        "toggle_fullscreen",
        "Toggle Full Screen",
        true,
        Some("F11"),
    )?;
    // P782: Chromium-style zoom steps; Actual Size returns to 100%.
    let zoom_in = MenuItem::with_id(app, "zoom_in", "Zoom In", true, Some("CmdOrCtrl+Equal"))?;
    let zoom_out = MenuItem::with_id(app, "zoom_out", "Zoom Out", true, Some("CmdOrCtrl+Minus"))?;
    let zoom_actual = MenuItem::with_id(app, "zoom_actual", "Actual Size", true, Some("CmdOrCtrl+0"))?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[&reload, &fullscreen, &zoom_in, &zoom_out, &zoom_actual],
    )?;
    // P777: Help › About emits the shell version/binary/port payload;
    // the webview renders it as a notification.
    let about = MenuItem::with_id(app, "about", "About ulnclaw", true, None::<&str>)?;
    let help = Submenu::with_items(app, "Help", true, &[&about])?;
    let menu = Menu::with_items(app, &[&file, &edit, &view, &help])?;
    app.set_menu(menu)?;
    app.on_menu_event(|handle, event| match event.id.as_ref() {
        "new_session" => {
            let _ = handle.emit("ulnclaw://menu-new-session", ());
        }
        "reload" => {
            let _ = handle.emit("ulnclaw://menu-reload", ());
        }
        "toggle_fullscreen" => {
            let _ = handle.emit("ulnclaw://menu-toggle-fullscreen", ());
        }
        // P782: zoom steps funnel through one clamped level.
        "zoom_in" => adjust_zoom(handle, 1.0, false),
        "zoom_out" => adjust_zoom(handle, -1.0, false),
        "zoom_actual" => adjust_zoom(handle, 0.0, true),
        "about" => {
            let _ = handle.emit("ulnclaw://about", desktop_about());
        }
        // P783: route through the quit guard.
        "quit" => request_quit(handle),
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
    // P779: restart the managed gateway child via the webview flow.
    let restart_gateway = MenuItem::with_id(
        app,
        "restart_gateway",
        "Restart Gateway",
        true,
        None::<&str>,
    )?;
    // P788: the gateway also serves the browser dashboard — open it in
    // the default browser.
    let open_dashboard = MenuItem::with_id(
        app,
        "open_dashboard",
        "Open Dashboard",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show, &new_session, &restart_gateway, &open_dashboard, &quit],
    )?;

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
            // P779: the webview owns the stop/respawn/health-wait
            // restart flow (restartGateway, P343) — the tray only
            // triggers it.
            "restart_gateway" => {
                let _ = app.emit("ulnclaw://restart-gateway", ());
            }
            // P788: gateway dashboard in the default browser.
            "open_dashboard" => {
                let url = format!("http://127.0.0.1:{}", default_gateway_port());
                if let Err(err) = app.opener().open_url(url, None::<&str>) {
                    eprintln!("ulnclaw desktop: could not open dashboard: {err}");
                }
            }
            // P783: route through the quit guard.
            "quit" => request_quit(app),
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
    let icon = builder.build(app.handle())?;
    // P790: remember the tray id for live tooltip updates.
    if let Some(tray_id) = app.try_state::<TrayId>() {
        if let Ok(mut guard) = tray_id.0.lock() {
            *guard = Some(icon.id().as_ref().to_string());
        }
    }
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
        .manage(ZoomLevel(Mutex::new(0.0)))
        .manage(ActiveWork(Mutex::new(0)))
        .manage(QuitConfirmed(Mutex::new(false)))
        .manage(TrayId(Mutex::new(None)))
        .plugin(
            // P780: global quick-entry shortcut — summon the window and
            // open the quick-entry overlay from anywhere.
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state == ShortcutState::Pressed
                        && *shortcut == quick_entry_shortcut()
                    {
                        show_main_window(app);
                        let _ = app.emit("ulnclaw://quick-entry", ());
                    }
                })
                .build(),
        )
        // P781: launch-at-login support (settings-dialog toggle).
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        // P784: a second launch surfaces the running window instead of
        // vanishing silently (hermes second-instance handoff parity).
        // The P555 pidfile guard stays as the hard single-instance lock.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        // P785: ulnclaw:// deep-link routing (hermes deep-link parity).
        .plugin(tauri_plugin_deep_link::init())
        // P788: default-browser opening for the dashboard tray item.
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Err(err) = setup_tray(app) {
                eprintln!("ulnclaw desktop: tray unavailable, continuing windowed: {err}");
            }
            if let Err(err) = setup_app_menu(app) {
                eprintln!("ulnclaw desktop: app menu unavailable: {err}");
            }
            // P780: register the accelerator; failure is non-fatal (e.g.
            // another app already owns the grab).
            if let Err(err) = app.global_shortcut().register(quick_entry_shortcut()) {
                eprintln!("ulnclaw desktop: global quick-entry shortcut unavailable: {err}");
            }
            // P785: register the ulnclaw:// scheme, then route live
            // arrivals to the page. Cold-launch links are flushed by the
            // page through desktop_deep_links_pending.
            {
                let deep_link = app.deep_link();
                if let Err(err) = deep_link.register_all() {
                    eprintln!("ulnclaw desktop: deep-link registration unavailable: {err}");
                }
                let handle = app.handle().clone();
                deep_link.on_open_url(move |event| {
                    let urls: Vec<String> =
                        event.urls().into_iter().map(|url| url.to_string()).collect();
                    if !urls.is_empty() {
                        show_main_window(&handle);
                        let _ = handle.emit("ulnclaw://deep-link", urls);
                    }
                });
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
                    if saved.fullscreen {
                        let _ = window.set_fullscreen(true);
                    }
                    // P782: restore the persisted zoom level.
                    let zoom_level = clamp_zoom_level(saved.zoom_level);
                    if let Some(zoom) = window.try_state::<ZoomLevel>() {
                        if let Ok(mut guard) = zoom.0.lock() {
                            *guard = zoom_level;
                        }
                    }
                    let _ = window.set_zoom(ZOOM_BASE.powf(zoom_level));
                    if let Some(geometry) = window.try_state::<WindowGeometry>() {
                        if let Ok(mut guard) = geometry.0.lock() {
                            *guard = Some(WindowState {
                                maximized: false,
                                fullscreen: false,
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
                    // P778: keep stamping the last normal geometry while
                    // fullscreen, but remember the fullscreen state itself.
                    let fullscreen = window.is_fullscreen().unwrap_or(false);
                    if maximized {
                        if fullscreen {
                            if let Some(geometry) = window.try_state::<WindowGeometry>() {
                                if let Ok(mut guard) = geometry.0.lock() {
                                    if let Some(state) = guard.as_mut() {
                                        state.fullscreen = true;
                                    }
                                }
                            }
                        }
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
                            let zoom_level = guard
                                .as_ref()
                                .map(|state| state.zoom_level)
                                .unwrap_or(0.0);
                            *guard = Some(WindowState {
                                width: size.width,
                                height: size.height,
                                x: position.x,
                                y: position.y,
                                maximized: false,
                                fullscreen,
                                zoom_level,
                            });
                        }
                    }
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // P783: block the close while a turn is in flight —
                    // the page confirms, then desktop_confirm_quit exits.
                    let active = window
                        .try_state::<ActiveWork>()
                        .and_then(|work| work.0.lock().ok().map(|guard| *guard))
                        .unwrap_or(0);
                    let confirmed = window
                        .try_state::<QuitConfirmed>()
                        .and_then(|flag| flag.0.lock().ok().map(|guard| *guard))
                        .unwrap_or(false);
                    if active > 0 && !confirmed {
                        show_main_window(window.app_handle());
                        let _ = window.emit(
                            "ulnclaw://quit-blocked",
                            serde_json::json!({ "count": active }),
                        );
                        api.prevent_close();
                        return;
                    }
                    let maximized = window.is_maximized().unwrap_or(false);
                    let fullscreen = window.is_fullscreen().unwrap_or(false);
                    // P782: fold the live zoom into the persisted state.
                    let zoom_level = window
                        .try_state::<ZoomLevel>()
                        .and_then(|zoom| zoom.0.lock().ok().map(|guard| *guard))
                        .unwrap_or(0.0);
                    if let Some(geometry) = window.try_state::<WindowGeometry>() {
                        if let Ok(guard) = geometry.0.lock() {
                            if let Some(normal) = guard.as_ref() {
                                let mut saved = normal.clone();
                                saved.maximized = maximized;
                                saved.fullscreen = fullscreen;
                                saved.zoom_level = zoom_level;
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
            stop_gateway,
            desktop_about,
            desktop_autostart_enabled,
            desktop_autostart_set,
            desktop_set_active_work,
            desktop_confirm_quit,
            desktop_deep_links_pending,
            desktop_open_session_window,
            desktop_gateway_pid_alive,
            desktop_set_tray_tooltip
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
