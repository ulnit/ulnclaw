import { useCallback, useEffect, useRef, useState } from "react";
import { GatewayClient, loadSettings } from "../gateway";
import type { SessionRow } from "../gateway";
import { Sidebar, type ShellView } from "./Sidebar";
import { Button } from "../hermes/button";
import { useT } from "./util";
import { loadBridge, listenBridgeEvents, type BootDiagnostics, type DesktopBridge } from "./bridge";
import { ChatView } from "./ChatView";
import { SessionsView } from "./views/SessionsView";
import { JobsView } from "./views/JobsView";
import { UsageView } from "./views/UsageView";
import { ModelsView } from "./views/ModelsView";
import { SettingsView } from "./views/SettingsView";
import { SkillsView } from "./views/SkillsView";
import { KanbanView } from "./views/KanbanView";
import { ProjectsView } from "./views/ProjectsView";
import { WebhooksView } from "./views/WebhooksView";
import { RunsView } from "./views/RunsView";
import { PluginsView } from "./views/PluginsView";
import { PairingView } from "./views/PairingView";
import { ProfilesView } from "./views/ProfilesView";
import { ConfigView } from "./views/ConfigView";
import { DoctorView } from "./views/DoctorView";
import { Palette } from "./Palette";

export function App() {
  const [client] = useState(() => new GatewayClient(loadSettings()));
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<ShellView>("chat");
  const [online, setOnline] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [bridge, setBridge] = useState<DesktopBridge | null>(null);
  const [diag, setDiag] = useState<BootDiagnostics | null>(null);
  const [logOpen, setLogOpen] = useState(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const managedPid = useRef<number | null>(null);
  const bridgeRef = useRef<DesktopBridge | null>(null);
  const onlineRef = useRef(false);
  const t = useT();

  const refresh = useCallback(async () => {
    try {
      setSessions(await client.listSessions(true, false));
      setOnline(true);
      onlineRef.current = true;
    } catch {
      setOnline(false);
      onlineRef.current = false;
    }
    const b = bridgeRef.current;
    if (b) {
      b.setTrayHealth(onlineRef.current).catch(() => undefined);
      b.setTrayTooltip(`ulnclaw — ${onlineRef.current ? "up" : "down"}`).catch(() => undefined);
    }
  }, [client]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const showGatewayLog = useCallback(async () => {
    const b = bridgeRef.current;
    if (!b) return;
    try {
      setLogLines(await b.gatewayLogTail(200));
      setLogOpen(true);
    } catch {
      /* bridge unavailable */
    }
  }, []);

  const restartGateway = useCallback(async () => {
    const b = bridgeRef.current;
    if (!b) return;
    try {
      if (managedPid.current) await b.stopGateway(managedPid.current).catch(() => undefined);
      const binary = await b.findBinary();
      if (!binary) return;
      const port = await b.defaultPort();
      managedPid.current = await b.spawnGateway(binary, port);
      for (let i = 0; i < 20; i++) {
        await new Promise((r) => setTimeout(r, 500));
        if (await client.health().catch(() => false)) break;
      }
      void refresh();
    } catch {
      /* spawn failed */
    }
  }, [client, refresh]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const b = await loadBridge();
      if (cancelled) return;
      bridgeRef.current = b;
      setBridge(b);
      if (b) {
        const up = await client.health().catch(() => false);
        if (!up) {
          try {
            const binary = await b.findBinary();
            if (binary) {
              const port = await b.defaultPort();
              managedPid.current = await b.spawnGateway(binary, port);
              for (let i = 0; i < 20; i++) {
                await new Promise((r) => setTimeout(r, 500));
                if (await client.health().catch(() => false)) break;
              }
              void refresh();
            }
          } catch {
            /* spawn failed — diagnostics below */
          }
          if (!onlineRef.current) {
            setDiag(await b.diagnostics().catch(() => null));
          }
        }
        unlisten = await listenBridgeEvents({
          "ulnclaw://menu-new-session": () => {
            setActiveId(null);
            setView("chat");
          },
          "ulnclaw://menu-reload": () => location.reload(),
          "ulnclaw://menu-toggle-fullscreen": () => {
            void import("@tauri-apps/api/window")
              .then(async (w) => {
                const win = w.getCurrentWindow();
                await win.setFullscreen(!(await win.isFullscreen()));
              })
              .catch(() => undefined);
          },
          "ulnclaw://restart-gateway": () => void restartGateway(),
          "ulnclaw://gateway-log": () => void showGatewayLog(),
          "ulnclaw://window-shown": () => window.focus(),
        });
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [client, refresh, restartGateway, showGatewayLog]);

  useEffect(() => {
    const onBusy = (e: Event) => {
      const busy = (e as CustomEvent<boolean>).detail;
      bridgeRef.current?.setActiveWork(busy ? 1 : 0).catch(() => undefined);
    };
    window.addEventListener("ulnclaw:busy", onBusy);
    return () => window.removeEventListener("ulnclaw:busy", onBusy);
  }, []);

  const openSession = (id: string) => {
    setActiveId(id);
    setView("chat");
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="grid h-full min-h-0 grid-cols-[14.8125rem_1fr]">
      <Sidebar
        sessions={sessions}
        activeId={activeId}
        online={online}
        view={view}
        onView={setView}
        onSelect={openSession}
        onRefresh={refresh}
      />
      <div className="relative min-h-0">
        {!online && (
          <div className="absolute left-1/2 top-3 z-40 flex -translate-x-1/2 items-center gap-2.5 rounded-lg border border-(--border) bg-(--seed-card) px-3.5 py-2 shadow-[var(--shadow-composer)]">
            <span className="size-2 shrink-0 rounded-full bg-(--bad)" />
            <span className="text-xs text-(--dim)">{t.boot.unreachable}</span>
            <Button variant="ghost" size="micro" onClick={() => void refresh()}>
              {t.boot.retry}
            </Button>
            {bridge && (
              <details className="relative">
                <summary className="cursor-pointer list-none text-xs font-medium text-(--accent)">
                  {t.boot.diagTitle}
                </summary>
                <div className="absolute right-0 top-full z-50 mt-1 w-[26rem] rounded-lg border border-(--border) bg-(--seed-card) p-3 text-[11px] leading-5 text-(--dim) shadow-[var(--shadow-composer)]">
                  {diag === null ? (
                    <p>{t.boot.unreachableDetail}</p>
                  ) : (
                    <div className="flex flex-col gap-1">
                      <p>
                        {t.boot.diagBinary}: {diag.binary ?? diag.bundled ?? t.boot.diagNoBinary}
                      </p>
                      <p>
                        {t.boot.diagPort}: {diag.port} · {t.boot.diagPid}:{" "}
                        {diag.child_pid ?? "—"} ({diag.child_alive ? t.boot.diagAlive : t.boot.diagDead})
                      </p>
                      {diag.spawnError && <p className="text-(--bad)">{diag.spawnError}</p>}
                      {diag.log_tail ? (
                        <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-(--hover) p-2 font-mono text-[10px]">
                          {diag.log_tail}
                        </pre>
                      ) : (
                        <p>{t.boot.diagNoLog}</p>
                      )}
                    </div>
                  )}
                </div>
              </details>
            )}
          </div>
        )}
      {view === "chat" && (
        <ChatView
          client={client}
          sessionId={activeId}
          onCreated={(id) => {
            setActiveId(id);
            void refresh();
          }}
        />
      )}
      {view === "sessions" && (
        <div className="min-h-0 overflow-y-auto">
          <SessionsView client={client} onOpen={openSession} />
        </div>
      )}
      {view === "jobs" && (
        <div className="min-h-0 overflow-y-auto">
          <JobsView client={client} />
        </div>
      )}
      {view === "usage" && (
        <div className="min-h-0 overflow-y-auto">
          <UsageView client={client} />
        </div>
      )}
      {view === "models" && (
        <div className="min-h-0 overflow-y-auto">
          <ModelsView client={client} />
        </div>
      )}
      {view === "skills" && (
        <div className="min-h-0 overflow-y-auto">
          <SkillsView client={client} />
        </div>
      )}
      {view === "kanban" && (
        <div className="min-h-0 overflow-y-auto">
          <KanbanView client={client} />
        </div>
      )}
      {view === "projects" && (
        <div className="min-h-0 overflow-y-auto">
          <ProjectsView client={client} />
        </div>
      )}
      {view === "runs" && (
        <div className="min-h-0 overflow-y-auto">
          <RunsView client={client} />
        </div>
      )}
      {view === "webhooks" && (
        <div className="min-h-0 overflow-y-auto">
          <WebhooksView client={client} />
        </div>
      )}
      {view === "plugins" && (
        <div className="min-h-0 overflow-y-auto">
          <PluginsView client={client} />
        </div>
      )}
      {view === "pairing" && (
        <div className="min-h-0 overflow-y-auto">
          <PairingView client={client} />
        </div>
      )}
      {view === "profiles" && (
        <div className="min-h-0 overflow-y-auto">
          <ProfilesView client={client} />
        </div>
      )}
      {view === "config" && (
        <div className="min-h-0 overflow-y-auto">
          <ConfigView client={client} />
        </div>
      )}
      {view === "doctor" && (
        <div className="min-h-0 overflow-y-auto">
          <DoctorView client={client} />
        </div>
      )}
      {view === "settings" && (
        <div className="min-h-0 overflow-y-auto">
          <SettingsView client={client} />
        </div>
      )}
      </div>
      {logOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setLogOpen(false)}>
          <div
            className="flex max-h-[80vh] w-[46rem] flex-col rounded-xl border border-(--border) bg-(--seed-card) p-4 shadow-[var(--shadow-composer)]"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between pb-2">
              <span className="text-sm font-semibold">{t.chrome.gatewayLogTitle}</span>
              <Button variant="ghost" size="xs" onClick={() => setLogOpen(false)}>
                {t.chrome.gatewayLogClose}
              </Button>
            </div>
            <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap rounded-md bg-(--hover) p-2 font-mono text-[11px] leading-4 text-(--dim)">
              {logLines.join("\n") || t.chrome.gatewayLogEmpty}
            </pre>
          </div>
        </div>
      )}
      <Palette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onView={(v) => setView(v)}
        onNewSession={() => {
          setActiveId(null);
          setView("chat");
        }}
      />
    </div>
  );
}
