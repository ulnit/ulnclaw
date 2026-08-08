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
  /** End reason stored on the session (complete/branched/compression…; P409). */
  end_reason?: string | null;
  /** Owning project slug (longest-prefix cwd match in projects.db). */
  project?: string | null;
}

export interface MessageRow {
  role: string;
  content: string | null;
  name?: string | null;
  tool_call_id?: string | null;
  tool_calls?: { id: string; function?: { name?: string; arguments?: string } }[];
  /** Stored epoch seconds when requested via `timestamps=true` (P367). */
  timestamp?: number;
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
export interface ModelCapabilities {
  reasoning?: boolean;
  tools?: boolean;
  vision?: boolean;
  context_window?: number | null;
  max_output_tokens?: number | null;
  family?: string;
  cost?: { input_per_mtok: number; output_per_mtok: number };
}

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
  api?: string;
  doc?: string;
  catalog?: string;
  catalog_stale?: boolean;
  capabilities?: Record<string, ModelCapabilities>;
  pricing?: Record<string, { input: string; output: string }>;
}

export interface ModelOptionsPayload {
  providers: ModelOptionRow[];
  model: string;
  provider: string;
  catalog_cache?: { providers: number; age_secs: number; fresh: boolean };
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

export interface KanbanDispatchResult {
  dry_run: boolean;
  skipped_locked: number;
  reclaimed: number;
  promoted: number;
  spawned: number;
  would_spawn: number;
  respawn_guarded: number;
  skipped_capped: number;
  skipped_unassigned: number;
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
  deliver?: string | null;
  last_delivery_error?: string | null;
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

export interface InsightsReport {
  days: number;
  source_filter?: string | null;
  empty: boolean;
  generated_at: number;
  overview: {
    total_sessions: number;
    total_messages: number;
    total_tool_calls: number;
    input_tokens: number;
    output_tokens: number;
    total_tokens: number;
    estimated_cost_usd: number;
    cost_known: boolean;
    avg_session_seconds: number;
    active_days: number;
  };
  models: { model: string; sessions: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  sources: { source: string; sessions: number }[];
  tools: { tool: string; calls: number }[];
  top_sessions: { id: string; title: string | null; model: string; started_at: number; messages: number; tool_calls: number; total_tokens: number }[];
  activity: {
    by_hour: number[];
    by_weekday: number[];
    peak_hour: number | null;
    peak_weekday: number | null;
  };
}

/** GET /health/detailed payload (open probe; P365). */
export interface HealthDetailed {
  status: string;
  service: string;
  version: string;
  model: string;
  provider: string;
  auth_required: boolean;
  sessions_total_at_least: number;
  runs_tracked: number;
}

export interface SystemInfo {
  service: string;
  version: string;
  os: string;
  arch: string;
  home: string;
  config_path: string;
  pid: number;
  uptime_secs: number;
  desktop_managed: boolean;
  sessions: number;
  messages: number;
  active_runs: number;
  cron_jobs_enabled: number;
  cron_jobs_disabled: number;
  plugins_loaded: number;
}

export interface PairingPlatform {
  platform: string;
  locked_out: boolean;
  pending: { request_id: string; user_id: string; user_name: string; age_minutes: number }[];
  approved: { user_id: string; user_name: string }[];
}

export interface PluginRow {
  name: string;
  version: string;
  description: string;
  hooks: string[];
  tools: { name: string; description: string }[];
  disabled: boolean;
  dir: string;
}

export interface PluginsPayload {
  plugins: PluginRow[];
  config_hooks: Record<string, string[]>;
  disabled: string[];
}

/** One active dashboard plugin row (`GET /api/dashboard/plugins`; P351). */
export interface DashboardPluginRow {
  name: string;
  version: string;
  description: string;
  source: string;
  enabled: boolean;
  hooks: number;
  tools: number;
  provides: string[];
}

/** One plugin-hub catalog entry (`GET /api/dashboard/plugins/hub`; P351). */
export interface HubCatalogEntry {
  name: string;
  description: string;
  identifier: string;
  version: string;
  homepage: string;
  tags: string[];
  source: string;
  installed: boolean;
}

/** One toolset row from GET /v1/toolsets (P354). */
export interface ToolsetRow {
  name: string;
  description: string;
  enabled: boolean;
  tools: string[];
}

/** Merged plugin-hub payload (hermes `_merged_plugins_hub`; P351). */
export interface PluginsHubPayload {
  agent_plugins: DashboardPluginRow[];
  catalog: HubCatalogEntry[];
  providers: { memory: string[]; context: string[] };
  selected: { memory_provider: string; context_engine: string };
  hidden: string[];
  generated_at: number;
}

export interface SessionSearchHit {
  session_id: string;
  title: string | null;
  snippet: string;
}

/** POST /api/sessions/prune|archive body (P314) — mirrors the CLI
 * `sessions prune/archive` filter flags. */
export interface SessionPruneOptions {
  older_than?: string;
  newer_than?: string;
  before?: string;
  after?: string;
  source?: string;
  title?: string;
  end_reason?: string;
  include_archived?: boolean;
  dry_run?: boolean;
}

export interface SessionPruneCandidate {
  id: string;
  title: string | null;
  source: string;
  model: string | null;
  message_count: number;
  last_active: number;
  archived: boolean;
}

export interface SessionPruneResult {
  dry_run: boolean;
  count?: number;
  affected?: number;
  describe: string;
  candidates?: SessionPruneCandidate[];
}

export interface StorageStats {
  db_path: string;
  size_bytes: number;
  wal_bytes: number;
  sessions: number;
  messages: number;
}

export interface StorageOptimizeResult {
  merged_indexes: number;
  before_bytes: number;
  after_bytes: number;
}

/** One quick-snapshot row from GET /api/backups (P315). */
export interface BackupSnapshot {
  id: string;
  files: number;
  bytes: number;
}

/** GET /api/curator payload (P316) — hermes curator parity. */
export interface CuratorUsageRow {
  name: string;
  provenance: string;
  use_count: number;
  view_count: number;
  patch_count: number;
  activity_count: number;
  last_activity_at: string | null;
  state: string;
  pinned: boolean;
}

export interface CuratorStatus {
  status: { label: string; count: number }[];
  archived: string[];
  usage: CuratorUsageRow[];
}

/** GET /api/checkpoints/status payload (P317). */
export interface CheckpointProject {
  hash: string;
  workdir: string;
  exists: boolean;
  created_at: number;
  last_touch: number;
  commits: number;
}

export interface CheckpointStoreStatus {
  base: string;
  store_size_bytes: number;
  total_size_bytes: number;
  project_count: number;
  projects: CheckpointProject[];
}

export interface CheckpointPruneStats {
  scanned: number;
  deleted_orphan: number;
  deleted_stale: number;
  errors: number;
  bytes_freed: number;
}

/** One GET /api/env row (P320). */
export interface EnvVarInfo {
  key: string;
  in_file: boolean;
  in_process_env: boolean;
}

/** One GET /api/config/schema field row (P336). */
export interface ConfigSchemaField {
  path: string;
  type: string;
  default: unknown;
}

/** One GET /api/messaging/platforms row (P337). */
export interface MessagingPlatformEnvVar {
  key: string;
  required: boolean;
  is_set: boolean;
  redacted_value: string | null;
}

export interface MessagingPlatform {
  id: string;
  name: string;
  description: string;
  docs_url: string;
  enabled: boolean;
  configured: boolean;
  gateway_running: boolean;
  state: string;
  error_code: string | null;
  error_message: string | null;
  env_vars: MessagingPlatformEnvVar[];
}

/** Update-check result from GET /api/update/check (P324). */
export interface UpdateCheckResult {
  install_method: string;
  current_version: string;
  behind: number | null;
  update_available: boolean;
  can_apply: boolean;
  update_command: string;
  log?: string[];
  error?: string;
}

/** Update-apply report from POST /api/update (P324). */
export interface UpdateApplyResult {
  ok: boolean;
  old_sha: string | null;
  new_sha: string | null;
  new_commits: number;
  rebuilt: boolean;
  rebuild_output: string | null;
  log: string[];
}

/** Persistent-memory status from GET /api/memory (P323). */
export interface MemoryStatus {
  active: string;
  providers: { name: string; ready: boolean; description: string }[];
  builtin_files: Record<string, number>;
  files: { file: string; desc: string; exists: boolean; bytes: number; entries: number }[];
  char_limits: { memory: number; user: number };
  dir: string;
}

/** Security audit report from GET /api/ops/security-audit (P321). */
export interface SecurityAuditReport {
  total_components_scanned: number;
  finding_count: number;
  findings: {
    component: { name: string; version: string; ecosystem: string; source: string };
    vuln: { osv_id: string; severity: string; summary: string; fixed_versions: string[] };
  }[];
  note?: string;
}

/** Prompt-size breakdown from GET /api/ops/prompt-size (P321). */
export interface PromptSizeReport {
  model: string;
  provider: string;
  system_prompt_chars: number;
  system_prompt_bytes: number;
  sections: { label: string; chars: number; bytes: number }[];
  memory_file_bytes: number;
  user_profile_file_bytes: number;
  tools_count: number;
  tools_json_bytes: number;
  toolsets: { toolset: string; tools: number; json_bytes: number }[];
  skills: { name: string; skill_md_bytes: number }[];
  skills_total_bytes: number;
}

export interface McpServerRow {
  name: string;
  kind: "stdio" | "http" | "sse";
  target: string;
  auth: "none" | "headers" | "oauth";
  oauth_tokens: boolean;
  cached_tools?: { name: string; description: string }[];
}

export interface McpOAuthFlow {
  flow_id: string;
  server_name: string;
  status: "starting" | "authorization_required" | "approved" | "error";
  authorization_url: string | null;
  error: string | null;
  tools?: { name: string; description?: string | null }[];
}

/** Filesystem entry from GET /api/fs/list (P329). */
export interface FsEntry {
  name: string;
  path: string;
  isDirectory: boolean;
}

/** GET /api/fs/read-text payload (P347). */
export interface FsReadText {
  binary: boolean;
  byteSize: number;
  language: string;
  mimeType: string;
  path: string;
  text: string;
  truncated: boolean;
}

/** GET /api/checkpoints list item (P347). */
export interface CheckpointEntry {
  hash: string;
  short_hash: string;
  timestamp: string;
  reason: string;
  files_changed: number;
  insertions: number;
  deletions: number;
}

/** OAuth device-flow posture from GET /api/oauth/status (P334). */
export interface OAuthStatus {
  logged_in: boolean;
  expired: boolean;
  provider: string;
  portal_url: string;
  scopes: string;
  expires_at: number;
  token_preview: string;
}

/** One OAuth-capable provider row from GET /api/providers/oauth (P350). */
export interface ProviderOAuthRow {
  id: string;
  name: string;
  flow: "device_code" | "pkce" | "external" | string;
  cli_command: string;
  docs_url: string;
  configured: boolean;
  disconnectable: boolean;
  status: {
    logged_in: boolean;
    expired: boolean;
    source: string;
    source_label: string;
    token_preview: string;
    expires_at: number;
    has_refresh_token: boolean;
  };
}

/** POST /api/providers/oauth/:id/start response (P350). */
export interface ProviderOAuthStart {
  session_id: string;
  status: string;
  verification_uri: string;
  user_code: string;
  expires_in: number;
  interval: number;
}

/** GET /api/providers/oauth/:id/poll/:session response (P350). */
export interface ProviderOAuthPoll {
  session_id: string;
  status: "pending" | "complete" | "error" | string;
  logged_in: boolean;
  error: string | null;
}

/** Custom provider endpoint row from GET /api/providers/custom-endpoints (P333). */
export interface CustomEndpoint {
  id: string;
  base_url: string;
  model: string;
  mode: string;
  key_state: "literal" | "env" | "missing";
}

/** Validate probe result (P333). */
export interface CustomEndpointValidation {
  ok: boolean;
  reachable: boolean;
  message: string;
  models: string[];
}

/** Resolved gateway model metadata from GET /api/model/info (P332). */
export interface ModelInfoPayload {
  provider: string;
  model: string;
  base_url: string;
  known: boolean;
  context: { auto: number; config: number; effective: number };
  capabilities: { vision: boolean; reasoning: boolean; tools: boolean } | null;
}

/** Dashboard theme row from GET /api/dashboard/themes (P331). */
export interface DashboardTheme {
  name: string;
  label: string;
  description: string;
}

/** Credential pool entry from GET /api/credentials/pool (P330). */
export interface CredentialPoolEntry {
  index: number;
  id: string;
  label: string;
  auth_type: string;
  priority: number;
  source: string;
  request_count: number;
  token_preview: string;
}

/** One provider's pooled credentials (P330). */
export interface CredentialPoolProvider {
  provider: string;
  entries: CredentialPoolEntry[];
}

/** Per-model usage row from GET /api/analytics/models (P328). */
export interface ModelUsageRow {
  model: string;
  sessions: number;
  messages: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  last_used_at: number;
}

/** Shell-hook consent census from GET /api/ops/hooks (P326). */
export interface HooksConsentPayload {
  hooks: { event: string; command: string; known: boolean; consented: boolean; state: string }[];
  valid_events: string[];
  auto_accept: boolean;
  allowlist: { path: string; entries: number };
}

/** Log-file inventory row from GET /api/logs (P325). */
export interface LogFileInfo {
  name: string;
  file: string;
  path: string;
  bytes: number;
  modified: number | null;
  exists: boolean;
}

export interface LogsTailPayload {
  path: string;
  lines: string[];
}

export interface BrowserStatus {
  configured: boolean;
  backend?: string;
  source?: string;
  endpoint?: string | null;
  mode?: string;
  managed_running?: boolean;
  available?: boolean;
  vnc_url?: string | null;
}

export interface DelegationRow {
  id: string;
  status: string;
  tasks?: number;
  parent_session_key: string;
  created_ms: number;
  finished_ms: number | null;
  log_dir: string;
  delivery_attempts?: number;
  persisted?: boolean;
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

export interface JobDeliveryTarget {
  id: string;
  name: string;
  home_target_set?: boolean;
  home_env_var?: string | null;
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

  async messages(sessionId: string, options?: { timestamps?: boolean }): Promise<MessageRow[]> {
    const suffix = options?.timestamps ? "?timestamps=true" : "";
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/messages${suffix}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`messages: HTTP ${response.status}`);
    const value = await response.json();
    return (value.data || value.messages || value || []) as MessageRow[];
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

  /**
   * Subscribe to a run's lifecycle SSE stream (/v1/runs/:id/events, P322).
   * Fetch-based reader (EventSource can't carry the bearer header). Invokes
   * onEvent with the SSE event name + fresh RunRow per status transition;
   * resolves when the stream closes (run reached a terminal state).
   */
  async runEvents(
    runId: string,
    onEvent: (name: string, run: RunRow) => void,
    signal: AbortSignal,
  ): Promise<void> {
    const response = await fetch(this.endpoint(`/v1/runs/${encodeURIComponent(runId)}/events`), {
      headers: this.headers(),
      signal,
    });
    if (!response.ok || !response.body) return;
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let eventName = "run.progress";
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
        if (!payload) continue;
        try {
          const run = JSON.parse(payload) as RunRow;
          if (run && typeof run.run_id === "string") onEvent(eventName, run);
        } catch {
          // ignore malformed frames
        }
        eventName = "run.progress";
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

  /** GET /api/insights — usage analytics over the session store. */
  async insights(days = 30, source?: string): Promise<InsightsReport> {
    const params = new URLSearchParams({ days: String(days) });
    if (source && source.trim()) params.set("source", source.trim());
    const response = await fetch(this.endpoint(`/api/insights?${params.toString()}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`insights HTTP ${response.status}`);
    return (await response.json()) as InsightsReport;
  }

  /** GET /api/mcp/servers — configured MCP servers + auth posture. */
  async mcpServers(): Promise<McpServerRow[]> {
    const response = await fetch(this.endpoint("/api/mcp/servers"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`mcp servers HTTP ${response.status}`);
    const value = await response.json();
    return (value.servers || []) as McpServerRow[];
  }

  /** GET /metrics — raw Prometheus text exposition. */
  async metricsRaw(): Promise<string> {
    const response = await fetch(this.endpoint("/metrics"), { headers: this.headers() });
    if (!response.ok) throw new Error(`metrics HTTP ${response.status}`);
    return response.text();
  }

  /** GET /api/channels — messaging-platform enabled posture. */
  async channels(): Promise<{ name: string; enabled: boolean }[]> {
    const response = await fetch(this.endpoint("/api/channels"), { headers: this.headers() });
    if (!response.ok) throw new Error(`channels HTTP ${response.status}`);
    const value = await response.json();
    return (value.channels || []) as { name: string; enabled: boolean }[];
  }

  /** GET /api/egress/status — egress-proxy status text (tokens redacted). */
  async egressStatus(): Promise<string> {
    const response = await fetch(this.endpoint("/api/egress/status"), { headers: this.headers() });
    if (!response.ok) throw new Error(`egress HTTP ${response.status}`);
    const value = await response.json();
    return typeof value.text === "string" ? value.text : "";
  }

  /** GET /health/detailed — open detailed health probe (P365). */
  async healthDetailed(): Promise<HealthDetailed | null> {
    try {
      const response = await fetch(this.endpoint("/health/detailed"), { headers: this.headers() });
      if (!response.ok) return null;
      return (await response.json()) as HealthDetailed;
    } catch {
      return null;
    }
  }

  /** GET /api/system — gateway/system facts for the Doctor panel. */
  async systemInfo(): Promise<SystemInfo> {
    const response = await fetch(this.endpoint("/api/system"), { headers: this.headers() });
    if (!response.ok) throw new Error(`system HTTP ${response.status}`);
    return (await response.json()) as SystemInfo;
  }

  /** GET /api/pairing — pending/approved pairings per platform. */
  async pairingStatus(): Promise<PairingPlatform[]> {
    const response = await fetch(this.endpoint("/api/pairing"), { headers: this.headers() });
    if (!response.ok) throw new Error(`pairing HTTP ${response.status}`);
    const value = await response.json();
    return (value.platforms || []) as PairingPlatform[];
  }

  /** POST /api/pairing/approve — approve a pending code/request id. */
  async pairingApprove(platform: string, code: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/pairing/approve"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ platform, code }),
    });
    if (!response.ok) {
      const value = await response.json().catch(() => ({}));
      throw new Error(value.error || `approve HTTP ${response.status}`);
    }
  }

  /** POST /api/pairing/revoke — revoke an approved pairing. */
  async pairingRevoke(platform: string, userId: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/pairing/revoke"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ platform, user_id: userId }),
    });
    if (!response.ok) {
      const value = await response.json().catch(() => ({}));
      throw new Error(value.error || `revoke HTTP ${response.status}`);
    }
  }

  /** POST /api/pairing/clear-pending — drop pending codes. */
  async pairingClearPending(platform?: string): Promise<number> {
    const response = await fetch(this.endpoint("/api/pairing/clear-pending"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(platform ? { platform } : {}),
    });
    if (!response.ok) throw new Error(`clear-pending HTTP ${response.status}`);
    const value = await response.json();
    return Number(value.cleared || 0);
  }

  /** GET /api/plugins — plugin inventory (manifests, hooks, deny-list). */
  async pluginsInventory(): Promise<PluginsPayload> {
    const response = await fetch(this.endpoint("/api/plugins"), { headers: this.headers() });
    if (!response.ok) throw new Error(`plugins HTTP ${response.status}`);
    return (await response.json()) as PluginsPayload;
  }

  /** POST /api/plugins/:name/enable — remove from the config deny-list. */
  async pluginEnable(name: string): Promise<string> {
    const response = await fetch(
      this.endpoint(`/api/plugins/${encodeURIComponent(name)}/enable`),
      { method: "POST", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `enable HTTP ${response.status}`);
    return value.message || "";
  }

  /** POST /api/plugins/:name/disable — add to the config deny-list. */
  async pluginDisable(name: string): Promise<string> {
    const response = await fetch(
      this.endpoint(`/api/plugins/${encodeURIComponent(name)}/disable`),
      { method: "POST", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `disable HTTP ${response.status}`);
    return value.message || "";
  }

  /** GET /api/dashboard/plugins — active dashboard plugins (P351). */
  async dashboardPlugins(): Promise<DashboardPluginRow[]> {
    const response = await fetch(this.endpoint("/api/dashboard/plugins"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`dashboard plugins HTTP ${response.status}`);
    return (await response.json()) as DashboardPluginRow[];
  }

  /** GET /api/dashboard/plugins/rescan — force discovery (P351). */
  async dashboardPluginsRescan(): Promise<number> {
    const response = await fetch(this.endpoint("/api/dashboard/plugins/rescan"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`rescan HTTP ${response.status}`);
    const value = await response.json();
    return Number(value.count || 0);
  }

  /** GET /api/dashboard/plugins/hub — merged hub payload (P351). */
  async pluginsHub(): Promise<PluginsHubPayload> {
    const response = await fetch(this.endpoint("/api/dashboard/plugins/hub"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`plugins hub HTTP ${response.status}`);
    return (await response.json()) as PluginsHubPayload;
  }

  /** POST /api/dashboard/agent-plugins/install — hub install (P351). */
  async agentPluginInstall(identifier: string, force = false, enable = true): Promise<string> {
    const response = await fetch(this.endpoint("/api/dashboard/agent-plugins/install"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ identifier, force, enable }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `install HTTP ${response.status}`);
    }
    return value.name || "";
  }

  /** POST /api/dashboard/agent-plugins/:name/update — git pull (P351). */
  async agentPluginUpdate(name: string): Promise<string> {
    const response = await fetch(
      this.endpoint(`/api/dashboard/agent-plugins/${encodeURIComponent(name)}/update`),
      { method: "POST", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `update HTTP ${response.status}`);
    }
    return value.output || "";
  }

  /** DELETE /api/dashboard/agent-plugins/:name — remove (P351). */
  async agentPluginRemove(name: string): Promise<string> {
    const response = await fetch(
      this.endpoint(`/api/dashboard/agent-plugins/${encodeURIComponent(name)}`),
      { method: "DELETE", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `remove HTTP ${response.status}`);
    }
    return value.message || "";
  }

  /** PUT /api/dashboard/plugin-providers — persist selection (P351). */
  async setPluginProviders(memory?: string, context?: string): Promise<void> {
    const body: Record<string, string> = {};
    if (memory) body.memory_provider = memory;
    if (context) body.context_engine = context;
    const response = await fetch(this.endpoint("/api/dashboard/plugin-providers"), {
      method: "PUT",
      headers: this.headers(),
      body: JSON.stringify(body),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `plugin-providers HTTP ${response.status}`);
    }
  }

  /** POST /api/dashboard/plugins/:name/visibility — hide/show (P351). */
  async setPluginVisibility(name: string, hidden: boolean): Promise<void> {
    const response = await fetch(
      this.endpoint(`/api/dashboard/plugins/${encodeURIComponent(name)}/visibility`),
      { method: "POST", headers: this.headers(), body: JSON.stringify({ hidden }) },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `visibility HTTP ${response.status}`);
    }
  }

  /** GET /v1/toolsets — tool groups with enablement posture (P354). */
  async toolsetsList(): Promise<ToolsetRow[]> {
    const response = await fetch(this.endpoint("/v1/toolsets"), { headers: this.headers() });
    if (!response.ok) throw new Error(`toolsets HTTP ${response.status}`);
    const value = await response.json();
    return (value.data || []) as ToolsetRow[];
  }

  /** GET /api/storage — session-store footprint for the Doctor panel. */
  async storageStats(): Promise<StorageStats> {
    const response = await fetch(this.endpoint("/api/storage"), { headers: this.headers() });
    if (!response.ok) throw new Error(`storage HTTP ${response.status}`);
    return (await response.json()) as StorageStats;
  }

  /** POST /api/storage/optimize — FTS merge + VACUUM the session store. */
  async storageOptimize(): Promise<StorageOptimizeResult> {
    const response = await fetch(this.endpoint("/api/storage/optimize"), {
      method: "POST",
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`optimize HTTP ${response.status}`);
    return (await response.json()) as StorageOptimizeResult;
  }

  /** POST /api/mcp/servers/:name/auth — start an OAuth flow for an MCP server. */
  async mcpAuth(name: string): Promise<McpOAuthFlow> {
    const response = await fetch(
      this.endpoint(`/api/mcp/servers/${encodeURIComponent(name)}/auth`),
      { method: "POST", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.detail || `mcp auth HTTP ${response.status}`);
    return value as McpOAuthFlow;
  }

  /** GET /api/mcp/oauth/flows/:flowId — poll OAuth flow status. */
  async mcpFlowStatus(flowId: string): Promise<McpOAuthFlow> {
    const response = await fetch(
      this.endpoint(`/api/mcp/oauth/flows/${encodeURIComponent(flowId)}`),
      { headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.detail || `mcp flow HTTP ${response.status}`);
    return value as McpOAuthFlow;
  }

  /** GET /api/logs/tail — tail of gateway.log with optional min level. */
  /** GET /api/ops/hooks — shell-hook consent census (P326). */
  async hooksConsent(): Promise<HooksConsentPayload> {
    const response = await fetch(this.endpoint("/api/ops/hooks"), { headers: this.headers() });
    if (!response.ok) throw new Error(`hooks HTTP ${response.status}`);
    return (await response.json()) as HooksConsentPayload;
  }

  /** POST /api/ops/hooks/accept-all — consent to every configured hook (P326). */
  async hooksAcceptAll(): Promise<number> {
    const response = await fetch(this.endpoint("/api/ops/hooks/accept-all"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: "{}",
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `hooks accept HTTP ${response.status}`);
    return typeof value.accepted === "number" ? value.accepted : 0;
  }

  /** POST /api/ops/hooks/revoke — revoke consent for a hook command (P326). */
  async hooksRevoke(command: string): Promise<number> {
    const response = await fetch(this.endpoint("/api/ops/hooks/revoke"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ command }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `hooks revoke HTTP ${response.status}`);
    return typeof value.removed === "number" ? value.removed : 0;
  }

  /** GET /api/logs — log-file inventory (P325). */
  async logsList(): Promise<{ dir: string; files: LogFileInfo[] }> {
    const response = await fetch(this.endpoint("/api/logs"), { headers: this.headers() });
    if (!response.ok) throw new Error(`logs HTTP ${response.status}`);
    return (await response.json()) as { dir: string; files: LogFileInfo[] };
  }

  /** GET /api/logs?file=… — filtered tail of one log file (P325). */
  async logsFile(
    file: string,
    opts?: { lines?: number; level?: string; search?: string },
  ): Promise<{ file: string; path: string; lines: string[] }> {
    const params = new URLSearchParams({ file });
    if (opts?.lines) params.set("lines", String(opts.lines));
    if (opts?.level) params.set("level", opts.level);
    if (opts?.search) params.set("search", opts.search);
    const response = await fetch(this.endpoint(`/api/logs?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `logs HTTP ${response.status}`);
    return value as { file: string; path: string; lines: string[] };
  }

  async logsTail(lines = 200, level?: string): Promise<LogsTailPayload> {
    const params = new URLSearchParams({ lines: String(lines) });
    if (level) params.set("level", level);
    const response = await fetch(this.endpoint(`/api/logs/tail?${params.toString()}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`logs HTTP ${response.status}`);
    return (await response.json()) as LogsTailPayload;
  }

  /** GET /v1/browser/status — CDP browser configuration state. */
  async browserStatus(): Promise<BrowserStatus> {
    const response = await fetch(this.endpoint("/v1/browser/status"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`browser status HTTP ${response.status}`);
    return (await response.json()) as BrowserStatus;
  }

  /** POST /v1/browser/connect — point browser tools at a CDP endpoint
   * for the process lifetime (hermes `/browser connect`; P352). */
  async browserConnect(url: string): Promise<string> {
    const response = await fetch(this.endpoint("/v1/browser/connect"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ url }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error?.message || `browser connect HTTP ${response.status}`);
    }
    return value.endpoint || url;
  }

  /** POST /v1/browser/disconnect — clear the live CDP override
   * (hermes `/browser disconnect`; P352). */
  async browserDisconnect(): Promise<void> {
    const response = await fetch(this.endpoint("/v1/browser/disconnect"), {
      method: "POST",
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`browser disconnect HTTP ${response.status}`);
  }

  /** GET /v1/delegations — async delegations (live + persisted history). */
  async listDelegations(): Promise<DelegationRow[]> {
    const response = await fetch(this.endpoint("/v1/delegations"), { headers: this.headers() });
    if (!response.ok) throw new Error(`delegations HTTP ${response.status}`);
    const value = await response.json();
    return (value.delegations || []) as DelegationRow[];
  }

  /** GET /v1/delegations/:id — delegation record + consolidated result. */
  async delegationDetail(id: string): Promise<{ result?: unknown } & DelegationRow> {
    const response = await fetch(this.endpoint(`/v1/delegations/${encodeURIComponent(id)}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`delegation HTTP ${response.status}`);
    return (await response.json()) as { result?: unknown } & DelegationRow;
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

  /** GET /api/sessions/search?q=... — full-text transcript search. */
  async searchSessions(query: string, limit = 30): Promise<SessionSearchHit[]> {
    const params = new URLSearchParams({ q: query, limit: String(limit) });
    const response = await fetch(this.endpoint(`/api/sessions/search?${params.toString()}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`search HTTP ${response.status}`);
    const value = await response.json();
    return (value.results || []) as SessionSearchHit[];
  }

  /** GET /api/env — env-key posture, values never returned (P320). */
  async envList(): Promise<{ path: string; vars: EnvVarInfo[] }> {
    const response = await fetch(this.endpoint("/api/env"), { headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `env HTTP ${response.status}`);
    return value as { path: string; vars: EnvVarInfo[] };
  }

  /** PUT /api/env — set an ALL_CAPS key in .env. */
  async envSet(key: string, value: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/env"), {
      method: "PUT",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ key, value }),
    });
    const value2 = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value2.error || `env set HTTP ${response.status}`);
  }

  /** DELETE /api/env — remove a key's line from .env. */
  async envDelete(key: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/env"), {
      method: "DELETE",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ key }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `env delete HTTP ${response.status}`);
  }

  /** POST /api/env/reveal — unredacted value for one key (P336).
   * Rate-limited server-side: 5 reveals per 30 s window. */
  async envReveal(key: string): Promise<string> {
    const response = await fetch(this.endpoint("/api/env/reveal"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ key }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `env reveal HTTP ${response.status}`);
    return String(value.value ?? "");
  }

  /** GET /api/messaging/platforms — catalog with posture (P337). */
  async messagingPlatforms(): Promise<MessagingPlatform[]> {
    const response = await fetch(this.endpoint("/api/messaging/platforms"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `messaging platforms HTTP ${response.status}`);
    return (value.platforms || []) as MessagingPlatform[];
  }

  /** PUT /api/messaging/platforms/:id — enable toggle + env set/clear
   * (P337). */
  async messagingPlatformUpdate(
    id: string,
    body: { enabled?: boolean; env?: Record<string, string>; clear_env?: string[] },
  ): Promise<void> {
    const response = await fetch(
      this.endpoint(`/api/messaging/platforms/${encodeURIComponent(id)}`),
      {
        method: "PUT",
        headers: { ...this.headers(), "content-type": "application/json" },
        body: JSON.stringify(body),
      },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error || `messaging platform update HTTP ${response.status}`);
    }
  }

  /** POST /api/messaging/platforms/:id/test — posture probe (P337). */
  async messagingPlatformTest(
    id: string,
  ): Promise<{ ok: boolean; state: string; message: string }> {
    const response = await fetch(
      this.endpoint(`/api/messaging/platforms/${encodeURIComponent(id)}/test`),
      {
        method: "POST",
        headers: { ...this.headers(), "content-type": "application/json" },
        body: "{}",
      },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(value.error || `messaging platform test HTTP ${response.status}`);
    }
    return value as { ok: boolean; state: string; message: string };
  }

  /** GET /api/config/defaults — full default config as JSON (P336). */
  async configDefaults(): Promise<Record<string, unknown>> {
    const response = await fetch(this.endpoint("/api/config/defaults"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `config defaults HTTP ${response.status}`);
    return (value.defaults || {}) as Record<string, unknown>;
  }

  /** GET /api/config/schema — flattened dotted-path fields with type +
   * default (P336). */
  async configSchema(): Promise<ConfigSchemaField[]> {
    const response = await fetch(this.endpoint("/api/config/schema"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `config schema HTTP ${response.status}`);
    return (value.fields || []) as ConfigSchemaField[];
  }

  /** GET /api/fs/list — directory listing (P329). */
  async fsList(path: string): Promise<{ entries: FsEntry[]; error?: string }> {
    const params = new URLSearchParams({ path });
    const response = await fetch(this.endpoint(`/api/fs/list?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
    return value as { entries: FsEntry[]; error?: string };
  }

  /** GET /api/fs/default-cwd — gateway cwd + branch (P329). */
  async fsDefaultCwd(): Promise<{ cwd: string; branch: string }> {
    const response = await fetch(this.endpoint("/api/fs/default-cwd"), { headers: this.headers() });
    if (!response.ok) throw new Error(`fs HTTP ${response.status}`);
    return (await response.json()) as { cwd: string; branch: string };
  }

  /** Build a download URL for browser opening (?token= auth, P335). */
  fsDownloadUrl(path: string): string {
    const params = new URLSearchParams({ path });
    if (this.settings.key) params.set("token", this.settings.key);
    return this.endpoint(`/api/fs/download?${params.toString()}`);
  }

  /** POST /api/fs/mkdir — create a directory (P335). */
  async fsMkdir(path: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/fs/mkdir"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ path }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
  }

  /** GET /api/fs/git-root — nearest enclosing git checkout (P347). */
  async fsGitRoot(path: string): Promise<{ root: string | null }> {
    const params = new URLSearchParams({ path });
    const response = await fetch(this.endpoint(`/api/fs/git-root?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
    return value as { root: string | null };
  }

  /** GET /api/fs/read-text — capped UTF-8 preview with language hint (P347). */
  async fsReadText(path: string): Promise<FsReadText> {
    const params = new URLSearchParams({ path });
    const response = await fetch(this.endpoint(`/api/fs/read-text?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
    return value as FsReadText;
  }

  /** POST /api/fs/write-text — atomic text-file overwrite/create (P347). */
  async fsWriteText(path: string, content: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/fs/write-text"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ path, content }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
  }

  /** POST /api/audio/speak — synthesize speech over the configured
   * [tts] provider, returns a playable base64 data URL (P344). */
  async audioSpeak(text: string): Promise<string> {
    const response = await fetch(this.endpoint("/api/audio/speak"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ text }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message =
        typeof value.error === "string"
          ? value.error
          : value.error?.message || `speak HTTP ${response.status}`;
      throw new Error(message);
    }
    return String(value.data_url ?? "");
  }

  /** GET /api/audio/elevenlabs/voices — voice picker metadata (P344). */
  async elevenlabsVoices(): Promise<{
    available: boolean;
    voices: { voice_id: string; name: string; label: string }[];
    error?: string;
  }> {
    const response = await fetch(this.endpoint("/api/audio/elevenlabs/voices"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `voices HTTP ${response.status}`);
    return value as {
      available: boolean;
      voices: { voice_id: string; name: string; label: string }[];
      error?: string;
    };
  }

  /** GET /api/media — gateway-host image as base64 data URL (hermes
   * `/api/media` parity; P338). Image extensions only, confined to the
   * gateway's media roots, 25 MiB cap. */
  async mediaDataUrl(path: string): Promise<string> {
    const params = new URLSearchParams({ path });
    const response = await fetch(this.endpoint(`/api/media?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `media HTTP ${response.status}`);
    return String(value.data_url ?? "");
  }

  /** GET /api/fs/read-data-url — small file as base64 data URL (P329). */
  async fsReadDataUrl(path: string): Promise<string> {
    const params = new URLSearchParams({ path });
    const response = await fetch(this.endpoint(`/api/fs/read-data-url?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `fs HTTP ${response.status}`);
    return typeof value.dataUrl === "string" ? value.dataUrl : "";
  }

  /** GET /api/oauth/status — device-flow auth posture (P334). */
  async oauthStatus(): Promise<OAuthStatus> {
    const response = await fetch(this.endpoint("/api/oauth/status"), { headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `oauth HTTP ${response.status}`);
    return value as OAuthStatus;
  }

  /** GET /api/providers/oauth — OAuth-capable provider catalog (P350). */
  async providersOAuth(): Promise<{ providers: ProviderOAuthRow[] }> {
    const response = await fetch(this.endpoint("/api/providers/oauth"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `providers oauth HTTP ${response.status}`);
    return value as { providers: ProviderOAuthRow[] };
  }

  /** POST /api/providers/oauth/:id/start — begin a device-flow session (P350). */
  async providersOAuthStart(providerId: string): Promise<ProviderOAuthStart> {
    const response = await fetch(
      this.endpoint(`/api/providers/oauth/${encodeURIComponent(providerId)}/start`),
      { method: "POST", headers: { ...this.headers(), "content-type": "application/json" }, body: "{}" },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message =
        typeof value.error === "string"
          ? value.error
          : value.error?.message || `oauth start HTTP ${response.status}`;
      throw new Error(message);
    }
    return value as ProviderOAuthStart;
  }

  /** GET /api/providers/oauth/:id/poll/:session — session posture (P350). */
  async providersOAuthPoll(
    providerId: string,
    sessionId: string,
  ): Promise<ProviderOAuthPoll> {
    const response = await fetch(
      this.endpoint(
        `/api/providers/oauth/${encodeURIComponent(providerId)}/poll/${encodeURIComponent(sessionId)}`,
      ),
      { headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message =
        typeof value.error === "string"
          ? value.error
          : value.error?.message || `oauth poll HTTP ${response.status}`;
      throw new Error(message);
    }
    return value as ProviderOAuthPoll;
  }

  /** DELETE /api/providers/oauth/:id — disconnect (clear tokens) (P350). */
  async providersOAuthDisconnect(providerId: string): Promise<void> {
    const response = await fetch(
      this.endpoint(`/api/providers/oauth/${encodeURIComponent(providerId)}`),
      { method: "DELETE", headers: this.headers() },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `oauth disconnect HTTP ${response.status}`);
  }

  /** GET /api/providers/custom-endpoints — custom provider rows (P333). */
  async customEndpoints(): Promise<{ endpoints: CustomEndpoint[] }> {
    const response = await fetch(this.endpoint("/api/providers/custom-endpoints"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `endpoints HTTP ${response.status}`);
    return value as { endpoints: CustomEndpoint[] };
  }

  /** POST /api/providers/custom-endpoints — upsert an endpoint (P333). */
  async customEndpointsUpsert(payload: {
    id: string;
    base_url: string;
    model?: string;
    mode?: string;
    api_key?: string;
  }): Promise<void> {
    const response = await fetch(this.endpoint("/api/providers/custom-endpoints"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(payload),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `endpoints HTTP ${response.status}`);
  }

  /** POST /api/providers/custom-endpoints/validate — probe /models (P333). */
  async customEndpointsValidate(payload: {
    base_url: string;
    api_key?: string;
  }): Promise<CustomEndpointValidation> {
    const response = await fetch(this.endpoint("/api/providers/custom-endpoints/validate"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify(payload),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `endpoints HTTP ${response.status}`);
    return value as CustomEndpointValidation;
  }

  /** POST /api/providers/custom-endpoints/:id/activate (P333). */
  async customEndpointsActivate(id: string): Promise<void> {
    const uri = `/api/providers/custom-endpoints/${encodeURIComponent(id)}/activate`;
    const response = await fetch(this.endpoint(uri), { method: "POST", headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `endpoints HTTP ${response.status}`);
  }

  /** DELETE /api/providers/custom-endpoints/:id (P333). */
  async customEndpointsDelete(id: string): Promise<void> {
    const uri = `/api/providers/custom-endpoints/${encodeURIComponent(id)}`;
    const response = await fetch(this.endpoint(uri), { method: "DELETE", headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `endpoints HTTP ${response.status}`);
  }

  /** GET /api/model/info — resolved gateway model metadata (P332). */
  async modelInfo(): Promise<ModelInfoPayload> {
    const response = await fetch(this.endpoint("/api/model/info"), { headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `model HTTP ${response.status}`);
    return value as ModelInfoPayload;
  }

  /** POST /api/model/set — persist the gateway provider/model (P332). */
  async modelSet(provider: string, model: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/model/set"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ provider, model }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `model HTTP ${response.status}`);
  }

  /** GET /api/model/recommended-default — sensible default model for a
   * provider (first curated catalog entry; P347). */
  async modelRecommendedDefault(provider?: string): Promise<{ provider: string; model: string }> {
    const params = provider ? `?${new URLSearchParams({ provider }).toString()}` : "";
    const response = await fetch(this.endpoint(`/api/model/recommended-default${params}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `model HTTP ${response.status}`);
    return value as { provider: string; model: string };
  }

  /** GET /api/dashboard/themes — theme catalog + active theme (P331). */
  async dashboardThemes(): Promise<{ themes: DashboardTheme[]; active: string }> {
    const response = await fetch(this.endpoint("/api/dashboard/themes"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `themes HTTP ${response.status}`);
    return value as { themes: DashboardTheme[]; active: string };
  }

  /** PUT /api/dashboard/theme — persist the active theme (P331). */
  async dashboardSetTheme(name: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/dashboard/theme"), {
      method: "PUT",
      headers: this.headers(),
      body: JSON.stringify({ name }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `themes HTTP ${response.status}`);
  }

  /** GET /api/dashboard/font — active font override (P331). */
  async dashboardFont(): Promise<string> {
    const response = await fetch(this.endpoint("/api/dashboard/font"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `font HTTP ${response.status}`);
    return typeof value.font === "string" ? value.font : "theme";
  }

  /** PUT /api/dashboard/font — persist the font override (P331). */
  async dashboardSetFont(font: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/dashboard/font"), {
      method: "PUT",
      headers: this.headers(),
      body: JSON.stringify({ font }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `font HTTP ${response.status}`);
  }

  /** GET /api/credentials/pool — pooled provider credentials (P330). */
  async credentialsPool(): Promise<{ providers: CredentialPoolProvider[] }> {
    const response = await fetch(this.endpoint("/api/credentials/pool"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `credentials HTTP ${response.status}`);
    return value as { providers: CredentialPoolProvider[] };
  }

  /** POST /api/credentials/pool — add a pooled key (P330). */
  async credentialsPoolAdd(
    provider: string,
    apiKey: string,
    label?: string,
  ): Promise<{ ok: boolean; provider: string; count: number }> {
    const response = await fetch(this.endpoint("/api/credentials/pool"), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({ provider, api_key: apiKey, label: label || undefined }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `credentials HTTP ${response.status}`);
    return value as { ok: boolean; provider: string; count: number };
  }

  /** DELETE /api/credentials/pool/:provider/:index — remove entry (P330). */
  async credentialsPoolRemove(
    provider: string,
    index: number,
  ): Promise<{ ok: boolean; provider: string; count: number }> {
    const uri = `/api/credentials/pool/${encodeURIComponent(provider)}/${index}`;
    const response = await fetch(this.endpoint(uri), { method: "DELETE", headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `credentials HTTP ${response.status}`);
    return value as { ok: boolean; provider: string; count: number };
  }

  /** GET /api/analytics/models — per-model usage aggregation (P328). */
  async analyticsModels(days = 30): Promise<{ days: number; models: ModelUsageRow[] }> {
    const response = await fetch(this.endpoint(`/api/analytics/models?days=${days}`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`analytics HTTP ${response.status}`);
    return (await response.json()) as { days: number; models: ModelUsageRow[] };
  }

  /** POST /api/audio/transcribe — voice-note transcription (P327). */
  async audioTranscribe(
    dataUrl: string,
    mimeType: string,
  ): Promise<{ ok: boolean; transcript: string; provider: string }> {
    const response = await fetch(this.endpoint("/api/audio/transcribe"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ data_url: dataUrl, mime_type: mimeType }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `transcribe HTTP ${response.status}`);
    return value as { ok: boolean; transcript: string; provider: string };
  }

  /** GET /api/update/check — non-applying update check (P324). */
  async updateCheck(): Promise<UpdateCheckResult> {
    const response = await fetch(this.endpoint("/api/update/check"), { headers: this.headers() });
    if (!response.ok) throw new Error(`update check HTTP ${response.status}`);
    return (await response.json()) as UpdateCheckResult;
  }

  /** POST /api/update — apply the pending update in place (P324). */
  async updateApply(): Promise<UpdateApplyResult> {
    const response = await fetch(this.endpoint("/api/update"), {
      method: "POST",
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `update HTTP ${response.status}`);
    return value as UpdateApplyResult;
  }

  /** GET /api/memory — persistent-memory status census (P323). */
  async memoryStatus(): Promise<MemoryStatus> {
    const response = await fetch(this.endpoint("/api/memory"), { headers: this.headers() });
    if (!response.ok) throw new Error(`memory HTTP ${response.status}`);
    return (await response.json()) as MemoryStatus;
  }

  /** POST /api/memory/reset — erase memory stores; returns deleted files (P323). */
  async memoryReset(target: "all" | "memory" | "user"): Promise<string[]> {
    const response = await fetch(this.endpoint("/api/memory/reset"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ target }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `memory reset HTTP ${response.status}`);
    return Array.isArray(value.deleted) ? value.deleted : [];
  }

  /** GET /api/ops/security-audit — OSV audit of pinned MCP components (P321). */
  async opsSecurityAudit(): Promise<SecurityAuditReport> {
    const response = await fetch(this.endpoint("/api/ops/security-audit"), { headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `security-audit HTTP ${response.status}`);
    return value as SecurityAuditReport;
  }

  /** GET /api/ops/prompt-size — system-prompt token breakdown (P321). */
  async opsPromptSize(): Promise<PromptSizeReport> {
    const response = await fetch(this.endpoint("/api/ops/prompt-size"), { headers: this.headers() });
    if (!response.ok) throw new Error(`prompt-size HTTP ${response.status}`);
    return (await response.json()) as PromptSizeReport;
  }

  /** GET /api/ops/dump — redacted diagnostic dump text (P321). */
  async opsDump(): Promise<string> {
    const response = await fetch(this.endpoint("/api/ops/dump"), { headers: this.headers() });
    if (!response.ok) throw new Error(`dump HTTP ${response.status}`);
    const value = await response.json();
    return typeof value.text === "string" ? value.text : "";
  }

  /** GET /api/config/raw — raw config.toml text with comments (P318). */
  async configRaw(): Promise<{ toml: string; path: string }> {
    const response = await fetch(this.endpoint("/api/config/raw"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `config raw HTTP ${response.status}`);
    return value as { toml: string; path: string };
  }

  /** PUT /api/config/raw — validate + atomically replace config.toml. */
  async saveConfigRaw(tomlText: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/config/raw"), {
      method: "PUT",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ toml_text: tomlText }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `config raw save HTTP ${response.status}`);
  }

  /** GET /api/checkpoints/status — checkpoint store census (P317). */
  async checkpointsStatus(): Promise<CheckpointStoreStatus> {
    const response = await fetch(this.endpoint("/api/checkpoints/status"), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `checkpoints HTTP ${response.status}`);
    return value.status as CheckpointStoreStatus;
  }

  /** POST /api/checkpoints/prune — drop orphan/stale checkpoints. */
  async checkpointsPrune(days?: number): Promise<CheckpointPruneStats> {
    const response = await fetch(this.endpoint("/api/checkpoints/prune"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify(days != null ? { days } : {}),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `checkpoint prune HTTP ${response.status}`);
    return value.stats as CheckpointPruneStats;
  }

  /** GET /api/checkpoints?dir= — checkpoint list for a workdir (P347). */
  async checkpointsList(dir: string): Promise<CheckpointEntry[]> {
    const params = new URLSearchParams({ dir });
    const response = await fetch(this.endpoint(`/api/checkpoints?${params.toString()}`), {
      headers: this.headers(),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `checkpoints HTTP ${response.status}`);
    return (value.checkpoints || []) as CheckpointEntry[];
  }

  /** POST /api/checkpoints/restore — restore a directory (or single
   * file) to a checkpoint hash (P347). */
  async checkpointsRestore(dir: string, hash: string, file?: string): Promise<void> {
    const response = await fetch(this.endpoint("/api/checkpoints/restore"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify(file ? { dir, hash, file } : { dir, hash }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `checkpoint restore HTTP ${response.status}`);
  }

  /** GET /api/curator — curation overview (status/archived/usage; P316). */
  async curatorStatus(): Promise<CuratorStatus> {
    const response = await fetch(this.endpoint("/api/curator"), { headers: this.headers() });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `curator HTTP ${response.status}`);
    return value as CuratorStatus;
  }

  /** POST /api/curator/{pin|unpin|archive|restore} — skill curation. */
  async curatorAction(
    action: "pin" | "unpin" | "archive" | "restore",
    skill: string,
  ): Promise<void> {
    const response = await fetch(this.endpoint(`/api/curator/${action}`), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ skill }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `curator ${action} HTTP ${response.status}`);
  }

  /** GET /api/backups — quick-snapshot inventory (`backup list`; P315). */
  async backupsList(): Promise<BackupSnapshot[]> {
    const response = await fetch(this.endpoint("/api/backups"), { headers: this.headers() });
    if (!response.ok) throw new Error(`backups HTTP ${response.status}`);
    const value = await response.json();
    return (value.snapshots || []) as BackupSnapshot[];
  }

  /** POST /api/backups — create a quick snapshot (`backup --quick`). */
  async backupCreate(label?: string): Promise<{ id: string | null; message?: string }> {
    const response = await fetch(this.endpoint("/api/backups"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify(label ? { label } : {}),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `backup HTTP ${response.status}`);
    return value as { id: string | null; message?: string };
  }

  /** POST /api/backups/:id/restore — overlay a snapshot onto home. */
  async backupRestore(id: string): Promise<void> {
    const response = await fetch(
      this.endpoint(`/api/backups/${encodeURIComponent(id)}/restore`),
      {
        method: "POST",
        headers: { ...this.headers(), "content-type": "application/json" },
        body: "{}",
      },
    );
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `restore HTTP ${response.status}`);
  }

  /** POST /api/backups/prune — keep only the newest `keep` snapshots. */
  async backupPrune(keep: number): Promise<number> {
    const response = await fetch(this.endpoint("/api/backups/prune"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ keep }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `backup prune HTTP ${response.status}`);
    return (value.removed ?? 0) as number;
  }

  /** POST /api/sessions/prune — delete ended sessions by filter
   * (hermes `sessions prune` parity; dry_run previews). */
  async sessionsPrune(options: SessionPruneOptions): Promise<SessionPruneResult> {
    const response = await fetch(this.endpoint("/api/sessions/prune"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify(options),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `prune HTTP ${response.status}`);
    return value as SessionPruneResult;
  }

  /** POST /api/sessions/archive — soft-hide ended sessions by filter
   * (hermes `sessions archive` parity; recoverable). */
  async sessionsArchive(options: SessionPruneOptions): Promise<SessionPruneResult> {
    const response = await fetch(this.endpoint("/api/sessions/archive"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify(options),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(value.error || `archive HTTP ${response.status}`);
    return value as SessionPruneResult;
  }

  /** POST /api/sessions/:id/fork — copy the session into a new branch. */
  async forkSession(sessionId: string): Promise<SessionRow> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/fork`), {
      method: "POST",
      headers: this.headers(),
      body: JSON.stringify({}),
    });
    if (!response.ok) throw new Error(`fork HTTP ${response.status}`);
    const value = await response.json();
    return value.session as SessionRow;
  }

  /** GET /api/sessions/:id/recap — gateway-built session recap text. */
  async sessionRecap(sessionId: string): Promise<string> {
    const response = await fetch(this.endpoint(`/api/sessions/${sessionId}/recap`), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`recap HTTP ${response.status}`);
    const value = await response.json();
    return typeof value.recap === "string" ? value.recap : "";
  }

  /** GET /api/sessions/:id/export — download the transcript as
   * md/html/json (json is the portable re-importable payload; P348). */
  async exportSession(
    sessionId: string,
    format: "md" | "html" | "json",
  ): Promise<{ blob: Blob; filename: string }> {
    const qs = format === "md" ? "" : `?format=${format}`;
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

  /** POST /api/sessions/import — import portable session JSON exports
   * (hermes `/api/sessions/import` parity; P348). */
  async sessionsImport(sessions: unknown[]): Promise<{
    ok: boolean;
    imported: number;
    skipped: number;
    messages: number;
    errors: { index: number; id: string | null; error: string }[];
  }> {
    const response = await fetch(this.endpoint("/api/sessions/import"), {
      method: "POST",
      headers: { ...this.headers(), "content-type": "application/json" },
      body: JSON.stringify({ sessions }),
    });
    const value = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message =
        typeof value.error === "string"
          ? value.error
          : value.error?.message || `sessions import HTTP ${response.status}`;
      throw new Error(message);
    }
    return value as {
      ok: boolean;
      imported: number;
      skipped: number;
      messages: number;
      errors: { index: number; id: string | null; error: string }[];
    };
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

  async kanbanDispatch(dryRun = false): Promise<KanbanDispatchResult | null> {
    const value = await this.kanbanJson("/api/kanban/dispatch", {
      method: "POST",
      body: JSON.stringify({ dry_run: dryRun }),
    });
    return (value || null) as KanbanDispatchResult | null;
  }

  async kanbanClaim(id: string): Promise<KanbanTask | null> {
    const value = await this.kanbanJson(`/api/kanban/tasks/${encodeURIComponent(id)}/claim`, {
      method: "POST",
    });
    return (value?.task || null) as KanbanTask | null;
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
    description?: string;
    icon?: string;
  }): Promise<Project | null> {
    const value = await this.kanbanJson("/api/projects", {
      method: "POST",
      body: JSON.stringify(request),
    });
    return (value?.project || null) as Project | null;
  }

  async projectUpdate(
    id: string,
    patch: { name?: string; description?: string; board_slug?: string; icon?: string },
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

  /** GET /api/jobs/delivery-targets — where cron results can be delivered. */
  async jobDeliveryTargets(): Promise<JobDeliveryTarget[]> {
    const response = await fetch(this.endpoint("/api/jobs/delivery-targets"), {
      headers: this.headers(),
    });
    if (!response.ok) throw new Error(`delivery targets HTTP ${response.status}`);
    const value = await response.json();
    return (value.targets || []) as JobDeliveryTarget[];
  }

  async jobCreate(request: {
    name: string;
    schedule: string;
    prompt: string;
    skills?: string[];
    repeat?: number;
    deliver?: string;
  }): Promise<CronJob | null> {
    const value = await this.kanbanJson("/api/jobs", {
      method: "POST",
      body: JSON.stringify(request),
    });
    return (value?.job || value) as CronJob | null;
  }

  async jobUpdate(
    id: string,
    patch: { name?: string; schedule?: string; prompt?: string; enabled?: boolean; deliver?: string | null },
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
