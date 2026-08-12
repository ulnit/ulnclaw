import { useCallback, useEffect, useState } from "react";
import { GatewayClient, loadSettings } from "../gateway";
import type { SessionRow } from "../gateway";
import { Sidebar, type ShellView } from "./Sidebar";
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

  const refresh = useCallback(async () => {
    try {
      setSessions(await client.listSessions(true, false));
      setOnline(true);
    } catch {
      setOnline(false);
    }
  }, [client]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15000);
    return () => window.clearInterval(timer);
  }, [refresh]);

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
