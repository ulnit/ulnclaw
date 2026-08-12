import { useEffect, useRef, useState } from "react";
import type { GatewayClient, MessageRow } from "../gateway";
import { Button } from "../hermes/button";
import { Loader } from "../hermes/loader";

interface Props {
  client: GatewayClient;
  sessionId: string | null;
  onCreated: (id: string) => void;
}

export function ChatView({ client, sessionId, onCreated }: Props) {
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [streaming, setStreaming] = useState("");
  const [toolLine, setToolLine] = useState("");
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState("");
  const [model, setModel] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!sessionId) {
      setMessages([]);
      return;
    }
    let cancelled = false;
    client
      .messages(sessionId, { timestamps: true })
      .then((rows) => {
        if (!cancelled) setMessages(rows);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [client, sessionId]);

  useEffect(() => {
    void client.models().then(setModel).catch(() => undefined);
  }, [client]);

  useEffect(() => {
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [messages, streaming]);

  const send = async () => {
    const text = draft.trim();
    if (!text || busy) return;
    setBusy(true);
    setDraft("");
    setStreaming("");
    try {
      let id = sessionId;
      if (!id) {
        const row = await client.createSession({});
        id = row.id;
        onCreated(id);
      }
      setMessages((prev) => [...prev, { role: "user", content: text }]);
      const finalText = await client.chatStream(
        id,
        text,
        (chunk) => setStreaming((prev) => prev + chunk),
        (tool, status) => setToolLine(`${tool} — ${status}`),
      );
      setStreaming("");
      setToolLine("");
      setMessages((prev) => [...prev, { role: "assistant", content: finalText }]);
    } catch {
      setToolLine("gateway unreachable");
    } finally {
      setBusy(false);
    }
  };

  const visible = messages.filter(
    (m) => (m.role === "user" || m.role === "assistant") && m.content,
  );

  return (
    <main className="flex min-h-0 flex-col">
      <header className="flex items-center justify-between border-b border-[color-mix(in_srgb,var(--seed-fg)_4%,transparent)] px-4 py-2.5">
        <span className="text-[13px] font-semibold">Chat</span>
        <span className="rounded-md px-2 py-0.5 text-xs text-(--dim) hover:bg-(--hover)">
          {model || "no model"}
        </span>
      </header>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-[52rem] flex-col gap-4 px-[clamp(1.25rem,4vw,3rem)] py-6">
          {visible.length === 0 && !streaming && (
            <div className="flex flex-col items-center gap-2 py-24 text-center">
              <span className="text-sm font-semibold">No session selected</span>
              <span className="max-w-[26rem] text-xs leading-5 text-(--dim)">
                Start a conversation below — a session is created on your first
                message. Secondary views live in the classic shell for now.
              </span>
            </div>
          )}
          {visible.map((m, i) =>
            m.role === "user" ? (
              <div key={i} className="flex justify-end">
                <div className="max-w-[75%] rounded-xl border border-(--border) bg-[color-mix(in_srgb,var(--accent)_6%,var(--seed-card))] px-3.5 py-2.5 text-[14px] leading-[1.5] whitespace-pre-wrap">
                  {m.content}
                </div>
              </div>
            ) : (
              <div key={i} className="text-[14px] leading-[1.6] whitespace-pre-wrap">
                {m.content}
              </div>
            ),
          )}
          {streaming && (
            <div className="text-[14px] leading-[1.6] whitespace-pre-wrap">
              {streaming}
              <span className="animate-pulse">▍</span>
            </div>
          )}
        </div>
      </div>

      {toolLine && (
        <div className="flex items-center gap-2 px-4 pb-1 text-xs text-(--dim)">
          <Loader type="lemniscate-bloom" />
          <span className="truncate">{toolLine}</span>
        </div>
      )}

      <div className="px-4 pb-4 pt-1">
        <div className="mx-auto flex w-full max-w-[48rem] flex-wrap items-center gap-1.5 rounded-2xl border border-[color-mix(in_srgb,var(--ring)_18%,var(--border))] bg-(--panel-2) p-3 shadow-[var(--shadow-composer)] focus-within:border-[color-mix(in_srgb,var(--accent)_40%,var(--border))]">
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void send();
              }
            }}
            rows={2}
            placeholder="Message ulnclaw… (Enter to send, Shift+Enter for newline)"
            className="min-w-0 flex-1 resize-none bg-transparent px-1 text-[14px] leading-[1.5] outline-none placeholder:text-(--dim-faint)"
          />
          <div className="flex w-full items-center justify-end gap-2">
            <span className="mr-auto text-[11px] text-(--dim-faint) tabular-nums">
              {draft.length > 0 ? draft.length : ""}
            </span>
            <Button variant="default" disabled={busy || draft.trim().length === 0} onClick={() => void send()}>
              Send
            </Button>
          </div>
        </div>
      </div>
    </main>
  );
}
