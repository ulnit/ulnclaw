import { useCallback, useEffect, useState } from "react";
import { GatewayClient, loadSettings } from "../gateway";
import type { SessionRow } from "../gateway";
import { Sidebar, type ShellView } from "./Sidebar";
import { ChatView } from "./ChatView";
import { SessionsView } from "./views/SessionsView";
import { JobsView } from "./views/JobsView";
import { UsageView } from "./views/UsageView";
import { ModelsView } from "./views/ModelsView";

export function App() {
  const [client] = useState(() => new GatewayClient(loadSettings()));
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<ShellView>("chat");
  const [online, setOnline] = useState(false);

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
    </div>
  );
}
