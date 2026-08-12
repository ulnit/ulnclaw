import { useEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import type { GatewayClient, MessageRow, ToolCardEvent } from "../gateway";
import { Button } from "../hermes/button";
import { Loader } from "../hermes/loader";
import { Icon } from "./Icon";
import { ModelPicker } from "./ModelPicker";
import { useT } from "./util";

interface Props {
  client: GatewayClient;
  sessionId: string | null;
  onCreated: (id: string) => void;
}

interface ToolCard {
  callId: string;
  name: string;
  status: "running" | "done";
  args?: string;
  result?: string;
}

const md = (text: string): string =>
  DOMPurify.sanitize(marked.parse(text, { async: false }) as string);

function dayKey(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleDateString();
}

export function ChatView({ client, sessionId, onCreated }: Props) {
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [streaming, setStreaming] = useState("");
  const [cards, setCards] = useState<ToolCard[]>([]);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState("");
  const t = useT();
  const [model, setModel] = useState("");
  const [attach, setAttach] = useState<{ name: string; dataUrl: string }[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

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
  }, [messages, streaming, cards]);

  const onToolCard = (e: ToolCardEvent) => {
    setCards((prev) => {
      const existing = prev.find((c) => c.callId === e.callId);
      if (e.kind === "started") {
        if (existing) return prev;
        return [...prev, { callId: e.callId, name: e.name || "tool", status: "running", args: e.arguments }];
      }
      if (existing) {
        return prev.map((c) =>
          c.callId === e.callId ? { ...c, status: "done", result: e.result } : c,
        );
      }
      return [...prev, { callId: e.callId, name: e.name || "tool", status: "done", result: e.result }];
    });
  };

  const pickFiles = (files: FileList | null) => {
    if (!files) return;
    for (const file of Array.from(files)) {
      const reader = new FileReader();
      reader.onload = () => {
        setAttach((prev) => [...prev, { name: file.name, dataUrl: String(reader.result) }]);
      };
      reader.readAsDataURL(file);
    }
  };

  const send = async () => {
    const text = draft.trim();
    if ((!text && attach.length === 0) || busy) return;
    setBusy(true);
    setDraft("");
    setStreaming("");
    setCards([]);
    const images = attach.map((a) => a.dataUrl);
    setAttach([]);
    try {
      let id = sessionId;
      if (!id) {
        const row = await client.createSession({});
        id = row.id;
        onCreated(id);
      }
      setMessages((prev) => [...prev, { role: "user", content: text, timestamp: Date.now() / 1000 }]);
      const finalText = await client.chatStream(
        id,
        text,
        (chunk) => setStreaming((prev) => prev + chunk),
        undefined,
        onToolCard,
        images,
      );
      setStreaming("");
      setCards([]);
      setMessages((prev) => [...prev, { role: "assistant", content: finalText, timestamp: Date.now() / 1000 }]);
    } catch {
      setStreaming("");
      setCards([]);
      setMessages((prev) => [
        ...prev,
        { role: "system", content: "gateway unreachable — is the gateway running?" },
      ]);
    } finally {
      setBusy(false);
    }
  };

  const visible = messages.filter(
    (m) => (m.role === "user" || m.role === "assistant" || m.role === "system") && m.content,
  );

  const rows = useMemo(() => {
    const out: { divider?: string; msg?: MessageRow; key: string }[] = [];
    let lastDay = "";
    for (const [i, m] of visible.entries()) {
      const ts = m.timestamp ?? 0;
      if (ts) {
        const day = dayKey(ts);
        if (day !== lastDay) {
          out.push({ divider: day, key: `d-${day}` });
          lastDay = day;
        }
      }
      out.push({ msg: m, key: `m-${i}` });
    }
    return out;
  }, [visible]);

  return (
    <main className="flex min-h-0 flex-col">
      <header className="flex items-center justify-between border-b border-[color-mix(in_srgb,var(--seed-fg)_4%,transparent)] px-4 py-2.5">
        <span className="text-[13px] font-semibold">{t.chrome.chatTab}</span>
        <ModelPicker client={client} onChanged={setModel} />
      </header>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-[52rem] flex-col gap-4 px-[clamp(1.25rem,4vw,3rem)] py-6">
          {visible.length === 0 && !streaming && (
            <div className="flex flex-col items-center gap-2 py-24 text-center">
              <span className="text-sm font-semibold">{t.chrome.selectOrStart}</span>
              <span className="max-w-[26rem] text-xs leading-5 text-(--dim)">
                Start a conversation below — a session is created on your first
                message. Attach images with the clip icon.
              </span>
            </div>
          )}
          {rows.map(({ divider, msg, key }) =>
            divider ? (
              <div key={key} className="flex items-center gap-2 py-1">
                <span className="shrink-0 text-[0.64rem] font-semibold uppercase tracking-[0.12em] text-(--dim-faint)">
                  {divider}
                </span>
                <span aria-hidden="true" className="h-px flex-1 bg-(--border)" />
              </div>
            ) : msg?.role === "user" ? (
              <div key={key} className="flex justify-end">
                <div className="max-w-[75%] rounded-xl border border-(--border) bg-[color-mix(in_srgb,var(--accent)_6%,var(--seed-card))] px-3.5 py-2.5 text-[14px] leading-[1.5] whitespace-pre-wrap">
                  {msg.content}
                </div>
              </div>
            ) : msg?.role === "system" ? (
              <div key={key} className="text-center text-xs text-(--dim)">
                {msg.content}
              </div>
            ) : (
              <div
                key={key}
                className="md-body text-[14px] leading-[1.6]"
                dangerouslySetInnerHTML={{ __html: md(msg?.content ?? "") }}
              />
            ),
          )}
          {cards.length > 0 && (
            <div className="flex flex-col gap-1">
              {cards.map((c) => (
                <details key={c.callId} className="group rounded-md">
                  <summary className="flex cursor-pointer list-none items-center gap-2 py-0.5 text-xs text-[color-mix(in_srgb,var(--seed-fg)_64%,transparent)]">
                    <Icon name="wrench" className="size-3" />
                    <span className="font-semibold text-(--accent)">{c.name}</span>
                    <span className="ml-auto">
                      {c.status === "running" ? "running…" : "done"}
                    </span>
                  </summary>
                  {(c.args || c.result) && (
                    <pre className="mt-1 max-h-56 overflow-y-auto rounded-md bg-[color-mix(in_srgb,var(--seed-fg)_4%,transparent)] p-2.5 font-[var(--mono)] text-[11px] text-(--dim) whitespace-pre-wrap">
                      {c.result ?? c.args ?? ""}
                    </pre>
                  )}
                </details>
              ))}
            </div>
          )}
          {streaming && (
            <div
              className="md-body text-[14px] leading-[1.6]"
              dangerouslySetInnerHTML={{ __html: md(streaming) }}
            />
          )}
        </div>
      </div>

      <div className="px-4 pb-4 pt-1">
        <div className="mx-auto flex w-full max-w-[48rem] flex-wrap items-center gap-1.5 rounded-2xl border border-[color-mix(in_srgb,var(--ring)_18%,var(--border))] bg-(--panel-2) p-3 shadow-[var(--shadow-composer)] focus-within:border-[color-mix(in_srgb,var(--accent)_40%,var(--border))]">
          {attach.length > 0 && (
            <div className="flex w-full flex-wrap gap-1.5 pb-1">
              {attach.map((a, i) => (
                <span
                  key={i}
                  className="inline-flex max-w-[16rem] items-center gap-1.5 rounded-full bg-(--hover) px-2.5 py-1 text-[11px] text-(--dim)"
                >
                  <span className="truncate text-(--text)">{a.name}</span>
                  <button
                    className="text-(--dim) hover:text-(--text)"
                    onClick={() => setAttach((prev) => prev.filter((_, j) => j !== i))}
                  >
                    <Icon name="x" className="size-2.5" />
                  </button>
                </span>
              ))}
            </div>
          )}
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
            placeholder={t.chrome.inputPlaceholder}
            className="min-w-0 flex-1 resize-none bg-transparent px-1 text-[14px] leading-[1.5] outline-none placeholder:text-(--dim-faint)"
          />
          <div className="flex w-full items-center gap-1.5">
            <button
              className="inline-flex size-7 items-center justify-center rounded-[4px] text-(--dim) hover:bg-(--hover) hover:text-(--text)"
              title="Attach file"
              onClick={() => fileRef.current?.click()}
            >
              <Icon name="link" className="size-3.5" />
            </button>
            <input
              ref={fileRef}
              type="file"
              multiple
              accept="image/*"
              hidden
              onChange={(e) => pickFiles(e.target.files)}
            />
            <span className="text-[11px] text-(--dim-faint) tabular-nums">
              {draft.length > 0 ? draft.length : ""}
            </span>
            <span className="flex-1" />
            {busy && <Loader type="lemniscate-bloom" />}
            <Button variant="default" disabled={busy || (draft.trim().length === 0 && attach.length === 0)} onClick={() => void send()}>
              {t.chrome.send}
            </Button>
          </div>
        </div>
      </div>
    </main>
  );
}
