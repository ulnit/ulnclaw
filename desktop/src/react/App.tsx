import { useCallback, useEffect, useState } from "react";
import { GatewayClient, loadSettings } from "../gateway";
import type { SessionRow } from "../gateway";
import { Sidebar } from "./Sidebar";
import { ChatView } from "./ChatView";

export function App() {
  const [client] = useState(() => new GatewayClient(loadSettings()));
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
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

  return (
    <div className="grid h-full min-h-0 grid-cols-[14.8125rem_1fr]">
      <Sidebar
        sessions={sessions}
        activeId={activeId}
        online={online}
        onSelect={setActiveId}
        onCreated={(id) => {
          setActiveId(id);
          void refresh();
        }}
        onRefresh={refresh}
      />
      <ChatView client={client} sessionId={activeId} onCreated={(id) => { setActiveId(id); void refresh(); }} />
    </div>
  );
}
