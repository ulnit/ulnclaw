// Optional Tauri desktop bridge for the React shell. The same UI runs in
// a plain browser tab against a gateway (dev mode), so every capability
// is guarded: `loadBridge()` resolves null outside the webview.

export interface BootDiagnostics {
  binary: string | null;
  bundled: string | null;
  port: number;
  log_path: string | null;
  child_pid: number | null;
  child_alive: boolean;
  log_tail: string;
  spawnError?: string | null;
}

export interface DesktopBridge {
  findBinary(): Promise<string | null>;
  spawnGateway(binary: string, port: number): Promise<number>;
  stopGateway(pid: number): Promise<void>;
  defaultPort(): Promise<number>;
  autostartEnabled(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  setActiveWork(count: number): Promise<void>;
  confirmQuit(): Promise<void>;
  deepLinksPending(): Promise<string[]>;
  pidAlive(pid: number): Promise<boolean>;
  setTrayTooltip(tooltip: string): Promise<void>;
  setCloseToTray(enabled: boolean): Promise<void>;
  setWindowTitle(title: string): Promise<void>;
  setTrayHealth(up: boolean): Promise<void>;
  gatewayLogTail(lines: number): Promise<string[]>;
  openPath(path: string): Promise<void>;
  notifyNative(title: string, body: string): Promise<void>;
  diagnostics(): Promise<BootDiagnostics | null>;
}

export async function loadBridge(): Promise<DesktopBridge | null> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return {
      findBinary: () => invoke<string | null>("find_ulnclaw_binary"),
      spawnGateway: (binary, port) => invoke<number>("spawn_gateway", { binary, port }),
      stopGateway: (pid) => invoke<void>("stop_gateway", { pid }),
      defaultPort: () => invoke<number>("default_gateway_port"),
      autostartEnabled: () => invoke<boolean>("desktop_autostart_enabled"),
      setAutostart: (enabled) => invoke<void>("desktop_autostart_set", { enabled }),
      setActiveWork: (count) => invoke<void>("desktop_set_active_work", { count }),
      confirmQuit: () => invoke<void>("desktop_confirm_quit"),
      deepLinksPending: () => invoke<string[]>("desktop_deep_links_pending"),
      pidAlive: (pid) => invoke<boolean>("desktop_gateway_pid_alive", { pid }),
      setTrayTooltip: (tooltip) => invoke<void>("desktop_set_tray_tooltip", { tooltip }),
      setCloseToTray: (enabled) => invoke<void>("desktop_set_close_to_tray", { enabled }),
      setWindowTitle: (title) => invoke<void>("desktop_set_window_title", { title }),
      setTrayHealth: (up) => invoke<void>("desktop_set_tray_health", { up }),
      gatewayLogTail: (lines) => invoke<string[]>("desktop_gateway_log_tail", { lines }),
      openPath: (path) => invoke<void>("desktop_open_path", { path }),
      notifyNative: (title, body) => invoke<void>("desktop_notify", { title, body }),
      diagnostics: () => invoke<BootDiagnostics>("desktop_gateway_diagnostics"),
    };
  } catch {
    return null;
  }
}

/** Subscribe to the Rust shell's menu/tray events; returns an unsubscriber. */
export async function listenBridgeEvents(
  handlers: Record<string, (payload: unknown) => void>,
): Promise<() => void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const unsubs = await Promise.all(
      Object.entries(handlers).map(([event, fn]) =>
        listen(event, (e) => fn((e as { payload: unknown }).payload)),
      ),
    );
    return () => unsubs.forEach((u) => u());
  } catch {
    return () => undefined;
  }
}
