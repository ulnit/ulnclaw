// Desktop shell i18n (P251) — scoped port of hermes apps/desktop
// `i18n/`: locale type + curated locale list (en → zh → zh-hant → ja →
// ar, endonym names), alias normalization, a translation catalog per
// locale, and a runtime that persists the choice and flips
// `<html lang/dir>`. hermes persists the locale in the gateway config
// (`display.language`) through the Electron IPC bridge; the gateway here
// exposes no config-write endpoint, so the choice rides localStorage —
// the same fallback hermes' browser mode uses.

export type Locale = "en" | "zh" | "zh-hant" | "ja" | "ar";

export const LOCALE_OPTIONS: { id: Locale; name: string; englishName: string }[] = [
  { id: "en", name: "English", englishName: "English" },
  { id: "zh", name: "简体中文", englishName: "Simplified Chinese" },
  { id: "zh-hant", name: "繁體中文", englishName: "Traditional Chinese" },
  { id: "ja", name: "日本語", englishName: "Japanese" },
  { id: "ar", name: "العربية", englishName: "Arabic" },
];

// Endonyms in the picker so users recognize their language regardless of
// the current UI language; englishName is search-only. No flags:
// languages are not countries (hermes parity).
export const LOCALE_META: Record<Locale, { name: string; englishName: string }> =
  Object.fromEntries(LOCALE_OPTIONS.map((l) => [l.id, { name: l.name, englishName: l.englishName }])) as
    Record<Locale, { name: string; englishName: string }>;

const LOCALE_ALIASES: Record<string, Locale> = {
  en: "en", "en-us": "en", en_us: "en", "en-gb": "en",
  zh: "zh", "zh-cn": "zh", zh_cn: "zh", "zh-hans": "zh", zh_hans: "zh", "zh-hans-cn": "zh",
  "zh-tw": "zh-hant", zh_tw: "zh-hant", "zh-hk": "zh-hant", zh_hk: "zh-hant",
  "zh-mo": "zh-hant", "zh-hant": "zh-hant", zh_hant: "zh-hant", "zh-hant-tw": "zh-hant",
  ja: "ja", "ja-jp": "ja", ja_jp: "ja",
  ar: "ar", "ar-sa": "ar", ar_sa: "ar", "ar-ae": "ar", "ar-eg": "ar",
};

export const DEFAULT_LOCALE: Locale = "en";

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && LOCALE_OPTIONS.some((l) => l.id === value);
}

/** Normalize free-form input (`zh_CN`, `ja-JP`, …) to a supported locale. */
export function normalizeLocale(value: unknown): Locale {
  if (typeof value !== "string" || !value) return DEFAULT_LOCALE;
  const key = value.trim().toLowerCase();
  if (isLocale(key)) return key;
  return LOCALE_ALIASES[key] ?? DEFAULT_LOCALE;
}

/** `{name}`-style interpolation for catalog templates. */
export function fmt(template: string, vars: Record<string, unknown>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) =>
    key in vars ? String(vars[key]) : match,
  );
}

export interface Translations {
  chrome: {
    chatTab: string; kanbanTab: string; projectsTab: string; jobsTab: string;
    newSession: string; settings: string; gatewayStatus: string; hatchPet: string;
    selectOrStart: string; inputPlaceholder: string; send: string;
    settingsTitle: string; gatewayUrl: string; apiKey: string; bearerToken: string;
    manageProcess: string; replayOnboarding: string; cancel: string; save: string;
  };
  session: {
    titlePrompt: string; renamed: string; renameFailed: string; deleteConfirm: string;
    deleteFailed: string; newTitle: string; loadFailed: string; createFailed: string;
    errorPrefix: string; modelLockTitle: string; gatewayModelTitle: string;
    reachable: string; unreachable: string; removeAttachment: string; uploadFailed: string; projectBadge: string;
  };
  tools: { running: string; done: string; arguments: string; result: string; fallbackName: string };
  slash: { help: string; skills: string; tools: string; recap: string; title: string; usage: string; skillFallback: string; resume: string };
  boot: { spawnFailed: string; unreachable: string; unreachableDetail: string; connecting: string; starting: string;
    failureTitle: string; retry: string; openSettings: string; dismiss: string };
  bridge: { preview: string; terminalClosed: string; stillRunning: string; terminalEmpty: string };
  kanban: {
    todo: string; doing: string; done: string; blocked: string;
    addTask: string; addComment: string; comment: string; unblock: string;
    blockEllipsis: string; complete: string; close: string;
    blockAction: string; unblockAction: string; doneAction: string;
    whyBlocked: string; refresh: string; switchBoard: string;
    counts: string; noDescription: string; resultPrefix: string; noComments: string;
  };
  projects: {
    addFolder: string; archive: string; restore: string; bindBoard: string;
    rebindBoard: string; delete: string; makePrimary: string; primaryFolder: string;
    noActiveProject: string; scanRepos: string; scanning: string; archived: string;
    toProject: string; createFromRepo: string; removeFolder: string; scanTitle: string;
    boardSlugPrompt: string; folderPathPrompt: string; scanRootsPrompt: string;
    newProject: string; discoveredRepos: string; nameLabel: string; foldersLabel: string;
    boardLabel: string; setActive: string; create: string; use: string; empty: string;
    reposEmpty: string; deleteConfirm: string; boardBadge: string; scanRecorded: string;
    createFailed: string; activePrefix: string;
  };
  jobs: {
    active: string; paused: string; pause: string; resume: string; runNow: string;
    delete: string; edit: string; promptPrompt: string; schedulePrompt: string;
    whatShouldAgentDo: string;
    fromNow: string; ago: string; newJob: string; newCronJob: string; nameLabel: string;
    scheduleLabel: string; promptLabel: string; skillsLabel: string; repeatLabel: string;
    create: string; createFailed: string; counts: string; empty: string; meta: string;
    runsLeft: string; deleteConfirm: string;
  };
  hatch: {
    title: string; styleLabel: string; draftsLabel: string; designing: string;
    drawing: string; pickBase: string; cancelHatch: string; startOver: string;
    tryAgain: string; failedToLoad: string; loadingSpritesheet: string;
    previewUnavailable: string; hatch: string; done: string; close: string;
    namePlaceholder: string; gatewayOffline: string;
    intro: string; promptPlaceholder: string; draftOne: string; draftMany: string;
    errorNoResult: string; errorCancelled: string; errorFailed: string;
    hatched: string; rowsMeta: string; draftAlt: string;
    stylePixelDefault: string; stylePixel: string; styleFlat: string; style3d: string;
    styleGlossy: string; stylePainterly: string; styleClay: string; stylePlush: string;
  };
  picker: {
    title: string; loading: string; notConnected: string; loadFailed: string;
    gatewayDefault: string; lockNote: string; noProviders: string; noModels: string;
    notAuthenticatedTitle: string; notAuthenticatedBit: string; currentBit: string;
    lockFailed: string;
    visibilityTitle: string; visibilitySearch: string; visibilityEmpty: string;
    addProvider: string; editVisibleModels: string; resetVisibility: string;
  };
  find: { placeholder: string; closeTitle: string; nextTitle: string; prevTitle: string };
  palette: {
    placeholder: string; noMatches: string; navigate: string; sessionGroup: string;
    sessionsGroup: string; gatewayGroup: string; goToChat: string; goToKanban: string;
    goToProjects: string; goToJobs: string; newSession: string; switchSession: string;
    findInChat: string; modelForSession: string; resumeSession: string; renameSession: string;
    deleteSession: string; browseArtifacts: string; learningGraph: string;
    openSettings: string; refreshSessions: string;
    hintFreshChat: string; hintArtifacts: string; hintLearning: string; switchTo: string;
  };
  artifacts: {
    title: string; filterPlaceholder: string; notConnected: string; none: string;
    scanning: string; openSession: string;
  };
  learning: {
    title: string; tagline: string; searchPlaceholder: string; building: string;
    loading: string; notConnected: string; noMatches: string; save: string;
    saved: string; archive: string; archived: string; delete: string; deleted: string;
    close: string;
  };
  notify: { dismiss: string; clearAll: string; details: string; stackTitle: string };
  onboarding: {
    welcomeTitle: string; intro: string; bullet1: string; bullet2: string;
    bullet3: string; skip: string; getStarted: string; finish: string;
    providersTitle: string; loadingProviders: string; noInventory: string;
    currentModel: string; recheck: string; active: string; configured: string;
    needsEnv: string; notConfigured: string; needsEnvTitle: string;
  };
  language: { switchTo: string; searchPlaceholder: string; noResults: string; description: string };
  sessionPicker: { title: string; searchPlaceholder: string; noResults: string; messages: string };
  intro: { headline1: string; body1: string; headline2: string; body2: string;
    headline3: string; body3: string; headline4: string; body4: string;
    headline5: string; body5: string };
}

const en: Translations = {
  chrome: {
    chatTab: "Chat", kanbanTab: "Kanban", projectsTab: "Projects", jobsTab: "Jobs",
    newSession: "New session", settings: "Settings", gatewayStatus: "gateway status",
    hatchPet: "\u{1F95A} Hatch pet",
    selectOrStart: "Select or start a session",
    inputPlaceholder: "Message ulnclaw… (Enter to send, Shift+Enter for newline)",
    send: "Send",
    settingsTitle: "Gateway settings", gatewayUrl: "Gateway URL",
    apiKey: "API key (optional, [gateway] key)", bearerToken: "bearer token",
    manageProcess: "Manage the gateway process (start/stop with the app)",
    replayOnboarding: "Replay onboarding", cancel: "Cancel", save: "Save",
  },
  session: {
    titlePrompt: "Session title:", renamed: "Session renamed.",
    renameFailed: "Rename failed: {error}",
    deleteConfirm: "Delete session \"{label}\" and its transcript?",
    deleteFailed: "Delete failed: {error}", newTitle: "New session",
    loadFailed: "Could not load messages: {error}",
    createFailed: "Could not create a session: {error}",
    errorPrefix: "error: {error}",
    modelLockTitle: "Session model lock — click to change",
    gatewayModelTitle: "Gateway default model — click to pick a session model",
    reachable: "gateway reachable", unreachable: "gateway unreachable",
    removeAttachment: "Remove attachment",
    uploadFailed: "Clipboard upload failed: {error}",
    projectBadge: "Project: {project}",
  },
  tools: { running: "running…", done: "done", arguments: "arguments", result: "result", fallbackName: "tool" },
  slash: {
    help: "gateway slash commands", skills: "list skills", tools: "list enabled tools",
    recap: "recap this session", title: "show or set the session title",
    usage: "this session's token usage", skillFallback: "skill",
    resume: "resume a recent session (desktop)",
  },
  boot: {
    spawnFailed: "Gateway spawn failed: {error}",
    unreachable: "Gateway unreachable — check the gateway URL and API key in Settings.",
    unreachableDetail: "The desktop shell polls /health once the gateway is up; managed mode spawns it automatically when the ulnclaw binary is on PATH.",
    connecting: "CONNECTING", starting: "starting the ulnclaw gateway…",
    failureTitle: "Desktop boot failed", retry: "Retry",
    openSettings: "Open settings", dismiss: "Dismiss",
  },
  bridge: {
    preview: "Preview: {label}",
    terminalClosed: "Terminal closed: {id}{running}",
    stillRunning: " (still running)", terminalEmpty: "terminal pane is empty",
  },
  kanban: {
    todo: "To do", doing: "Doing", done: "Done", blocked: "Blocked",
    addTask: "+ Add a task…", addComment: "Add a comment…", comment: "Comment",
    unblock: "Unblock", blockEllipsis: "Block…", complete: "Complete", close: "Close",
    blockAction: "⛔ block", unblockAction: "↩ unblock", doneAction: "✓ done",
    whyBlocked: "Why is it blocked?", refresh: "Refresh", switchBoard: "Switch board",
    counts: "{open} open · {total} total", noDescription: "(no description)",
    resultPrefix: "Result: {result}", noComments: "No comments yet.",
  },
  projects: {
    addFolder: "Add folder", archive: "Archive", restore: "Restore",
    bindBoard: "Bind board", rebindBoard: "Rebind board", delete: "Delete",
    makePrimary: "Make primary", primaryFolder: "Primary folder",
    noActiveProject: "No active project", scanRepos: "Scan repos",
    scanning: "Scanning…", archived: "archived", toProject: "→ project",
    createFromRepo: "Create a project from this repo", removeFolder: "Remove folder",
    scanTitle: "Scan the filesystem for git repos",
    boardSlugPrompt: "Board slug (empty unbinds):", folderPathPrompt: "Folder path:",
    scanRootsPrompt: "Scan roots (comma-separated; empty = home directory):",
    newProject: "New project", discoveredRepos: "Discovered repos",
    nameLabel: "Name", foldersLabel: "Folders (comma-separated; first = primary)",
    boardLabel: "Bind kanban board (optional slug)", setActive: "Set as active project",
    create: "Create", use: "Use",
    empty: "No projects yet — create one, or scan the filesystem for git repos.",
    reposEmpty: "Discovery cache is empty — run “Scan repos”.",
    deleteConfirm: "Delete project “{name}”? This only removes the registry entry.",
    boardBadge: "board: {slug}",
    scanRecorded: "Recorded {count} repo(s) into the discovery cache.",
    createFailed: "Project creation failed (gateway unreachable or invalid input).",
    activePrefix: "Active: {name}",
  },
  jobs: {
    active: "Active", paused: "Paused", pause: "Pause", resume: "Resume",
    runNow: "Run now", delete: "Delete", edit: "Edit prompt/schedule",
    promptPrompt: "Prompt:", schedulePrompt: "Schedule:",
    whatShouldAgentDo: "What should the agent do?",
    fromNow: "from now", ago: "ago", newJob: "New job", newCronJob: "New cron job",
    nameLabel: "Name",
    scheduleLabel: "Schedule (cron expression, @every 30m, or @at unix-ts)",
    promptLabel: "Prompt", skillsLabel: "Skills (comma-separated, optional)",
    repeatLabel: "Repeat (runs remaining; empty = forever)", create: "Create",
    createFailed: "Job creation failed (gateway unreachable or invalid schedule).",
    counts: "{active} active / {total} jobs",
    empty: "No cron jobs yet — create one, or use `ulnclaw cron add` in the terminal.",
    meta: "next: {next} · last: {last}", runsLeft: " · {count} run(s) left",
    deleteConfirm: "Delete job “{name}”?",
  },
  hatch: {
    title: "\u{1F95A} Hatch a pet", styleLabel: "Style ", draftsLabel: "Drafts ",
    designing: "Designing base looks…", drawing: "Drawing animation rows…",
    pickBase: "Pick the base look you like best — it anchors every animation row.",
    cancelHatch: "Cancel hatch", startOver: "Start over", tryAgain: "Try again",
    failedToLoad: "failed to load", loadingSpritesheet: "loading spritesheet…",
    previewUnavailable: "spritesheet preview unavailable",
    hatch: "Hatch", done: "Done", close: "Close",
    namePlaceholder: "Name (optional)", gatewayOffline: "gateway offline",
    stylePixelDefault: "Pixel art (hermes default)", stylePixel: "Pixel art",
    styleFlat: "Flat vector", styleGlossy: "Glossy sticker",
    stylePainterly: "Painterly", styleClay: "Claymation", stylePlush: "Plush toy", style3d: "3D toy",
    intro: "Describe a pet; the image model sketches base looks, you pick one, and the hatch pipeline draws every animation row (a few minutes).",
    promptPlaceholder: "a tiny cyber fox with neon accents",
    draftOne: "{count} draft", draftMany: "{count} drafts",
    errorNoResult: "hatch finished without a result", errorCancelled: "hatch cancelled",
    errorFailed: "hatch failed",
    hatched: "(^_^)b {name} hatched and adopted!",
    rowsMeta: "{count} animation rows — it'll pop into the corner shortly.",
    draftAlt: "draft {index}",
  },
  picker: {
    title: "\u{1F9E0} Model for this session", loading: "Loading model inventory…",
    notConnected: "Gateway not connected.", loadFailed: "Failed to load models: {error}",
    gatewayDefault: "(gateway default)",
    lockNote: "\u{1F512} This session is locked to {model}.",
    noProviders: "No providers reported by the gateway.", noModels: "No models listed",
    notAuthenticatedTitle: "Provider not authenticated",
    notAuthenticatedBit: "⚠ not authenticated", currentBit: "current",
    lockFailed: "Lock failed: {error}",
    visibilityTitle: "Visible models", visibilitySearch: "Filter models…",
    visibilityEmpty: "No authenticated providers reported models.",
    addProvider: "Add provider…", editVisibleModels: "Edit visible models…",
    resetVisibility: "Reset to defaults",
  },
  find: {
    placeholder: "Find in chat…", closeTitle: "Close (Esc)",
    nextTitle: "Next match (Enter)", prevTitle: "Previous match (Shift+Enter)",
  },
  palette: {
    placeholder: "Type a command… (Esc to close)", noMatches: "No matching commands",
    navigate: "Navigate", sessionGroup: "Session", sessionsGroup: "Sessions",
    gatewayGroup: "Gateway", goToChat: "Go to Chat", goToKanban: "Go to Kanban",
    goToProjects: "Go to Projects", goToJobs: "Go to Jobs (cron)",
    newSession: "New session", switchSession: "Switch session",
    findInChat: "Find in chat", modelForSession: "Model for this session…",
    resumeSession: "Resume session… (/resume)",
    renameSession: "Rename session…", deleteSession: "Delete session…",
    browseArtifacts: "Browse artifacts…", learningGraph: "Learning graph…",
    openSettings: "Open gateway settings…", refreshSessions: "Refresh session list",
    hintFreshChat: "start a fresh chat", hintArtifacts: "links, files, images",
    hintLearning: "learned skills + memory", switchTo: "Switch to: {title}",
  },
  artifacts: {
    title: "\u{1F5C2}️ Artifacts", filterPlaceholder: "Filter artifacts…",
    notConnected: "Gateway not connected.",
    none: "No artifacts found in recent sessions.",
    scanning: "Scanning recent sessions…", openSession: "Open session",
  },
  learning: {
    title: "✨ Learning", tagline: "learned skills + memory, linked",
    searchPlaceholder: "Search nodes…", building: "Building learning graph…",
    loading: "Loading…", notConnected: "Gateway not connected.",
    noMatches: "No matching nodes.", save: "Save", saved: "Saved.",
    archive: "Archive", archived: "Archived.", delete: "Delete", deleted: "Deleted.",
    close: "Close",
  },
  notify: { dismiss: "Dismiss notification", clearAll: "Clear all", details: "Details", stackTitle: "Notifications" },
  onboarding: {
    welcomeTitle: "Welcome to ulnclaw",
    intro: "ulnclaw is a Rust re-implementation of the Hermes Agent engine: 50+ tools, skills, scheduled jobs, messaging gateways and a local HTTP gateway — this desktop shell talks to the gateway over plain HTTP/SSE.",
    bullet1: "Chat streams tokens live with tool-progress cards.",
    bullet2: "Kanban, Projects and Jobs dashboards live in the sidebar tabs.",
    bullet3: "Ctrl/Cmd+K opens the command palette; Ctrl/Cmd+F finds in chat.",
    skip: "Skip", getStarted: "Get started", finish: "Finish",
    providersTitle: "Model providers", loadingProviders: "Loading provider inventory…",
    noInventory: "The gateway did not return a provider inventory. You can still continue — configure [model] in ~/.ulnclaw/config.toml and restart the gateway.",
    currentModel: "Current model: {model} ({provider})",
    recheck: "Re-check providers", active: "✓ active", configured: "✓ configured",
    needsEnv: "needs {env}", notConfigured: "not configured",
    needsEnvTitle: "Set {env} (env var or ~/.ulnclaw/.env), then re-check.",
  },
  language: {
    switchTo: "Switch language", searchPlaceholder: "Search languages…",
    noResults: "No matching languages", description: "Choose the display language for the desktop shell.",
  },
  sessionPicker: {
    title: "Sessions", searchPlaceholder: "Search sessions…",
    noResults: "No matching sessions", messages: "{count} messages",
  },
  intro: {
    headline1: "What are we moving today?",
    body1: "Send a bug, branch, plan, or rough idea. I'll inspect the repo and turn it into the next concrete step.",
    headline2: "What's on your mind?",
    body2: "Bring the code, question, or stuck part. I'll read the room before making changes.",
    headline3: "What should ulnclaw look at?",
    body3: "Send the task, failing path, or half-formed plan. I'll help turn it into action.",
    headline4: "Where should we start?",
    body4: "Bring the problem, goal, or file. I'll inspect first and keep the next step concrete.",
    headline5: "What needs attention?",
    body5: "Send the context you have. I'll help sort it into a plan or a fix.",
  },
};

const zh: Translations = {
  chrome: {
    chatTab: "聊天", kanbanTab: "看板", projectsTab: "项目", jobsTab: "任务",
    newSession: "新建会话", settings: "设置", gatewayStatus: "网关状态",
    hatchPet: "\u{1F95A} 孵化宠物",
    selectOrStart: "选择或开始一个会话",
    inputPlaceholder: "给 ulnclaw 发消息…（Enter 发送，Shift+Enter 换行）",
    send: "发送",
    settingsTitle: "网关设置", gatewayUrl: "网关 URL",
    apiKey: "API 密钥（可选，[gateway] key）", bearerToken: "bearer 令牌",
    manageProcess: "管理网关进程（随应用启动/停止）",
    replayOnboarding: "重放引导", cancel: "取消", save: "保存",
  },
  session: {
    titlePrompt: "会话标题：", renamed: "会话已重命名。",
    renameFailed: "重命名失败：{error}",
    deleteConfirm: "删除会话「{label}」及其对话记录？",
    deleteFailed: "删除失败：{error}", newTitle: "新会话",
    loadFailed: "无法加载消息：{error}",
    createFailed: "无法创建会话：{error}",
    errorPrefix: "错误：{error}",
    modelLockTitle: "会话模型锁定 — 点击更改",
    gatewayModelTitle: "网关默认模型 — 点击挑选会话模型",
    reachable: "网关可达", unreachable: "网关不可达",
    removeAttachment: "移除附件",
    uploadFailed: "剪贴板上传失败：{error}",
    projectBadge: "项目：{project}",
  },
  tools: { running: "运行中…", done: "完成", arguments: "参数", result: "结果", fallbackName: "工具" },
  slash: {
    help: "网关斜杠命令", skills: "列出技能", tools: "列出已启用工具",
    recap: "回顾本会话", title: "查看或设置会话标题",
    usage: "本会话的 token 用量", skillFallback: "技能",
    resume: "恢复近期会话（桌面）",
  },
  boot: {
    spawnFailed: "网关拉起失败：{error}",
    unreachable: "网关不可达 — 请在设置中检查网关 URL 与 API 密钥。",
    unreachableDetail: "网关就绪后桌面外壳会轮询 /health；托管模式在 ulnclaw 二进制位于 PATH 上时会自动拉起网关。",
    connecting: "连接中", starting: "正在启动 ulnclaw 网关…",
    failureTitle: "桌面启动失败", retry: "重试",
    openSettings: "打开设置", dismiss: "忽略",
  },
  bridge: {
    preview: "预览：{label}",
    terminalClosed: "终端已关闭：{id}{running}",
    stillRunning: "（仍在运行）", terminalEmpty: "终端面板为空",
  },
  kanban: {
    todo: "待办", doing: "进行中", done: "已完成", blocked: "受阻",
    addTask: "+ 添加任务…", addComment: "添加评论…", comment: "评论",
    unblock: "解除阻塞", blockEllipsis: "阻塞…", complete: "完成", close: "关闭",
    blockAction: "⛔ 阻塞", unblockAction: "↩ 解除阻塞", doneAction: "✓ 完成",
    whyBlocked: "为什么受阻？", refresh: "刷新", switchBoard: "切换看板",
    counts: "{open} 进行中 · 共 {total}", noDescription: "（无描述）",
    resultPrefix: "结果：{result}", noComments: "暂无评论。",
  },
  projects: {
    addFolder: "添加文件夹", archive: "归档", restore: "恢复",
    bindBoard: "绑定看板", rebindBoard: "重新绑定看板", delete: "删除",
    makePrimary: "设为主目录", primaryFolder: "主文件夹",
    noActiveProject: "无启用项目", scanRepos: "扫描仓库",
    scanning: "扫描中…", archived: "已归档", toProject: "→ 设为项目",
    createFromRepo: "从该仓库创建项目", removeFolder: "移除文件夹",
    scanTitle: "扫描文件系统查找 git 仓库",
    boardSlugPrompt: "看板 slug（留空解绑）：", folderPathPrompt: "文件夹路径：",
    scanRootsPrompt: "扫描根目录（逗号分隔；留空 = 主目录）：",
    newProject: "新建项目", discoveredRepos: "发现的仓库",
    nameLabel: "名称", foldersLabel: "文件夹（逗号分隔；第一个 = 主目录）",
    boardLabel: "绑定看板（可选 slug）", setActive: "设为启用项目",
    create: "创建", use: "启用",
    empty: "还没有项目 —— 创建一个，或扫描文件系统查找 git 仓库。",
    reposEmpty: "发现缓存为空 —— 运行“扫描仓库”。",
    deleteConfirm: "删除项目「{name}」？这只会移除登记簿条目。",
    boardBadge: "看板：{slug}",
    scanRecorded: "已记录 {count} 个仓库到发现缓存。",
    createFailed: "项目创建失败（网关不可达或输入无效）。",
    activePrefix: "启用：{name}",
  },
  jobs: {
    active: "活跃", paused: "已暂停", pause: "暂停", resume: "恢复",
    runNow: "立即运行", delete: "删除", edit: "编辑提示词/调度",
    promptPrompt: "提示词：", schedulePrompt: "调度：",
    whatShouldAgentDo: "让 agent 做什么？",
    fromNow: "后", ago: "前", newJob: "新建任务", newCronJob: "新建定时任务",
    nameLabel: "名称",
    scheduleLabel: "调度（cron 表达式、@every 30m 或 @at unix 时间戳）",
    promptLabel: "提示词", skillsLabel: "技能（逗号分隔，可选）",
    repeatLabel: "重复（剩余运行次数；留空 = 永远）", create: "创建",
    createFailed: "任务创建失败（网关不可达或调度无效）。",
    counts: "{active} 活跃 / 共 {total} 个任务",
    empty: "还没有定时任务 —— 创建一个，或在终端使用 `ulnclaw cron add`。",
    meta: "下次：{next} · 上次：{last}", runsLeft: " · 剩余 {count} 次运行",
    deleteConfirm: "删除任务「{name}」？",
  },
  hatch: {
    title: "\u{1F95A} 孵化宠物", styleLabel: "风格 ", draftsLabel: "草稿数 ",
    designing: "设计基础外观中…", drawing: "绘制动画行中…",
    pickBase: "挑选你最喜欢的基础外观 —— 它是每一行动画的基准。",
    cancelHatch: "取消孵化", startOver: "重新开始", tryAgain: "重试",
    failedToLoad: "加载失败", loadingSpritesheet: "加载精灵图中…",
    previewUnavailable: "精灵图预览不可用",
    hatch: "孵化", done: "完成", close: "关闭",
    namePlaceholder: "名称（可选）", gatewayOffline: "网关离线",
    stylePixelDefault: "像素画（hermes 默认）", stylePixel: "像素画",
    styleFlat: "扁平矢量", styleGlossy: "亮面贴纸",
    stylePainterly: "绘画风", styleClay: "黏土动画", stylePlush: "毛绒玩具", style3d: "3D 玩具",
    intro: "描述一只宠物；图像模型会草绘基础外观，你挑选一个，孵化流水线随后绘制每一行动画（约几分钟）。",
    promptPlaceholder: "一只带霓虹点缀的迷你赛博狐狸",
    draftOne: "{count} 份草稿", draftMany: "{count} 份草稿",
    errorNoResult: "孵化完成但没有结果", errorCancelled: "孵化已取消",
    errorFailed: "孵化失败",
    hatched: "(^_^)b {name} 已孵化并领养！",
    rowsMeta: "{count} 行动画 —— 它马上会出现在角落。",
    draftAlt: "草稿 {index}",
  },
  picker: {
    title: "\u{1F9E0} 本会话模型", loading: "加载模型清单中…",
    notConnected: "网关未连接。", loadFailed: "模型加载失败：{error}",
    gatewayDefault: "（网关默认）",
    lockNote: "\u{1F512} 本会话已锁定为 {model}。",
    noProviders: "网关未报告任何 provider。", noModels: "未列出模型",
    notAuthenticatedTitle: "provider 未认证",
    notAuthenticatedBit: "⚠ 未认证", currentBit: "当前",
    lockFailed: "锁定失败：{error}",
    visibilityTitle: "可见模型", visibilitySearch: "过滤模型…",
    visibilityEmpty: "没有已认证 provider 报告模型。",
    addProvider: "添加 provider…", editVisibleModels: "编辑可见模型…",
    resetVisibility: "恢复默认",
  },
  find: {
    placeholder: "聊天中查找…", closeTitle: "关闭（Esc）",
    nextTitle: "下一处匹配（Enter）", prevTitle: "上一处匹配（Shift+Enter）",
  },
  palette: {
    placeholder: "输入命令…（Esc 关闭）", noMatches: "没有匹配的命令",
    navigate: "导航", sessionGroup: "会话", sessionsGroup: "会话列表",
    gatewayGroup: "网关", goToChat: "前往聊天", goToKanban: "前往看板",
    goToProjects: "前往项目", goToJobs: "前往任务（cron）",
    newSession: "新建会话", switchSession: "切换会话",
    findInChat: "聊天内查找", modelForSession: "本会话模型…",
    resumeSession: "恢复会话…（/resume）",
    renameSession: "重命名会话…", deleteSession: "删除会话…",
    browseArtifacts: "浏览工件…", learningGraph: "学习图谱…",
    openSettings: "打开网关设置…", refreshSessions: "刷新会话列表",
    hintFreshChat: "开始全新聊天", hintArtifacts: "链接、文件、图片",
    hintLearning: "已学技能 + 记忆", switchTo: "切换到：{title}",
  },
  artifacts: {
    title: "\u{1F5C2}️ 工件", filterPlaceholder: "过滤工件…",
    notConnected: "网关未连接。",
    none: "近期会话中未发现工件。",
    scanning: "扫描近期会话中…", openSession: "打开会话",
  },
  learning: {
    title: "✨ 学习", tagline: "已学技能 + 记忆，相互连接",
    searchPlaceholder: "搜索节点…", building: "构建学习图谱中…",
    loading: "加载中…", notConnected: "网关未连接。",
    noMatches: "没有匹配的节点。", save: "保存", saved: "已保存。",
    archive: "归档", archived: "已归档。", delete: "删除", deleted: "已删除。",
    close: "关闭",
  },
  notify: { dismiss: "关闭通知", clearAll: "全部清除", details: "详情", stackTitle: "通知" },
  onboarding: {
    welcomeTitle: "欢迎使用 ulnclaw",
    intro: "ulnclaw 是 Hermes Agent 引擎的 Rust 复刻：50+ 工具、技能、定时任务、消息网关与本地 HTTP 网关 —— 本桌面外壳通过纯 HTTP/SSE 与网关对话。",
    bullet1: "聊天实时流式输出 token，并带工具进度卡片。",
    bullet2: "看板、项目与任务仪表盘位于侧栏标签页。",
    bullet3: "Ctrl/Cmd+K 打开命令面板；Ctrl/Cmd+F 聊天内查找。",
    skip: "跳过", getStarted: "开始使用", finish: "完成",
    providersTitle: "模型 provider", loadingProviders: "加载 provider 清单中…",
    noInventory: "网关未返回 provider 清单。你仍可继续 —— 在 ~/.ulnclaw/config.toml 配置 [model] 后重启网关即可。",
    currentModel: "当前模型：{model}（{provider}）",
    recheck: "重新检查 provider", active: "✓ 活跃", configured: "✓ 已配置",
    needsEnv: "需要 {env}", notConfigured: "未配置",
    needsEnvTitle: "设置 {env}（环境变量或 ~/.ulnclaw/.env），然后重新检查。",
  },
  language: {
    switchTo: "切换语言", searchPlaceholder: "搜索语言…",
    noResults: "未找到匹配语言", description: "选择桌面外壳的显示语言。",
  },
  sessionPicker: {
    title: "会话", searchPlaceholder: "搜索会话…",
    noResults: "没有匹配的会话", messages: "{count} 条消息",
  },
  intro: {
    headline1: "今天想推进什么？",
    body1: "丢一个 bug、分支、计划或粗糙的想法过来，我会查看仓库并把它变成下一步具体行动。",
    headline2: "你在想什么？",
    body2: "带上代码、问题或卡住的地方，我会先摸清状况再动手。",
    headline3: "想让 ulnclaw 看什么？",
    body3: "发来任务、失败路径或半成品计划，我帮你把它变成行动。",
    headline4: "从哪里开始？",
    body4: "带上问题、目标或文件，我会先检查，再让下一步保持具体。",
    headline5: "有什么需要处理？",
    body5: "把你手头的上下文发来，我帮你梳理成计划或修复。",
  },
};

const zhHant: Translations = {
  chrome: {
    chatTab: "聊天", kanbanTab: "看板", projectsTab: "專案", jobsTab: "工作",
    newSession: "新增工作階段", settings: "設定", gatewayStatus: "閘道狀態",
    hatchPet: "\u{1F95A} 孵化寵物",
    selectOrStart: "選擇或開始工作階段",
    inputPlaceholder: "傳送訊息給 ulnclaw…（Enter 傳送，Shift+Enter 換行）",
    send: "傳送",
    settingsTitle: "閘道設定", gatewayUrl: "閘道 URL",
    apiKey: "API 金鑰（選填，[gateway] key）", bearerToken: "bearer 權杖",
    manageProcess: "管理閘道程序（隨應用程式啟動/停止）",
    replayOnboarding: "重播引導", cancel: "取消", save: "儲存",
  },
  session: {
    titlePrompt: "工作階段標題：", renamed: "已重新命名工作階段。",
    renameFailed: "重新命名失敗：{error}",
    deleteConfirm: "刪除工作階段「{label}」及其對話紀錄？",
    deleteFailed: "刪除失敗：{error}", newTitle: "新工作階段",
    loadFailed: "無法載入訊息：{error}",
    createFailed: "無法建立工作階段：{error}",
    errorPrefix: "錯誤：{error}",
    modelLockTitle: "工作階段模型鎖定 — 點選以變更",
    gatewayModelTitle: "閘道預設模型 — 點選以挑選工作階段模型",
    reachable: "閘道可達", unreachable: "閘道不可達",
    removeAttachment: "移除附件",
    uploadFailed: "剪貼簿上傳失敗：{error}",
    projectBadge: "專案：{project}",
  },
  tools: { running: "執行中…", done: "完成", arguments: "參數", result: "結果", fallbackName: "工具" },
  slash: {
    help: "閘道斜線命令", skills: "列出技能", tools: "列出已啟用工具",
    recap: "回顧本工作階段", title: "檢視或設定工作階段標題",
    usage: "本工作階段的 token 用量", skillFallback: "技能",
    resume: "恢復近期工作階段（桌面）",
  },
  boot: {
    spawnFailed: "閘道啟動失敗：{error}",
    unreachable: "閘道不可達 — 請在設定中檢查閘道 URL 與 API 金鑰。",
    unreachableDetail: "閘道就緒後桌面外殼會輪詢 /health；託管模式在 ulnclaw 二進位檔位於 PATH 上時會自動啟動閘道。",
    connecting: "連線中", starting: "正在啟動 ulnclaw 閘道…",
    failureTitle: "桌面啟動失敗", retry: "重試",
    openSettings: "開啟設定", dismiss: "忽略",
  },
  bridge: {
    preview: "預覽：{label}",
    terminalClosed: "終端已關閉：{id}{running}",
    stillRunning: "（仍在執行）", terminalEmpty: "終端面板為空",
  },
  kanban: {
    todo: "待辦", doing: "進行中", done: "已完成", blocked: "受阻",
    addTask: "+ 新增任務…", addComment: "新增留言…", comment: "留言",
    unblock: "解除阻塞", blockEllipsis: "阻塞…", complete: "完成", close: "關閉",
    blockAction: "⛔ 阻塞", unblockAction: "↩ 解除阻塞", doneAction: "✓ 完成",
    whyBlocked: "為何受阻？", refresh: "重新整理", switchBoard: "切換看板",
    counts: "{open} 進行中 · 共 {total}", noDescription: "（無描述）",
    resultPrefix: "結果：{result}", noComments: "尚無留言。",
  },
  projects: {
    addFolder: "新增資料夾", archive: "封存", restore: "還原",
    bindBoard: "繫結看板", rebindBoard: "重新繫結看板", delete: "刪除",
    makePrimary: "設為主要目錄", primaryFolder: "主要資料夾",
    noActiveProject: "無啟用專案", scanRepos: "掃描儲存庫",
    scanning: "掃描中…", archived: "已封存", toProject: "→ 設為專案",
    createFromRepo: "從此儲存庫建立專案", removeFolder: "移除資料夾",
    scanTitle: "掃描檔案系統尋找 git 儲存庫",
    boardSlugPrompt: "看板 slug（留空解除繫結）：", folderPathPrompt: "資料夾路徑：",
    scanRootsPrompt: "掃描根目錄（逗號分隔；留空 = 家目錄）：",
    newProject: "新增專案", discoveredRepos: "發現的儲存庫",
    nameLabel: "名稱", foldersLabel: "資料夾（逗號分隔；第一個 = 主要目錄）",
    boardLabel: "繫結看板（選填 slug）", setActive: "設為啟用專案",
    create: "建立", use: "啟用",
    empty: "還沒有專案 —— 建立一個，或掃描檔案系統尋找 git 儲存庫。",
    reposEmpty: "發現快取為空 —— 執行「掃描儲存庫」。",
    deleteConfirm: "刪除專案「{name}」？這只會移除登記簿條目。",
    boardBadge: "看板：{slug}",
    scanRecorded: "已記錄 {count} 個儲存庫至發現快取。",
    createFailed: "專案建立失敗（閘道不可達或輸入無效）。",
    activePrefix: "啟用：{name}",
  },
  jobs: {
    active: "活躍", paused: "已暫停", pause: "暫停", resume: "恢復",
    runNow: "立即執行", delete: "刪除", edit: "編輯提示詞/排程",
    promptPrompt: "提示詞：", schedulePrompt: "排程：",
    whatShouldAgentDo: "要讓 agent 做什麼？",
    fromNow: "後", ago: "前", newJob: "新增工作", newCronJob: "新增定時工作",
    nameLabel: "名稱",
    scheduleLabel: "排程（cron 表示式、@every 30m 或 @at unix 時間戳記）",
    promptLabel: "提示詞", skillsLabel: "技能（逗號分隔，選填）",
    repeatLabel: "重複（剩餘執行次數；留空 = 永遠）", create: "建立",
    createFailed: "工作建立失敗（閘道不可達或排程無效）。",
    counts: "{active} 活躍 / 共 {total} 個工作",
    empty: "還沒有定時工作 —— 建立一個，或在終端機使用 `ulnclaw cron add`。",
    meta: "下次：{next} · 上次：{last}", runsLeft: " · 剩餘 {count} 次執行",
    deleteConfirm: "刪除工作「{name}」？",
  },
  hatch: {
    title: "\u{1F95A} 孵化寵物", styleLabel: "風格 ", draftsLabel: "草稿數 ",
    designing: "設計基礎外觀中…", drawing: "繪製動畫列中…",
    pickBase: "挑選你最喜歡的基礎外觀 —— 它是每一列動畫的基準。",
    cancelHatch: "取消孵化", startOver: "重新開始", tryAgain: "重試",
    failedToLoad: "載入失敗", loadingSpritesheet: "載入精靈圖中…",
    previewUnavailable: "精靈圖預覽不可用",
    hatch: "孵化", done: "完成", close: "關閉",
    namePlaceholder: "名稱（選填）", gatewayOffline: "閘道離線",
    stylePixelDefault: "像素畫（hermes 預設）", stylePixel: "像素畫",
    styleFlat: "扁平向量", styleGlossy: "亮面貼紙",
    stylePainterly: "繪畫風", styleClay: "黏土動畫", stylePlush: "毛絨玩具", style3d: "3D 玩具",
    intro: "描述一隻寵物；圖像模型會草繪基礎外觀，你挑選一個，孵化流水線隨後繪製每一列動畫（約幾分鐘）。",
    promptPlaceholder: "一隻帶霓虹點綴的迷你賽博狐狸",
    draftOne: "{count} 份草稿", draftMany: "{count} 份草稿",
    errorNoResult: "孵化完成但沒有結果", errorCancelled: "孵化已取消",
    errorFailed: "孵化失敗",
    hatched: "(^_^)b {name} 已孵化並領養！",
    rowsMeta: "{count} 列動畫 —— 它馬上會出現在角落。",
    draftAlt: "草稿 {index}",
  },
  picker: {
    title: "\u{1F9E0} 本工作階段模型", loading: "載入模型清單中…",
    notConnected: "閘道未連線。", loadFailed: "模型載入失敗：{error}",
    gatewayDefault: "（閘道預設）",
    lockNote: "\u{1F512} 本工作階段已鎖定為 {model}。",
    noProviders: "閘道未回報任何 provider。", noModels: "未列出模型",
    notAuthenticatedTitle: "provider 未認證",
    notAuthenticatedBit: "⚠ 未認證", currentBit: "目前",
    lockFailed: "鎖定失敗：{error}",
    visibilityTitle: "可見模型", visibilitySearch: "過濾模型…",
    visibilityEmpty: "沒有已認證 provider 報告模型。",
    addProvider: "新增 provider…", editVisibleModels: "編輯可見模型…",
    resetVisibility: "恢復預設",
  },
  find: {
    placeholder: "在聊天中尋找…", closeTitle: "關閉（Esc）",
    nextTitle: "下一處符合（Enter）", prevTitle: "上一處符合（Shift+Enter）",
  },
  palette: {
    placeholder: "輸入命令…（Esc 關閉）", noMatches: "沒有符合的命令",
    navigate: "導覽", sessionGroup: "工作階段", sessionsGroup: "工作階段清單",
    gatewayGroup: "閘道", goToChat: "前往聊天", goToKanban: "前往看板",
    goToProjects: "前往專案", goToJobs: "前往工作（cron）",
    newSession: "新增工作階段", switchSession: "切換工作階段",
    findInChat: "聊天內尋找", modelForSession: "本工作階段模型…",
    resumeSession: "恢復工作階段…（/resume）",
    renameSession: "重新命名工作階段…", deleteSession: "刪除工作階段…",
    browseArtifacts: "瀏覽工件…", learningGraph: "學習圖譜…",
    openSettings: "開啟閘道設定…", refreshSessions: "重新整理工作階段清單",
    hintFreshChat: "開始全新聊天", hintArtifacts: "連結、檔案、圖片",
    hintLearning: "已學技能 + 記憶", switchTo: "切換至：{title}",
  },
  artifacts: {
    title: "\u{1F5C2}️ 工件", filterPlaceholder: "過濾工件…",
    notConnected: "閘道未連線。",
    none: "近期工作階段中未發現工件。",
    scanning: "掃描近期工作階段中…", openSession: "開啟工作階段",
  },
  learning: {
    title: "✨ 學習", tagline: "已學技能 + 記憶，相互連結",
    searchPlaceholder: "搜尋節點…", building: "建置學習圖譜中…",
    loading: "載入中…", notConnected: "閘道未連線。",
    noMatches: "沒有符合的節點。", save: "儲存", saved: "已儲存。",
    archive: "封存", archived: "已封存。", delete: "刪除", deleted: "已刪除。",
    close: "關閉",
  },
  notify: { dismiss: "關閉通知", clearAll: "全部清除", details: "詳情", stackTitle: "通知" },
  onboarding: {
    welcomeTitle: "歡迎使用 ulnclaw",
    intro: "ulnclaw 是 Hermes Agent 引擎的 Rust 重製：50+ 工具、技能、排程工作、訊息閘道與本機 HTTP 閘道 —— 本桌面外殼透過純 HTTP/SSE 與閘道對話。",
    bullet1: "聊天即時串流輸出 token，並附工具進度卡片。",
    bullet2: "看板、專案與工作儀表板位於側欄分頁。",
    bullet3: "Ctrl/Cmd+K 開啟命令面板；Ctrl/Cmd+F 聊天內尋找。",
    skip: "略過", getStarted: "開始使用", finish: "完成",
    providersTitle: "模型 provider", loadingProviders: "載入 provider 清單中…",
    noInventory: "閘道未回傳 provider 清單。你仍可繼續 —— 在 ~/.ulnclaw/config.toml 設定 [model] 後重新啟動閘道即可。",
    currentModel: "目前模型：{model}（{provider}）",
    recheck: "重新檢查 provider", active: "✓ 活躍", configured: "✓ 已設定",
    needsEnv: "需要 {env}", notConfigured: "未設定",
    needsEnvTitle: "設定 {env}（環境變數或 ~/.ulnclaw/.env），然後重新檢查。",
  },
  language: {
    switchTo: "切換語言", searchPlaceholder: "搜尋語言…",
    noResults: "未找到符合的語言", description: "選擇桌面外殼的顯示語言。",
  },
  sessionPicker: {
    title: "工作階段", searchPlaceholder: "搜尋工作階段…",
    noResults: "沒有符合的工作階段", messages: "{count} 則訊息",
  },
  intro: {
    headline1: "今天想推進什麼？",
    body1: "丟一個 bug、分支、計畫或粗糙的想法過來，我會檢視儲存庫並把它變成下一步具體行動。",
    headline2: "你在想什麼？",
    body2: "帶上程式碼、問題或卡住的地方，我會先摸清狀況再動手。",
    headline3: "想讓 ulnclaw 看什麼？",
    body3: "發來任務、失敗路徑或半成品計畫，我幫你把它變成行動。",
    headline4: "從哪裡開始？",
    body4: "帶上問題、目標或檔案，我會先檢視，再讓下一步保持具體。",
    headline5: "有什麼需要處理？",
    body5: "把你手邊的上下文發來，我幫你梳理成計畫或修復。",
  },
};

const ja: Translations = {
  chrome: {
    chatTab: "チャット", kanbanTab: "カンバン", projectsTab: "プロジェクト", jobsTab: "ジョブ",
    newSession: "新規セッション", settings: "設定", gatewayStatus: "ゲートウェイ状態",
    hatchPet: "\u{1F95A} ペットをふ化",
    selectOrStart: "セッションを選択または開始",
    inputPlaceholder: "ulnclaw にメッセージ…（Enter で送信、Shift+Enter で改行）",
    send: "送信",
    settingsTitle: "ゲートウェイ設定", gatewayUrl: "ゲートウェイ URL",
    apiKey: "API キー（任意、[gateway] key）", bearerToken: "bearer トークン",
    manageProcess: "ゲートウェイプロセスを管理（アプリと連動して起動/停止）",
    replayOnboarding: "オンボーディングを再生", cancel: "キャンセル", save: "保存",
  },
  session: {
    titlePrompt: "セッションタイトル：", renamed: "セッション名を変更しました。",
    renameFailed: "名前の変更に失敗：{error}",
    deleteConfirm: "セッション「{label}」とその会話を削除しますか？",
    deleteFailed: "削除に失敗：{error}", newTitle: "新規セッション",
    loadFailed: "メッセージを読み込めません：{error}",
    createFailed: "セッションを作成できません：{error}",
    errorPrefix: "エラー：{error}",
    modelLockTitle: "セッションモデルのロック — クリックで変更",
    gatewayModelTitle: "ゲートウェイ既定モデル — クリックでセッションモデルを選択",
    reachable: "ゲートウェイ到達可能", unreachable: "ゲートウェイ到達不能",
    removeAttachment: "添付を削除",
    uploadFailed: "クリップボードのアップロードに失敗：{error}",
    projectBadge: "プロジェクト：{project}",
  },
  tools: { running: "実行中…", done: "完了", arguments: "引数", result: "結果", fallbackName: "ツール" },
  slash: {
    help: "ゲートウェイのスラッシュコマンド", skills: "スキル一覧", tools: "有効なツール一覧",
    recap: "このセッションの要約", title: "セッションタイトルを表示/設定",
    usage: "このセッションのトークン使用量", skillFallback: "スキル",
    resume: "最近のセッションを再開（デスクトップ）",
  },
  boot: {
    spawnFailed: "ゲートウェイの起動に失敗：{error}",
    unreachable: "ゲートウェイに到達できません — 設定でゲートウェイ URL と API キーを確認してください。",
    unreachableDetail: "ゲートウェイが起動するとデスクトップシェルは /health をポーリングします。管理モードは ulnclaw バイナリが PATH にあれば自動的に起動します。",
    connecting: "接続中", starting: "ulnclaw ゲートウェイを起動中…",
    failureTitle: "デスクトップの起動に失敗", retry: "再試行",
    openSettings: "設定を開く", dismiss: "閉じる",
  },
  bridge: {
    preview: "プレビュー：{label}",
    terminalClosed: "ターミナルを閉じました：{id}{running}",
    stillRunning: "（まだ実行中）", terminalEmpty: "ターミナルペインは空です",
  },
  kanban: {
    todo: "未着手", doing: "進行中", done: "完了", blocked: "ブロック中",
    addTask: "+ タスクを追加…", addComment: "コメントを追加…", comment: "コメント",
    unblock: "ブロック解除", blockEllipsis: "ブロック…", complete: "完了", close: "閉じる",
    blockAction: "⛔ ブロック", unblockAction: "↩ ブロック解除", doneAction: "✓ 完了",
    whyBlocked: "ブロックの理由は？", refresh: "更新", switchBoard: "ボードを切り替え",
    counts: "未完了 {open} · 合計 {total}", noDescription: "（説明なし）",
    resultPrefix: "結果：{result}", noComments: "まだコメントはありません。",
  },
  projects: {
    addFolder: "フォルダを追加", archive: "アーカイブ", restore: "復元",
    bindBoard: "ボードを紐付け", rebindBoard: "ボードを再紐付け", delete: "削除",
    makePrimary: "プライマリに設定", primaryFolder: "プライマリフォルダ",
    noActiveProject: "有効なプロジェクトなし", scanRepos: "リポジトリをスキャン",
    scanning: "スキャン中…", archived: "アーカイブ済み", toProject: "→ プロジェクト化",
    createFromRepo: "このリポジトリからプロジェクトを作成", removeFolder: "フォルダを削除",
    scanTitle: "ファイルシステムから git リポジトリをスキャン",
    boardSlugPrompt: "ボード slug（空で紐付け解除）：", folderPathPrompt: "フォルダパス：",
    scanRootsPrompt: "スキャンルート（カンマ区切り；空 = ホームディレクトリ）：",
    newProject: "新規プロジェクト", discoveredRepos: "発見されたリポジトリ",
    nameLabel: "名前", foldersLabel: "フォルダ（カンマ区切り；最初 = プライマリ）",
    boardLabel: "カンバンボードを紐付け（任意の slug）", setActive: "有効なプロジェクトに設定",
    create: "作成", use: "使用",
    empty: "まだプロジェクトがありません — 作成するか、ファイルシステムから git リポジトリをスキャンしてください。",
    reposEmpty: "発見キャッシュが空です — 「リポジトリをスキャン」を実行してください。",
    deleteConfirm: "プロジェクト「{name}」を削除しますか？レジストリエントリのみ削除されます。",
    boardBadge: "ボード：{slug}",
    scanRecorded: "{count} 件のリポジトリを発見キャッシュに記録しました。",
    createFailed: "プロジェクトの作成に失敗（ゲートウェイ到達不能または入力が無効）。",
    activePrefix: "有効：{name}",
  },
  jobs: {
    active: "有効", paused: "一時停止中", pause: "一時停止", resume: "再開",
    runNow: "今すぐ実行", delete: "削除", edit: "プロンプト/スケジュールを編集",
    promptPrompt: "プロンプト：", schedulePrompt: "スケジュール：",
    whatShouldAgentDo: "エージェントに何をさせますか？",
    fromNow: "後", ago: "前", newJob: "新規ジョブ", newCronJob: "新規 cron ジョブ",
    nameLabel: "名前",
    scheduleLabel: "スケジュール（cron 式、@every 30m、@at unix-ts）",
    promptLabel: "プロンプト", skillsLabel: "スキル（カンマ区切り、任意）",
    repeatLabel: "繰り返し（残り実行回数；空 = 無期限）", create: "作成",
    createFailed: "ジョブの作成に失敗（ゲートウェイ到達不能またはスケジュールが無効）。",
    counts: "有効 {active} / 合計 {total} ジョブ",
    empty: "まだ cron ジョブがありません — 作成するか、ターミナルで `ulnclaw cron add` を使用してください。",
    meta: "次回：{next} · 前回：{last}", runsLeft: " · 残り {count} 回",
    deleteConfirm: "ジョブ「{name}」を削除しますか？",
  },
  hatch: {
    title: "\u{1F95A} ペットをふ化する", styleLabel: "スタイル ", draftsLabel: "草稿数 ",
    designing: "ベースの外見を設計中…", drawing: "アニメーション行を描画中…",
    pickBase: "気に入ったベースの外見を選んでください — すべてのアニメーション行の基準になります。",
    cancelHatch: "ふ化をキャンセル", startOver: "やり直す", tryAgain: "再試行",
    failedToLoad: "読み込みに失敗", loadingSpritesheet: "スプライトシートを読み込み中…",
    previewUnavailable: "スプライトシート预览は利用できません",
    hatch: "ふ化", done: "完了", close: "閉じる",
    namePlaceholder: "名前（任意）", gatewayOffline: "ゲートウェイがオフライン",
    stylePixelDefault: "ピクセルアート（hermes 既定）", stylePixel: "ピクセルアート",
    styleFlat: "フラットベクター", styleGlossy: "光沢ステッカー",
    stylePainterly: "絵画的", styleClay: "クレイメーション", stylePlush: "ぬいぐるみ", style3d: "3D おもちゃ",
    intro: "ペットを説明してください。画像モデルがベースの外見をスケッチし、ひとつを選ぶと、ふ化パイプラインがすべてのアニメーション行を描画します（数分）。",
    promptPlaceholder: "ネオンアクセントの小さなサイバーフォックス",
    draftOne: "{count} 草稿", draftMany: "{count} 草稿",
    errorNoResult: "ふ化は結果なく終了しました", errorCancelled: "ふ化をキャンセルしました",
    errorFailed: "ふ化に失敗しました",
    hatched: "(^_^)b {name} がふ化して家族に！",
    rowsMeta: "アニメーション {count} 行 — まもなくコーナーに表示されます。",
    draftAlt: "草稿 {index}",
  },
  picker: {
    title: "\u{1F9E0} このセッションのモデル", loading: "モデル一覧を読み込み中…",
    notConnected: "ゲートウェイ未接続。", loadFailed: "モデルの読み込みに失敗：{error}",
    gatewayDefault: "（ゲートウェイ既定）",
    lockNote: "\u{1F512} このセッションは {model} にロックされています。",
    noProviders: "ゲートウェイから provider が報告されていません。", noModels: "モデルがありません",
    notAuthenticatedTitle: "provider が未認証",
    notAuthenticatedBit: "⚠ 未認証", currentBit: "現在",
    lockFailed: "ロックに失敗：{error}",
    visibilityTitle: "表示するモデル", visibilitySearch: "モデルを絞り込み…",
    visibilityEmpty: "認証済み provider からモデルが報告されていません。",
    addProvider: "provider を追加…", editVisibleModels: "表示モデルを編集…",
    resetVisibility: "既定に戻す",
  },
  find: {
    placeholder: "チャット内を検索…", closeTitle: "閉じる（Esc）",
    nextTitle: "次へ（Enter）", prevTitle: "前へ（Shift+Enter）",
  },
  palette: {
    placeholder: "コマンドを入力…（Esc で閉じる）", noMatches: "一致するコマンドがありません",
    navigate: "移動", sessionGroup: "セッション", sessionsGroup: "セッション一覧",
    gatewayGroup: "ゲートウェイ", goToChat: "チャットへ", goToKanban: "カンバンへ",
    goToProjects: "プロジェクトへ", goToJobs: "ジョブ（cron）へ",
    newSession: "新規セッション", switchSession: "セッションを切り替え",
    findInChat: "チャット内を検索", modelForSession: "このセッションのモデル…",
    resumeSession: "セッションを再開…（/resume）",
    renameSession: "セッション名を変更…", deleteSession: "セッションを削除…",
    browseArtifacts: "成果物を閲覧…", learningGraph: "学習グラフ…",
    openSettings: "ゲートウェイ設定を開く…", refreshSessions: "セッション一覧を更新",
    hintFreshChat: "新しいチャットを開始", hintArtifacts: "リンク、ファイル、画像",
    hintLearning: "学習済みスキル + メモリ", switchTo: "切り替え先：{title}",
  },
  artifacts: {
    title: "\u{1F5C2}️ 成果物", filterPlaceholder: "成果物を絞り込み…",
    notConnected: "ゲートウェイ未接続。",
    none: "最近のセッションに成果物が見つかりません。",
    scanning: "最近のセッションをスキャン中…", openSession: "セッションを開く",
  },
  learning: {
    title: "✨ 学習", tagline: "学習済みスキル + メモリ、リンク付き",
    searchPlaceholder: "ノードを検索…", building: "学習グラフを構築中…",
    loading: "読み込み中…", notConnected: "ゲートウェイ未接続。",
    noMatches: "一致するノードがありません。", save: "保存", saved: "保存しました。",
    archive: "アーカイブ", archived: "アーカイブしました。", delete: "削除", deleted: "削除しました。",
    close: "閉じる",
  },
  notify: { dismiss: "通知を閉じる", clearAll: "すべてクリア", details: "詳細", stackTitle: "通知" },
  onboarding: {
    welcomeTitle: "ulnclaw へようこそ",
    intro: "ulnclaw は Hermes Agent エンジンの Rust 再実装です：50 以上のツール、スキル、定時ジョブ、メッセージングゲートウェイ、ローカル HTTP ゲートウェイ — このデスクトップシェルは純 HTTP/SSE でゲートウェイと通信します。",
    bullet1: "チャットはトークンをライブストリーミングし、ツール進行カードを表示します。",
    bullet2: "カンバン、プロジェクト、ジョブのダッシュボードはサイドバータブにあります。",
    bullet3: "Ctrl/Cmd+K でコマンドパレット、Ctrl/Cmd+F でチャット内検索。",
    skip: "スキップ", getStarted: "始める", finish: "完了",
    providersTitle: "モデル provider", loadingProviders: "provider 一覧を読み込み中…",
    noInventory: "ゲートウェイが provider 一覧を返しませんでした。続行は可能です — ~/.ulnclaw/config.toml の [model] を設定してゲートウェイを再起動してください。",
    currentModel: "現在のモデル：{model}（{provider}）",
    recheck: "provider を再チェック", active: "✓ 使用中", configured: "✓ 設定済み",
    needsEnv: "{env} が必要", notConfigured: "未設定",
    needsEnvTitle: "{env}（環境変数または ~/.ulnclaw/.env）を設定してから再チェックしてください。",
  },
  language: {
    switchTo: "言語を切り替え", searchPlaceholder: "言語を検索…",
    noResults: "一致する言語がありません", description: "デスクトップシェルの表示言語を選択します。",
  },
  sessionPicker: {
    title: "セッション", searchPlaceholder: "セッションを検索…",
    noResults: "一致するセッションがありません", messages: "{count} メッセージ",
  },
  intro: {
    headline1: "今日は何を動かしますか？",
    body1: "バグ、ブランチ、計画、荒いアイデアをどうぞ。リポジトリを調べ、次の具体的なステップにします。",
    headline2: "何を考えていますか？",
    body2: "コードや質問、詰まっている部分をどうぞ。変更前に状況を読み取ります。",
    headline3: "ulnclaw に何を見せますか？",
    body3: "タスクや失敗パス、途中の計画をどうぞ。実行に移すのを手伝います。",
    headline4: "どこから始めますか？",
    body4: "問題や目標、ファイルをどうぞ。まず調べ、次のステップを具体的に保ちます。",
    headline5: "何が必要ですか？",
    body5: "手持ちの文脈をどうぞ。計画や修正に整理します。",
  },
};

const ar: Translations = {
  chrome: {
    chatTab: "الدردشة", kanbanTab: "كانبان", projectsTab: "المشاريع", jobsTab: "المهام",
    newSession: "جلسة جديدة", settings: "الإعدادات", gatewayStatus: "حالة البوابة",
    hatchPet: "\u{1F95A} فقّس حيوانًا أليفًا",
    selectOrStart: "اختر جلسة أو ابدأ واحدة",
    inputPlaceholder: "راسل ulnclaw… (Enter للإرسال، Shift+Enter لسطر جديد)",
    send: "إرسال",
    settingsTitle: "إعدادات البوابة", gatewayUrl: "عنوان البوابة",
    apiKey: "مفتاح API (اختياري، [gateway] key)", bearerToken: "رمز bearer",
    manageProcess: "إدارة عملية البوابة (تشغيل/إيقاف مع التطبيق)",
    replayOnboarding: "إعادة عرض التهيئة", cancel: "إلغاء", save: "حفظ",
  },
  session: {
    titlePrompt: "عنوان الجلسة:", renamed: "أُعيدت تسمية الجلسة.",
    renameFailed: "فشلت إعادة التسمية: {error}",
    deleteConfirm: "حذف الجلسة «{label}» وسجل محادثتها؟",
    deleteFailed: "فشل الحذف: {error}", newTitle: "جلسة جديدة",
    loadFailed: "تعذّر تحميل الرسائل: {error}",
    createFailed: "تعذّر إنشاء الجلسة: {error}",
    errorPrefix: "خطأ: {error}",
    modelLockTitle: "قفل نموذج الجلسة — انقر للتغيير",
    gatewayModelTitle: "نموذج البوابة الافتراضي — انقر لاختيار نموذج الجلسة",
    reachable: "البوابة متاحة", unreachable: "البوابة غير متاحة",
    removeAttachment: "إزالة المرفق",
    uploadFailed: "فشل رفع الحافظة: {error}",
    projectBadge: "المشروع: {project}",
  },
  tools: { running: "قيد التشغيل…", done: "تم", arguments: "المعاملات", result: "النتيجة", fallbackName: "أداة" },
  slash: {
    help: "أوامر البوابة المائلة", skills: "قائمة المهارات", tools: "قائمة الأدوات المفعّلة",
    recap: "تلخيص هذه الجلسة", title: "عرض عنوان الجلسة أو تعيينه",
    usage: "استخدام الرموز لهذه الجلسة", skillFallback: "مهارة",
    resume: "استئناف جلسة حديثة (سطح المكتب)",
  },
  boot: {
    spawnFailed: "فشل تشغيل البوابة: {error}",
    unreachable: "البوابة غير متاحة — تحقق من عنوان البوابة ومفتاح API في الإعدادات.",
    unreachableDetail: "يستطلع سطح المكتب /health بمجرد عمل البوابة؛ ويطلقها الوضع المُدار تلقائيًا عندما يكون ملف ulnclaw على PATH.",
    connecting: "جارٍ الاتصال", starting: "جارٍ تشغيل بوابة ulnclaw…",
    failureTitle: "فشل تشغيل سطح المكتب", retry: "إعادة المحاولة",
    openSettings: "فتح الإعدادات", dismiss: "تجاهل",
  },
  bridge: {
    preview: "معاينة: {label}",
    terminalClosed: "أُغلق الطرفي: {id}{running}",
    stillRunning: " (لا يزال يعمل)", terminalEmpty: "لوحة الطرفي فارغة",
  },
  kanban: {
    todo: "قيد الانتظار", doing: "قيد التنفيذ", done: "منجز", blocked: "محجوب",
    addTask: "+ أضف مهمة…", addComment: "أضف تعليقًا…", comment: "تعليق",
    unblock: "رفع الحجب", blockEllipsis: "حجب…", complete: "إنجاز", close: "إغلاق",
    blockAction: "⛔ حجب", unblockAction: "↩ رفع الحجب", doneAction: "✓ إنجاز",
    whyBlocked: "لماذا هي محجوبة؟", refresh: "تحديث", switchBoard: "تبديل اللوحة",
    counts: "{open} مفتوحة · {total} إجمالًا", noDescription: "(بدون وصف)",
    resultPrefix: "النتيجة: {result}", noComments: "لا تعليقات بعد.",
  },
  projects: {
    addFolder: "إضافة مجلد", archive: "أرشفة", restore: "استعادة",
    bindBoard: "ربط لوحة", rebindBoard: "إعادة ربط اللوحة", delete: "حذف",
    makePrimary: "تعيين كأساسي", primaryFolder: "المجلد الأساسي",
    noActiveProject: "لا يوجد مشروع نشط", scanRepos: "فحص المستودعات",
    scanning: "جارٍ الفحص…", archived: "مؤرشف", toProject: "→ تحويل لمشروع",
    createFromRepo: "إنشاء مشروع من هذا المستودع", removeFolder: "إزالة المجلد",
    scanTitle: "فحص نظام الملفات بحثًا عن مستودعات git",
    boardSlugPrompt: "معرّف اللوحة (الفراغ يفك الربط):", folderPathPrompt: "مسار المجلد:",
    scanRootsPrompt: "جذور الفحص (مفصولة بفواصل؛ فارغ = المجلد الرئيسي):",
    newProject: "مشروع جديد", discoveredRepos: "المستودعات المكتشفة",
    nameLabel: "الاسم", foldersLabel: "المجلدات (مفصولة بفواصل؛ الأول = الأساسي)",
    boardLabel: "ربط لوحة kanban (معرّف اختياري)", setActive: "تعيين كمشروع نشط",
    create: "إنشاء", use: "استخدام",
    empty: "لا مشاريع بعد — أنشئ واحدًا أو افحص نظام الملفات بحثًا عن مستودعات git.",
    reposEmpty: "ذاكرة الاكتشاف فارغة — شغّل «فحص المستودعات».",
    deleteConfirm: "حذف المشروع «{name}»؟ سيُزال قيد السجل فقط.",
    boardBadge: "اللوحة: {slug}",
    scanRecorded: "سُجّلت {count} مستودعات في ذاكرة الاكتشاف.",
    createFailed: "فشل إنشاء المشروع (البوابة غير متاحة أو الإدخال غير صالح).",
    activePrefix: "النشط: {name}",
  },
  jobs: {
    active: "نشط", paused: "متوقف مؤقتًا", pause: "إيقاف مؤقت", resume: "استئناف",
    runNow: "تشغيل الآن", delete: "حذف", edit: "تعديل المطالب/الجدولة",
    promptPrompt: "المطالب:", schedulePrompt: "الجدولة:",
    whatShouldAgentDo: "ماذا يجب أن يفعل الوكيل؟",
    fromNow: "من الآن", ago: "مضت", newJob: "مهمة جديدة", newCronJob: "مهمة cron جديدة",
    nameLabel: "الاسم",
    scheduleLabel: "الجدولة (تعبير cron أو @every 30m أو @at unix-ts)",
    promptLabel: "المطالب", skillsLabel: "المهارات (مفصولة بفواصل، اختيارية)",
    repeatLabel: "التكرار (مرات التشغيل المتبقية؛ فارغ = للأبد)", create: "إنشاء",
    createFailed: "فشل إنشاء المهمة (البوابة غير متاحة أو الجدولة غير صالحة).",
    counts: "{active} نشطة / {total} مهام",
    empty: "لا مهام cron بعد — أنشئ واحدة أو استخدم `ulnclaw cron add` في الطرفي.",
    meta: "التالية: {next} · الأخيرة: {last}", runsLeft: " · {count} تشغيلات متبقية",
    deleteConfirm: "حذف المهمة «{name}»؟",
  },
  hatch: {
    title: "\u{1F95A} فقّس حيوانًا أليفًا", styleLabel: "النمط ", draftsLabel: "المسودات ",
    designing: "جارٍ تصميم المظاهر الأساسية…", drawing: "جارٍ رسم صفوف الحركة…",
    pickBase: "اختر المظهر الأساسي الذي يعجبك — فهو أساس كل صف حركي.",
    cancelHatch: "إلغاء الفقس", startOver: "البدء من جديد", tryAgain: "إعادة المحاولة",
    failedToLoad: "فشل التحميل", loadingSpritesheet: "جارٍ تحميل ورقة الصور…",
    previewUnavailable: "معاينة ورقة الصور غير متاحة",
    hatch: "فقّس", done: "تم", close: "إغلاق",
    namePlaceholder: "الاسم (اختياري)", gatewayOffline: "البوابة غير متصلة",
    stylePixelDefault: "فن البكسل (افتراضي hermes)", stylePixel: "فن البكسل",
    styleFlat: "متجهات مسطحة", styleGlossy: "ملصق لامع",
    stylePainterly: "أسلوب تصويري", styleClay: "صلصال متحرك", stylePlush: "لعبة قطيفة", style3d: "لعبة ثلاثية الأبعاد",
    intro: "صف حيوانًا أليفًا؛ يرسم نموذج الصور مظاهر أساسية، تختار أحدها، ثم يرسم خط الفقس كل صف حركي (بضع دقائق).",
    promptPlaceholder: "ثعلب سيبراني صغير بلمسات نيون",
    draftOne: "مسودة واحدة", draftMany: "{count} مسودات",
    errorNoResult: "انتهى الفقس دون نتيجة", errorCancelled: "أُلغي الفقس",
    errorFailed: "فشل الفقس",
    hatched: "(^_^)b فقّس {name} وتم تبنّيه!",
    rowsMeta: "{count} صفوف حركية — ستظهر في الزاوية قريبًا.",
    draftAlt: "مسودة {index}",
  },
  picker: {
    title: "\u{1F9E0} نموذج هذه الجلسة", loading: "جارٍ تحميل قائمة النماذج…",
    notConnected: "البوابة غير متصلة.", loadFailed: "فشل تحميل النماذج: {error}",
    gatewayDefault: "(افتراضي البوابة)",
    lockNote: "\u{1F512} هذه الجلسة مقفلة على {model}.",
    noProviders: "لم تُبلّغ البوابة عن أي provider.", noModels: "لا توجد نماذج",
    notAuthenticatedTitle: "provider غير مصادَق",
    notAuthenticatedBit: "⚠ غير مصادَق", currentBit: "الحالي",
    lockFailed: "فشل القفل: {error}",
    visibilityTitle: "النماذج المرئية", visibilitySearch: "تصفية النماذج…",
    visibilityEmpty: "لم تُبلّغ أي موارد مصادَق عنها نماذج.",
    addProvider: "إضافة provider…", editVisibleModels: "تحرير النماذج المرئية…",
    resetVisibility: "إعادة تعيين للافتراضي",
  },
  find: {
    placeholder: "ابحث في الدردشة…", closeTitle: "إغلاق (Esc)",
    nextTitle: "التطابق التالي (Enter)", prevTitle: "التطابق السابق (Shift+Enter)",
  },
  palette: {
    placeholder: "اكتب أمرًا… (Esc للإغلاق)", noMatches: "لا أوامر مطابقة",
    navigate: "تنقّل", sessionGroup: "الجلسة", sessionsGroup: "الجلسات",
    gatewayGroup: "البوابة", goToChat: "إلى الدردشة", goToKanban: "إلى كانبان",
    goToProjects: "إلى المشاريع", goToJobs: "إلى المهام (cron)",
    newSession: "جلسة جديدة", switchSession: "تبديل الجلسة",
    findInChat: "البحث في الدردشة", modelForSession: "نموذج هذه الجلسة…",
    resumeSession: "استئناف جلسة… (/resume)",
    renameSession: "إعادة تسمية الجلسة…", deleteSession: "حذف الجلسة…",
    browseArtifacts: "تصفح المخرجات…", learningGraph: "رسم التعلم…",
    openSettings: "فتح إعدادات البوابة…", refreshSessions: "تحديث قائمة الجلسات",
    hintFreshChat: "بدء دردشة جديدة", hintArtifacts: "روابط وملفات وصور",
    hintLearning: "مهارات متعلَّمة + ذاكرة", switchTo: "التبديل إلى: {title}",
  },
  artifacts: {
    title: "\u{1F5C2}️ المخرجات", filterPlaceholder: "تصفية المخرجات…",
    notConnected: "البوابة غير متصلة.",
    none: "لا مخرجات في الجلسات الأخيرة.",
    scanning: "جارٍ فحص الجلسات الأخيرة…", openSession: "فتح الجلسة",
  },
  learning: {
    title: "✨ التعلم", tagline: "مهارات متعلَّمة + ذاكرة، مترابطة",
    searchPlaceholder: "ابحث في العقد…", building: "جارٍ بناء رسم التعلم…",
    loading: "جارٍ التحميل…", notConnected: "البوابة غير متصلة.",
    noMatches: "لا عقد مطابقة.", save: "حفظ", saved: "تم الحفظ.",
    archive: "أرشفة", archived: "تمت الأرشفة.", delete: "حذف", deleted: "تم الحذف.",
    close: "إغلاق",
  },
  notify: { dismiss: "إغلاق الإشعار", clearAll: "مسح الكل", details: "التفاصيل", stackTitle: "الإشعارات" },
  onboarding: {
    welcomeTitle: "مرحبًا بك في ulnclaw",
    intro: "ulnclaw إعادة تنفيذ بـ Rust لمحرك Hermes Agent: أكثر من 50 أداة، ومهارات، ومهام مجدولة، وبوابات رسائل، وبوابة HTTP محلية — تتحدث هذه الواجهة مع البوابة عبر HTTP/SSE مباشرة.",
    bullet1: "الدردشة تبث الرموز مباشرة مع بطاقات تقدم الأدوات.",
    bullet2: "لوحات كانبان والمشاريع والمهام في ألسنة الشريط الجانبي.",
    bullet3: "Ctrl/Cmd+K يفتح لوحة الأوامر؛ Ctrl/Cmd+F يبحث في الدردشة.",
    skip: "تخطي", getStarted: "ابدأ", finish: "إنهاء",
    providersTitle: "موارد النماذج", loadingProviders: "جارٍ تحميل قائمة الموارد…",
    noInventory: "لم تُرجع البوابة قائمة الموارد. يمكنك المتابعة — اضبط [model] في ~/.ulnclaw/config.toml وأعد تشغيل البوابة.",
    currentModel: "النموذج الحالي: {model} ({provider})",
    recheck: "إعادة فحص الموارد", active: "✓ نشط", configured: "✓ مضبوط",
    needsEnv: "يحتاج {env}", notConfigured: "غير مضبوط",
    needsEnvTitle: "اضبط {env} (متغير بيئة أو ~/.ulnclaw/.env) ثم أعد الفحص.",
  },
  language: {
    switchTo: "تبديل اللغة", searchPlaceholder: "ابحث عن لغة…",
    noResults: "لا لغات مطابقة", description: "اختر لغة العرض لواجهة سطح المكتب.",
  },
  sessionPicker: {
    title: "الجلسات", searchPlaceholder: "البحث في الجلسات…",
    noResults: "لا جلسات مطابقة", messages: "{count} رسائل",
  },
  intro: {
    headline1: "ماذا ننجز اليوم؟",
    body1: "أرسل خللًا أو فرعًا أو خطة أو فكرة أولية؛ سأفحص المستودع وأحوّلها إلى خطوة ملموسة.",
    headline2: "بمَ تفكر؟",
    body2: "أحضر الكود أو السؤال أو الجزء العالق؛ سأقرأ السياق قبل إجراء التغييرات.",
    headline3: "ماذا يجب أن ينظر ulnclaw؟",
    body3: "أرسل المهمة أو المسار الفاشل أو الخطة غير المكتملة؛ سأساعد في تحويلها إلى تنفيذ.",
    headline4: "من أين نبدأ؟",
    body4: "أحضر المشكلة أو الهدف أو الملف؛ سأفحص أولًا وأبقي الخطوة التالية ملموسة.",
    headline5: "ما الذي يحتاج انتباهًا؟",
    body5: "أرسل ما لديك من سياق؛ سأساعد في ترتيبه في خطة أو إصلاح.",
  },
};

const CATALOGS: Record<Locale, Translations> = { en, zh, "zh-hant": zhHant, ja, ar };

const STORAGE_KEY = "***";
const RTL_LOCALES = new Set<Locale>(["ar"]);

let current: Locale = normalizeLocale(((): string | null => {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
})());

const listeners = new Set<() => void>();

/** Resolve a dot path (`kanban.todo`) against a catalog. */
function lookup(catalog: Translations, path: string): string | null {
  let node: unknown = catalog;
  for (const part of path.split(".")) {
    if (typeof node !== "object" || node === null || !(part in node)) return null;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === "string" ? node : null;
}

/**
 * Live translation accessor: `t.kanban.todo` always reads the active
 * locale's catalog, so render code never caches stale strings.
 */
export const t = new Proxy({} as Translations, {
  get: (_target, prop: string) => (CATALOGS[current] as unknown as Record<string, unknown>)[prop],
});

export function currentLocale(): Locale {
  return current;
}

export function setLocale(next: Locale): void {
  if (next === current) return;
  current = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // storage unavailable — choice lasts for this run only
  }
  document.documentElement.lang = next;
  document.documentElement.dir = RTL_LOCALES.has(next) ? "rtl" : "ltr";
  applyStatic();
  for (const listener of listeners) listener();
}

/** Subscribe to locale changes (re-render dynamic DOM). */
export function onLocaleChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Translate every `data-i18n` (textContent), `data-i18n-ph` (placeholder)
 * and `data-i18n-title` (title) element currently in the document. Values
 * are dot paths into the catalog; missing keys leave the source text.
 */
export function applyStatic(root: ParentNode = document): void {
  document.documentElement.lang = current;
  document.documentElement.dir = RTL_LOCALES.has(current) ? "rtl" : "ltr";
  const catalog = CATALOGS[current];
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((node) => {
    const value = lookup(catalog, node.dataset.i18n || "");
    if (value !== null) node.textContent = value;
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-ph]").forEach((node) => {
    const value = lookup(catalog, node.dataset.i18nPh || "");
    if (value !== null) node.setAttribute("placeholder", value);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((node) => {
    const value = lookup(catalog, node.dataset.i18nTitle || "");
    if (value !== null) node.title = value;
  });
}
