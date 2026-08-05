// Typed client for the ulnclaw HTTP gateway (OpenAI-compatible API +
// session endpoints). See docs/en/architecture.md for the endpoint
// inventory.

export interface GatewaySettings {
  url: string;
  key: string;
  manage: boolean;
}

export interface SessionRow {
  id: string;
  title: string | null;
  source: string;
  model: string | null;
  started_at: number;
  last_activity_at: number;
  message_count?: number;
}

export interface MessageRow {
  role: string;
  content: string | null;
  name?: string | null;
}

export interface ChatReply {
  content: string;
  session_id: string;
}

export interface SkillRow {
  name: string;
  description: string;
}

export interface ToolCardEvent {
  kind: "started" | "completed";
  callId: string;
  name?: string;
  arguments?: string;
  result?: string;
}

export interface UploadReply {
  path: string;
  mime: string;
  bytes: number;
}

const SETTINGS_KEY = "ulnclaw.gateway";

export function loadSettings(): GatewaySettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      return {
        url: parsed.url || "http://127.0.0.1:8642",
        key: parsed.key || "",
        manage: Boolean(parsed.manage),
      };
    }
  } catch {
    /* fall through */
  }
  return { url: "http://127.0.0.1:8642", key: "", manage: true };
}

export function saveSettings(settings: GatewaySettings): void {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}

export class GatewayClient {
  constructor(public settings: GatewaySettings) {}

  private headers(): Record<string, string> {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (this.settings.key) {
      headers["Authorization"] = `Bearer ${this.settings.key}`;
    }
    return headers;
  }

  private endpoint(path: string): string {
    return `${this.settings.url.replace(/\/$/, "")}${path}`;
  }

  async health(): Promise<boolean> {
    try {
      const response = await fetch(this.endpoint("/health"), {
        headers: this.headers(),
        signal: AbortSignal.timeout(3000),
      });
      return response.ok;
    } catch {
      return false;
    }
  }

  async models(): Promise<string> {
    try {
      const response = await fetch(this.endpoint("/v1/models"), { headers: this.headers() });
      if (!response.ok) return "";
      const value = await response.json();
      const first = value?.data?.[0];
      return first?.id || "";
    } catch {
      return "";
    }
  }

  async listSessions(): Promise<SessionRow[]> {
    const response = await fetch(this.endpoint("/api/sessions"), { headers: this.headers() });
    if (!response.ok) throw new Error(`sessions: HTTP ${response.status}`);
    const value = await response.json();
    return (value.sessions || value || []) as SessionRow[];
  }

  async createSession(): Promise<SessionRow> {
    const response = await fetch(this.endpoint("/api/sessions"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ source: "desktop" }),
    });
    if (!response.ok) throw new Error(`create session: HTTP ${response.status}`);
    return response.json();
  }

  async messages(sessionId: string): Promise<MessageRow[]> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/messages`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`messages: HTTP ${response.status}`);
    const value = await response.json();
    return (value.messages || value || []) as MessageRow[];
  }

  /** Stream a chat turn over SSE; invokes onDelta per content chunk. */
  async chatStream(
    sessionId: string,
    message: string,
    onDelta: (chunk: string) => void,
    onToolProgress?: (tool: string, status: string) => void,
    onToolCard?: (event: ToolCardEvent) => void,
  ): Promise<string> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/chat/stream`), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ message }),
    });
    if (!response.ok || !response.body) {
      // Fall back to the non-streaming endpoint.
      return this.chat(sessionId, message);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let full = "";
    let eventName = "";
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.startsWith("event:")) {
          eventName = trimmed.slice(6).trim();
          continue;
        }
        if (!trimmed.startsWith("data:")) continue;
        const payload = trimmed.slice(5).trim();
        const currentEvent = eventName;
        eventName = "";
        if (!payload || payload === "[DONE]") continue;
        try {
          const event = JSON.parse(payload);
          if (currentEvent === "hermes.tool.progress") {
            if (onToolProgress && typeof event?.tool === "string") {
              onToolProgress(event.tool, String(event?.status ?? ""));
            }
            continue;
          }
          if (currentEvent === "hermes.tool.started") {
            if (onToolCard && typeof event?.call_id === "string") {
              onToolCard({
                kind: "started",
                callId: event.call_id,
                name: typeof event?.name === "string" ? event.name : undefined,
                arguments: typeof event?.arguments === "string" ? event.arguments : undefined,
              });
            }
            continue;
          }
          if (currentEvent === "hermes.tool.completed") {
            if (onToolCard && typeof event?.call_id === "string") {
              onToolCard({
                kind: "completed",
                callId: event.call_id,
                result: typeof event?.result === "string" ? event.result : undefined,
              });
            }
            continue;
          }
          const delta =
            event?.choices?.[0]?.delta?.content ??
            event?.delta ??
            event?.content ??
            "";
          if (typeof delta === "string" && delta.length > 0) {
            full += delta;
            onDelta(delta);
          }
        } catch {
          /* non-JSON keepalive line */
        }
      }
    }
    return full;
  }

  async chat(sessionId: string, message: string): Promise<string> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/chat`), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ message }),
    });
    if (!response.ok) throw new Error(`chat: HTTP ${response.status}`);
    const value = await response.json();
    return value.content ?? value.reply ?? JSON.stringify(value);
  }

  /** Rename a session (PATCH accepts only `title` / `end_reason`). */
  async renameSession(sessionId: string, title: string): Promise<void> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}`), {
      method: "PATCH",
      headers: this.headers(),
      body: JSON.stringify({ title }),
    });
    if (!response.ok) throw new Error(`rename session: HTTP ${response.status}`);
  }

  /** Delete a session and its transcript. */
  async deleteSession(sessionId: string): Promise<void> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}`), {
      method: "DELETE",
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`delete session: HTTP ${response.status}`);
  }

  /** Installed skills (drives the composer's /slash completion). */
  async listSkills(): Promise<SkillRow[]> {
    try {
      const response = await fetch(this.endpoint("/v1/skills"), { headers: this.headers() });
      if (!response.ok) return [];
      const value = await response.json();
      return (value.data || []) as SkillRow[];
    } catch {
      return [];
    }
  }

  /** Upload a binary blob (clipboard image) into the media cache. */
  async uploadFile(blob: Blob, name: string): Promise<UploadReply> {
    const headers: Record<string, string> = { "Content-Type": blob.type || "application/octet-stream" };
    if (this.settings.key) {
      headers["Authorization"] = `Bearer ${this.settings.key}`;
    }
    const url = `${this.endpoint("/api/uploads")}?name=${encodeURIComponent(name)}`;
    const response = await fetch(url, { method: "POST", headers, body: blob });
    if (!response.ok) throw new Error(`upload: HTTP ${response.status}`);
    return response.json();
  }
}
