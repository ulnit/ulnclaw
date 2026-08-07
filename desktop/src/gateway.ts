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
  /** Owning project slug (longest-prefix cwd match in projects.db). */
  project?: string | null;
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

export interface LearningGraphNode {
  id: string;
  label: string;
  kind: "skill" | "memory";
  timestamp: number | null;
  category: string;
  useCount: number;
  state: string;
  createdBy: string | null;
  pinned: boolean;
  memorySource?: string;
}

export interface LearningGraphPayload {
  nodes: LearningGraphNode[];
  edges: { source: string; target: string }[];
  clusters: { category: string; count: number }[];
  memory: { source: string; title: string; body: string; timestamp: number | null }[];
  stats: Record<string, number>;
}

export interface LearningNodeDetail {
  ok: boolean;
  kind: "skill" | "memory";
  label: string;
  content: string;
  message?: string;
}

/** One provider row from GET /api/model/options (gateway render_row). */
export interface ModelOptionRow {
  slug: string;
  models: string[];
  total_models?: number;
  is_user_defined?: boolean;
  authenticated?: boolean;
  source?: string;
  current?: boolean;
  name?: string;
  base_url?: string;
  mode?: string;
  probed?: boolean;
  key_env?: string;
  featured_models?: string[];
}

export interface ModelOptionsPayload {
  providers: ModelOptionRow[];
  model: string;
  provider: string;
}

export interface SkillRow {
  name: string;
  description: string;
  category?: string;
  path?: string;
}

export interface ToolsetRow {
  name: string;
  description: string;
  enabled: boolean;
  tools: string[];
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

export interface PetAtlasRow {
  state: string;
  row: number;
  frames: number;
}

export interface PetConfig {
  enabled: boolean;
  slug: string | null;
  scale: number | null;
  render_mode: string | null;
  atlas: {
    frame_w: number;
    frame_h: number;
    columns: number;
    rows: PetAtlasRow[];
  };
}

export type HatchStatus =
  | "generating_drafts"
  | "awaiting_pick"
  | "hatching"
  | "done"
  | "failed"
  | "cancelled";

export interface HatchJobResult {
  slug: string;
  display_name: string;
  states: string[];
  spritesheet: string;
}

export interface HatchJobStatus {
  job_id: string;
  status: HatchStatus;
  prompt?: string;
  style?: string | null;
  name?: string;
  drafts?: string[];
  progress?: { event: string; detail: string }[];
  result?: HatchJobResult | null;
  error?: string | null;
}

export interface KanbanTask {
  id: string;
  board: string;
  title: string;
  body: string;
  assignee: string | null;
  status: string;
  priority: number;
  created_by: string;
  created_at: number;
  started_at: number | null;
  completed_at: number | null;
  result: string | null;
  parents: string[];
  children: string[];
}

export interface KanbanBoard {
  slug: string;
  name: string;
  current: boolean;
  open_tasks: number;
  total_tasks: number;
}

export interface KanbanComment {
  id: number;
  author: string;
  body: string;
  created_at: number;
}

export interface KanbanDetail {
  task: KanbanTask;
  comments: KanbanComment[];
  attachments: { kind: string; value: string }[];
}

export interface ProjectFolder {
  path: string;
  label: string | null;
  is_primary: boolean;
  added_at: number;
}

export interface Project {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  icon: string | null;
  color: string | null;
  board_slug: string | null;
  primary_path: string | null;
  archived: boolean;
  created_at: number;
  folders: ProjectFolder[];
}

export interface DiscoveredRepo {
  root: string;
  label: string;
  last_seen: number;
}

export interface CronJob {
  id: string;
  name: string;
  schedule: string;
  prompt: string;
  skills: string[];
  enabled: boolean;
  repeat: number | null;
  next_run: number | null;
  created_at: number;
  last_run: number | null;
  last_status: string | null;
}

const SETTINGS_KEY = "ulnclaw.gateway";

export interface UsageSessionRow {
  id: string;
  source: string;
  model: string;
  title: string | null;
  started_at: number | null;
  ended_at: number | null;
  end_reason: string | null;
  message_count: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
}

export interface UsagePayload {
  process: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    tool_calls: number;
    requests: { chat_completions: number; responses: number; session_chats: number };
    runs: { started: number; completed: number; failed: number };
  };
  store: {
    sessions: number;
    messages: number;
    input_tokens: number;
    output_tokens: number;
    total_tokens: number;
  };
  sessions: UsageSessionRow[];
}

export interface ConfigPayload {
  path: string;
  config: Record<string, unknown>;
  redacted: string[];
  env_keys: string[];
  note: string;
}

export interface RunApproval {
  command: string;
  reason: string;
  choices: string[];
  resolved?: string;
}

export interface RunRow {
  run_id: string;
  status: string;
  session_id: string | null;
  message: string;
  created_at: number;
  finished_at: number | null;
  result: string | null;
  error: string | null;
  iterations: number | null;
  stop_requested: boolean;
  approval: RunApproval | null;
}

export interface MonitoringPayload {
  enabled: boolean;
  metrics: boolean;
  metrics_interval_seconds: number;
  diagnostic_events: boolean;
  warning_error_logs: boolean;
  logs_interval_seconds: number;
  otlp: { enabled: boolean; endpoint: string | null; transport: string };
  install_id: string | null;
  queue_depth: number;
  scope: string;
}

export interface WebhookSubscription {
  name: string;
  url: string;
  description: string;
  events: string[];
  deliver: string;
  deliver_only: boolean;
  script: string | null;
  created_at: string;
  has_secret: boolean;
  secret_preview: string;
}

export interface WebhookListPayload {
  base_url: string;
  subscriptions: WebhookSubscription[];
}

export interface WebhookCreateBody {
  name: string;
  description?: string;
  events?: string;
  prompt?: string;
  skills?: string;
  deliver?: string;
  deliver_chat_id?: string;
  deliver_only?: boolean;
  script?: string;
  secret?: string;
}

export interface DoctorCheck {
  level: "ok" | "warn" | "fail" | "info";
  text: string;
  detail?: string;
}

export interface DoctorSection {
  title: string;
  checks: DoctorCheck[];
}

export interface DoctorReport {
  sections: DoctorSection[];
  issues: string[];
  fixed: number;
}

export interface DoctorPayload {
  report: DoctorReport;
  online: boolean;
}

export interface ConfigSaveReply {
  ok: boolean;
  applied: string[];
  skipped_redacted: string[];
  path: string;
  note: string;
  error?: string;
}

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

  /** GET /api/model/options — provider/model inventory for the picker. */
  async modelOptions(): Promise<ModelOptionsPayload> {
    const response = await fetch(this.endpoint("/api/model/options"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`model options: HTTP ${response.status}`);
    return (await response.json()) as ModelOptionsPayload;
  }

  /** POST /api/sessions/:id/model — lock the session to a model. */
  async lockSessionModel(sessionId: string, model: string, provider?: string): Promise<void> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/model`), {
      method: "POST",
      headers: { ...this.headers(), "Content-Type": "application/json" },
      body: JSON.stringify(provider ? { model, provider } : { model }),
    });
    if (!response.ok) {
      throw new Error(`model lock: HTTP ${response.status}`);
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

  async learningGraph(): Promise<LearningGraphPayload> {
    const response = await fetch(this.endpoint("/api/learning/graph"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`learning graph: HTTP ${response.status}`);
    return (await response.json()) as LearningGraphPayload;
  }

  async learningNode(id: string): Promise<LearningNodeDetail> {
    const response = await fetch(
      this.endpoint(`/api/learning/node?id=${encodeURIComponent(id)}`),
      { headers: this.headers() },
    );
    const value = (await response.json()) as LearningNodeDetail;
    if (!response.ok || !value.ok) {
      throw new Error(value.message || `learning node: HTTP ${response.status}`);
    }
    return value;
  }

  async editLearningNode(id: string, content: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/learning/node"), {
      method: "PUT",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ id, content }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok || !value.ok) {
      throw new Error(value.message || `edit node: HTTP ${response.status}`);
    }
  }

  async deleteLearningNode(id: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/learning/node"), {
      method: "DELETE",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ id }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok || !value.ok) {
      throw new Error(value.message || `delete node: HTTP ${response.status}`);
    }
  }

  async messages(sessionId: string): Promise<MessageRow[]> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/messages`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`messages: HTTP ${response.status}`);
    const value = await response.json();
    return (value.messages || value || []) as MessageRow[];
  }

  /**
   * Subscribe to the desktop UI bridge event stream (P231). Fetch-based
   * SSE reader (EventSource can't carry the bearer header). Invokes
   * onEvent per `{session_id, event, payload}` envelope until aborted.
   */
  async desktopEvents(
    onEvent: (envelope: {
      session_id: string;
      event: string;
      payload: Record<string, unknown>;
    }) => void,
    signal: AbortSignal,
  ): Promise<void> {
    const response = await fetch(this.endpoint("/api/desktop/events"), {
      headers: this.headers(),
      signal,
    });
    if (!response.ok || !response.body) return;
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data:")) continue;
        const payload = trimmed.slice(5).trim();
        if (!payload || payload === "[DONE]") continue;
        try {
          const envelope = JSON.parse(payload);
          if (!envelope || typeof envelope.event !== "string") continue;
          if (typeof envelope.session_id !== "string") envelope.session_id = "";
          if (!envelope.payload || typeof envelope.payload !== "object") {
            envelope.payload = {};
          }
          onEvent(envelope);
        } catch {
          // ignore malformed frames
        }
      }
    }
  }

  /** Answer a pending `terminal.read` bridge request (P231). */
  async answerTerminalRead(id: string, ok: boolean, result: string): Promise<void> {
    await fetch(this.endpoint("/api/desktop/read-response"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ id, ok, result }),
    });
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
  async usage(limit = 50): Promise<UsagePayload> {
    const response = await fetch(this.endpoint(`/api/usage?limit=${limit}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`usage HTTP ${response.status}`);
    return (await response.json()) as UsagePayload;
  }

  /** GET /v1/toolsets — toolsets with resolved tool lists. */
  async listToolsets(): Promise<ToolsetRow[]> {
    try {
      const response = await fetch(this.endpoint("/v1/toolsets"), { headers: this.headers() });
      if (!response.ok) return [];
      const value = await response.json();
      return (value.data || []) as ToolsetRow[];
    } catch {
      return [];
    }
  }

  /** GET /v1/runs — tracked async runs (with pending approvals). */
  async listRuns(): Promise<RunRow[]> {
    const response = await fetch(this.endpoint("/v1/runs"), { headers: this.headers() });
    if (!response.ok) throw new Error(`runs HTTP ${response.status}`);
    const value = await response.json();
    return (value.data || value || []) as RunRow[];
  }

  /** POST /v1/runs/:id/approval — resolve a pending approval. */
  async approveRun(runId: string, decision: "once" | "session" | "always" | "deny"): Promise<void> {
    const response = await fetch(this.endpoint(`/v1/runs/${runId}/approval`), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ decision }),
    });
    if (!response.ok) {
      const value = (await response.json().catch(() => ({}))) as { error?: { message?: string } };
      throw new Error(value.error?.message || `approve HTTP ${response.status}`);
    }
  }

  /** POST /v1/runs/:id/stop — request a stop of a running/queued run. */
  async stopRun(runId: string): Promise<void> {
    const response = await fetch(this.endpoint(`/v1/runs/${runId}/stop`), {
      method: "POST",
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`stop run HTTP ${response.status}`);
  }

  /** GET /api/monitoring — health-export posture for the Doctor view. */
  async monitoring(): Promise<MonitoringPayload> {
    const response = await fetch(this.endpoint("/api/monitoring"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`monitoring HTTP ${response.status}`);
    return (await response.json()) as MonitoringPayload;
  }

  /** GET /api/webhooks/subscriptions — dynamic webhook routes. */
  async webhooksList(): Promise<WebhookListPayload> {
    const response = await fetch(this.endpoint("/api/webhooks/subscriptions"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`webhooks HTTP ${response.status}`);
    return (await response.json()) as WebhookListPayload;
  }

  /** POST /api/webhooks/subscriptions — create/update a subscription. */
  async webhooksCreate(body: WebhookCreateBody): Promise<{ ok: boolean; name: string; message: string }> {
    const response = await fetch(this.endpoint("/api/webhooks/subscriptions"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(body),
    });
    const value = (await response.json().catch(() => ({}))) as {
      ok?: boolean; name?: string; message?: string; error?: string;
    };
    if (!response.ok) throw new Error(value.error || `webhook create HTTP ${response.status}`);
    return { ok: Boolean(value.ok), name: value.name || "", message: value.message || "" };
  }

  /** DELETE /api/webhooks/subscriptions/:name. */
  async webhooksDelete(name: string): Promise<void> {
    const response = await fetch(
      this.endpoint(`/api/webhooks/subscriptions/${encodeURIComponent(name)}`),
      { method: "DELETE", headers: this.headers() },
    );
    if (!response.ok) {
      const value = (await response.json().catch(() => ({}))) as { error?: string };
      throw new Error(value.error || `webhook delete HTTP ${response.status}`);
    }
  }

  /** POST /api/webhooks/subscriptions/:name/test — signed test fire. */
  async webhooksTest(name: string, payload?: string): Promise<{ ok: boolean; message: string }> {
    const response = await fetch(
      this.endpoint(`/api/webhooks/subscriptions/${encodeURIComponent(name)}/test`),
      {
        method: "POST",
        headers: this.headers(),
        body: JSON.stringify(payload ? { payload } : {}),
      },
    );
    const value = (await response.json().catch(() => ({}))) as {
      ok?: boolean; message?: string; error?: string;
    };
    if (!response.ok) throw new Error(value.error || `webhook test HTTP ${response.status}`);
    return { ok: Boolean(value.ok), message: value.message || "" };
  }

  /** GET /api/sessions/:id/export — download the transcript as md/html. */
  async exportSession(
    sessionId: string,
    format: "md" | "html",
  ): Promise<{ blob: Blob; filename: string }> {
    const qs = format === "html" ? "?format=html" : "";
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/export${qs}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`export HTTP ${response.status}`);
    const disposition = response.headers.get("content-disposition") || "";
    const match = /filename="?([^";]+)"?/.exec(disposition);
    const filename =
      match?.[1] || `ulnclaw-session-${sessionId.slice(0, 8)}.${format}`;
    return { blob: await response.blob(), filename };
  }

  /** GET /api/doctor — run the doctor report, optionally with online probes. */
  async doctor(online = false): Promise<DoctorPayload> {
    const qs = online ? "?online=true" : "";
    const response = await fetch(this.endpoint(`/api/doctor${qs}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`doctor HTTP ${response.status}`);
    return (await response.json()) as DoctorPayload;
  }

  /** GET /api/config — config.toml as redacted nested JSON. */
  async configGet(): Promise<ConfigPayload> {
    const response = await fetch(this.endpoint("/api/config"), { headers: this.headers() });
    if (!response.ok) throw new Error(`config HTTP ${response.status}`);
    return (await response.json()) as ConfigPayload;
  }

  /** PUT /api/config — apply dotted-path sets/unsets to config.toml. */
  async configSave(
    set: Record<string, unknown>,
    unset: string[],
  ): Promise<ConfigSaveReply> {
    const response = await fetch(this.endpoint("/api/config"), {
      method: "PUT",
      headers: this.headers(),
      body: JSON.stringify({ set, unset }),
    });
    const value = (await response.json().catch(() => ({}))) as ConfigSaveReply;
    if (!response.ok) {
      throw new Error(value.error || `config save HTTP ${response.status}`);
    }
    return value;
  }

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

  // ---- Petdex mascot surfaces ----

  async petConfig(): Promise<PetConfig | null> {
    try {
      const response = await fetch(this.endpoint("/api/pets/config"), { headers: this.headers() });
      if (!response.ok) return null;
      return response.json();
    } catch {
      return null;
    }
  }

  spritesheetUrl(slug: string): string {
    return this.endpoint(`/api/pets/${encodeURIComponent(slug)}/spritesheet`);
  }

  /** Number of in-flight /v1/runs (drives the pet's working animation). */
  async activeRunCount(): Promise<number> {
    try {
      const response = await fetch(this.endpoint("/v1/runs"), { headers: this.headers() });
      if (!response.ok) return 0;
      const value = await response.json();
      const runs = (value.data || []) as { finished_at: number | null }[];
      return runs.filter((r) => r.finished_at === null || r.finished_at === 0).length;
    } catch {
      return 0;
    }
  }

  // ---- Kanban board (shared with the CLI + agent tools) ----

  private async kanbanJson(path: string, init?: RequestInit): Promise<any | null> {
    try {
      const response = await fetch(this.endpoint(path), {
        headers: this.headers(),
        ...init,
      });
      if (!response.ok) return null;
      return response.json();
    } catch {
      return null;
    }
  }

  async kanbanBoards(): Promise<KanbanBoard[]> {
    const value = await this.kanbanJson("/api/kanban/boards");
    return (value?.boards || []) as KanbanBoard[];
  }

  async kanbanSwitchBoard(slug: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/kanban/boards/${encodeURIComponent(slug)}/switch`, {
      method: "POST",
      body: "{}",
    });
    return Boolean(value?.ok);
  }

  async kanbanTasks(board?: string): Promise<KanbanTask[]> {
    const query = board ? `?board=${encodeURIComponent(board)}&limit=500` : "?limit=500";
    const value = await this.kanbanJson(`/api/kanban/tasks${query}`);
    return (value?.tasks || []) as KanbanTask[];
  }

  async kanbanCreateTask(title: string, body?: string): Promise<KanbanTask | null> {
    const value = await this.kanbanJson("/api/kanban/tasks", {
      method: "POST",
      body: JSON.stringify({ title, body: body || "" }),
    });
    return (value?.task || null) as KanbanTask | null;
  }

  async kanbanTask(id: string): Promise<KanbanDetail | null> {
    return (await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}`)) as KanbanDetail | null;
  }

  async kanbanComplete(id: string, result?: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}/complete`, {
      method: "POST",
      body: JSON.stringify({ result: result || "" }),
    });
    return Boolean(value?.task);
  }

  async kanbanBlock(id: string, reason: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}/block`, {
      method: "POST",
      body: JSON.stringify({ reason }),
    });
    return Boolean(value?.task);
  }

  async kanbanUnblock(id: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}/unblock`, {
      method: "POST",
      body: "{}",
    });
    return Boolean(value?.task);
  }

  async kanbanComment(id: string, body: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}/comment`, {
      method: "POST",
      body: JSON.stringify({ body, author: "desktop" }),
    });
    return Boolean(value?.ok);
  }

  // ---- Projects registry (shared with the `ulnclaw project` CLI) ----

  async projectsList(includeArchived = false): Promise<{ active_id: string | null; projects: Project[] }> {
    const query = includeArchived ? "?all=true" : "";
    const value = await this.kanbanJson(`/api/projects${query}`);
    return {
      active_id: (value?.active_id || null) as string | null,
      projects: (value?.projects || []) as Project[],
    };
  }

  async projectCreate(request: {
    name: string;
    folders?: string[];
    board_slug?: string;
    use?: boolean;
  }): Promise<Project | null> {
    const value = await this.kanbanJson("/api/projects", {
      method: "POST",
      body: JSON.stringify(request),
    });
    return (value?.project || null) as Project | null;
  }

  async projectUpdate(
    id: string,
    patch: { name?: string; description?: string; board_slug?: string },
  ): Promise<Project | null> {
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    });
    return (value?.project || null) as Project | null;
  }

  async projectDelete(id: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return Boolean(value?.id);
  }

  async projectAddFolder(id: string, path: string, primary = false): Promise<Project | null> {
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}/folders`, {
      method: "POST",
      body: JSON.stringify({ path, primary }),
    });
    return (value?.project || null) as Project | null;
  }

  async projectRemoveFolder(id: string, path: string): Promise<Project | null> {
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}/folders`, {
      method: "DELETE",
      body: JSON.stringify({ path }),
    });
    return (value?.project || null) as Project | null;
  }

  async projectSetPrimary(id: string, path: string): Promise<Project | null> {
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}/primary`, {
      method: "POST",
      body: JSON.stringify({ path }),
    });
    return (value?.project || null) as Project | null;
  }

  async projectArchive(id: string, archived: boolean): Promise<boolean> {
    const action = archived ? "restore" : "archive";
    const value = await this.kanbanJson(`/api/projects/${encodeURIComponent(id)}/${action}`, {
      method: "POST",
      body: "{}",
    });
    return Boolean(value?.id);
  }

  async projectSetActive(id: string | null): Promise<boolean> {
    const value = await this.kanbanJson("/api/projects/active", {
      method: "POST",
      body: JSON.stringify({ id }),
    });
    return value !== null;
  }

  async projectScan(roots?: string[]): Promise<{ recorded: number } | null> {
    const value = await this.kanbanJson("/api/projects/scan", {
      method: "POST",
      body: JSON.stringify({ roots: roots || [] }),
    });
    return value ? { recorded: (value.recorded || 0) as number } : null;
  }

  async projectsRepos(): Promise<DiscoveredRepo[]> {
    const value = await this.kanbanJson("/api/projects/repos");
    return (value?.repos || []) as DiscoveredRepo[];
  }

  // ---- Cron jobs (hermes cron dashboard, /api/jobs) ----

  async jobsList(includeDisabled = true): Promise<CronJob[]> {
    const query = includeDisabled ? "?include_disabled=true" : "";
    const value = await this.kanbanJson(`/api/jobs${query}`);
    return (value?.jobs || []) as CronJob[];
  }

  async jobCreate(request: {
    name: string;
    schedule: string;
    prompt: string;
    skills?: string[];
    repeat?: number;
  }): Promise<CronJob | null> {
    const value = await this.kanbanJson("/api/jobs", {
      method: "POST",
      body: JSON.stringify(request),
    });
    return (value?.job || value) as CronJob | null;
  }

  async jobUpdate(
    id: string,
    patch: { name?: string; schedule?: string; prompt?: string; enabled?: boolean },
  ): Promise<CronJob | null> {
    const value = await this.kanbanJson(`/api/jobs/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    });
    return (value?.job || value) as CronJob | null;
  }

  async jobDelete(id: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/jobs/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return value !== null;
  }

  async jobSetEnabled(id: string, enabled: boolean): Promise<boolean> {
    const action = enabled ? "resume" : "pause";
    const value = await this.kanbanJson(`/api/jobs/${encodeURIComponent(id)}/${action}`, {
      method: "POST",
      body: "{}",
    });
    return value !== null;
  }

  async jobRunNow(id: string): Promise<boolean> {
    const value = await this.kanbanJson(`/api/jobs/${encodeURIComponent(id)}/run`, {
      method: "POST",
      body: "{}",
    });
    return value !== null;
  }

  // ---- Pet hatch jobs (desktop hatch overlay, hermes pet-generate parity) ----

  /** Gateway hatch requests surface the server error message verbatim. */
  private async hatchJson(path: string, init?: RequestInit): Promise<any> {
    const response = await fetch(this.endpoint(path), {
      headers: this.headers(),
      ...init,
    });
    const value = await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error(value?.error?.message || `HTTP ${response.status}`);
    }
    return value;
  }

  async hatchStart(request: {
    prompt: string;
    style?: string;
    name?: string;
    drafts?: number;
    auto?: boolean;
  }): Promise<{ job_id: string }> {
    return this.hatchJson("/api/pets/hatch", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async hatchJob(jobId: string): Promise<HatchJobStatus> {
    return this.hatchJson(`/api/pets/hatch/${encodeURIComponent(jobId)}`);
  }

  async hatchPick(jobId: string, draft: number, name?: string): Promise<HatchJobStatus> {
    return this.hatchJson(`/api/pets/hatch/${encodeURIComponent(jobId)}/pick`, {
      method: "POST",
      body: JSON.stringify({ draft, name: name || undefined }),
    });
  }

  async hatchCancel(jobId: string): Promise<HatchJobStatus | null> {
    try {
      return await this.hatchJson(`/api/pets/hatch/${encodeURIComponent(jobId)}/cancel`, {
        method: "POST",
        body: "{}",
      });
    } catch {
      return null;
    }
  }

  /** Draft/spritesheet bytes need auth headers, so fetch them as blob URLs. */
  async hatchImageBlob(pathName: string): Promise<string> {
    const response = await fetch(this.endpoint(pathName), { headers: this.headers() });
    if (!response.ok) throw new Error(`image: HTTP ${response.status}`);
    const blob = await response.blob();
    return URL.createObjectURL(blob);
  }
}
