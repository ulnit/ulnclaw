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
    chatTab: string; kanbanTab: string; projectsTab: string; jobsTab: string; usageTab: string; configTab: string; doctorTab: string; webhooksTab: string; runsTab: string; skillsTab: string; sessionsTab: string; modelsTab: string; pluginsTab: string; pairingTab: string;
    newSession: string; settings: string; gatewayStatus: string; hatchPet: string;
    selectOrStart: string; inputPlaceholder: string; send: string;
    micTitle: string; micRecording: string; micFailed: string;
    attachTitle: string; fsTitle: string; fsUpTitle: string; fsEmpty: string; fsFailed: string;
    fsDownloadTitle: string; fsMkdirTitle: string; fsMkdirPrompt: string;
    settingsTitle: string; gatewayUrl: string; apiKey: string; bearerToken: string;
    manageProcess: string; replayOnboarding: string; cancel: string; save: string;
    restartGateway: string; restartDone: string; restartFailed: string; restartUnavailable: string;
    settingsTheme: string; settingsFont: string;
  };
  session: {
    titlePrompt: string; renamed: string; renameFailed: string; deleteConfirm: string;
    deleteFailed: string; exported: string; exportFailed: string; newTitle: string; loadFailed: string; createFailed: string;
    errorPrefix: string; modelLockTitle: string; gatewayModelTitle: string;
    reachable: string; unreachable: string; removeAttachment: string; uploadFailed: string; projectBadge: string;
    speakTitle: string; speakFailed: string;
  };
  tools: { running: string; done: string; thinking: string; arguments: string; result: string; fallbackName: string };
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
    claim: string; metaAssignee: string; metaPriority: string; metaCreated: string;
    metaStarted: string; metaCompleted: string; metaParents: string; metaChildren: string;
    attachmentsTitle: string;
    dispatch: string; dispatchResult: string; dispatchFailed: string;
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
    rename: string; renamePrompt: string; editAbout: string; aboutPrompt: string;
    descriptionLabel: string; iconLabel: string;
  };
  jobs: {
    active: string; paused: string; pause: string; resume: string; runNow: string;
    delete: string; edit: string; promptPrompt: string; schedulePrompt: string;
    whatShouldAgentDo: string;
    fromNow: string; ago: string; newJob: string; newCronJob: string; nameLabel: string;
    scheduleLabel: string; promptLabel: string; skillsLabel: string; repeatLabel: string; deliverLabel: string;
    create: string; createFailed: string; counts: string; empty: string; meta: string;
    runsLeft: string; deleteConfirm: string;
    deliverBadge: string; deliverTitle: string; deliveryError: string; deliverPrompt: string;
  };
  usage: {
    windowNote: string; perSession: string; empty: string;
    totalTokens: string; input: string; output: string;
    sessions: string; messages: string; processTokens: string;
    prompt: string; completion: string; toolCalls: string;
    requests: string; runs: string; completed: string; failed: string;
    colSession: string; colModel: string; colMessages: string;
    colInput: string; colOutput: string; colTotal: string; colStarted: string;
  };
  insights: {
    title: string; days7: string; days30: string; days90: string; sourcePlaceholder: string;
    sessions: string; messages: string; toolCalls: string; tokens: string;
    estCost: string; avgSession: string; activeDays: string;
    topModels: string; topTools: string; topSessions: string;
    colModel: string; colTool: string; colSession: string; calls: string;
    empty: string; loadFailed: string;
  };
  sessionsView: {
    filterPlaceholder: string; count: string; empty: string; noMatch: string;
    select: string; loading: string; loadFailed: string; transcriptFailed: string;
    emptyTranscript: string; exportTitle: string; exportFailed: string;
    roleUser: string; roleAssistant: string; roleTool: string; roleSystem: string;
    recap: string; recapTitle: string; recapFailed: string;
    forkTitle: string; forked: string; forkFailed: string;
    deleteTitle: string; deleteConfirm: string; deleted: string; deleteFailed: string;
    searchPlaceholder: string; noResults: string; searchFailed: string;
    renameTitle: string; renamePrompt: string; renamed: string; renameFailed: string;
    exportHtmlTitle: string;
    msgCount: string; project: string; source: string;
    stats: string;
    prune: string; archive: string; pruneTitle: string; archiveTitle: string;
    pruneDialogTitle: string; archiveDialogTitle: string; olderThanLabel: string;
    sourceLabel: string; includeArchived: string; preview: string; apply: string;
    previewCount: string; previewEmpty: string; appliedPruned: string; appliedArchived: string;
    confirmPrune: string; confirmArchive: string; failed: string;
  };
  modelsView: {
    count: string; current: string; catalog: string; providersLower: string; stale: string;
    none: string; loadFailed: string; currentBadge: string; authenticated: string;
    unauthenticated: string; docs: string; noModels: string;
    colModel: string; colFamily: string; colContext: string; colMaxOut: string;
    colCaps: string; colPrice: string;
    usageTitle: string; usageEmpty: string; usageSessions: string; usageMessages: string; usageTokens: string;
    gatewayTitle: string; gatewayContext: string; gatewaySet: string; gatewaySetConfirm: string; gatewaySetDone: string; gatewaySetFailed: string;
    endpointsTitle: string; endpointsEmpty: string; endpointsTest: string; endpointsActivate: string; endpointsActivated: string; endpointsDeleteConfirm: string; endpointsSaved: string; endpointsFailed: string;
  };
  pluginsView: {
    count: string; none: string; loadFailed: string; hooksWord: string; toolsWord: string;
    disabledBadge: string; enable: string; disable: string; noConfigHooks: string;
    toggleFailed: string; configHooksTitle: string;
    hooksRevoke: string; hooksAcceptAll: string; hooksAllowlist: string;
  };
  pairingView: {
    count: string; none: string; loadFailed: string; clearPending: string; lockedOut: string;
    pendingTitle: string; approvedTitle: string; age: string; approve: string; revoke: string;
    emptyPlatform: string; approvedNote: string; approveFailed: string; revokedNote: string;
    revokeFailed: string; clearedNote: string;
  };
  config: {
    loading: string; notConnected: string; loadFailed: string; save: string;
    reload: string; saving: string; saved: string; saveFailed: string;
    addKey: string; keyPlaceholder: string; valuePlaceholder: string; add: string;
    removeTitle: string; redactedNote: string; envKeys: string; envKeysNote: string;
    restartNote: string; noKeys: string; noChanges: string; pending: string;
    rawButton: string; rawTitle: string; rawSave: string; rawConfirm: string; rawSaved: string; rawFailed: string;
    envAddLabel: string; envValuePlaceholder: string; envEmpty: string; envFile: string;
    envProcess: string; envBoth: string; envRemoveTitle: string; envRemoveConfirm: string; envRevealTitle: string;
    envSaved: string; envFailed: string;
    memoryTitle: string; memoryTargetAll: string; memoryTargetMemory: string;
    memoryTargetUser: string; memoryReset: string; memoryNote: string;
    memoryMissing: string; memoryEntries: string; memoryLimit: string;
    memoryResetConfirm: string; memoryResetDone: string; memoryResetNone: string;
    memoryResetFailed: string;
    poolTitle: string; poolAddLabel: string; poolEmpty: string;
    poolRemoveConfirm: string; poolSaved: string; poolFailed: string; poolNote: string;
    oauthTitle: string; oauthLoggedIn: string; oauthLoggedOut: string; oauthPortal: string; oauthNote: string;
    schemaTitle: string; schemaNote: string;
    messagingTitle: string; messagingNote: string; messagingEnable: string; messagingDisable: string; messagingTest: string; messagingFailed: string; messagingSaveEnv: string; messagingClearEnv: string;
    ttsTitle: string; ttsNote: string; ttsPreview: string; ttsSample: string; ttsPreviewFailed: string; ttsVoicesUnavailable: string; ttsVoicesUnauthorized: string;
  };
  doctor: {
    run: string; running: string; online: string; issues: string;
    noIssues: string; failed: string; empty: string;
  };
  webhooks: {
    count: string; empty: string; loadFailed: string; createTitle: string;
    name: string; namePh: string; description: string; descriptionPh: string;
    events: string; eventsPh: string; deliver: string; deliverChat: string;
    deliverOnly: string; prompt: string; promptPh: string; skills: string;
    script: string; scriptPh: string; secret: string; create: string;
    test: string; copy: string; delete: string; direct: string; allEvents: string;
    copied: string; copyFailed: string; removed: string; removeFailed: string;
    testing: string; testFailed: string; createFailed: string;
  };
  monitoring: {
    title: string; healthExport: string; metrics: string; diagnosticEvents: string;
    warningLogs: string; otlpEndpoint: string; otlpNotConfigured: string;
    queueDepth: string; installId: string; on: string; off: string;
  };
  runs: {
    count: string; empty: string; loadFailed: string; stop: string; stopping: string;
    result: string; approvalTitle: string; approveOnce: string; approveSession: string;
    approveAlways: string; deny: string; approveFailed: string; stopFailed: string;
    delegationsTitle: string; noDelegations: string; loading: string; noResult: string;
    approvalWaitingTitle: string; approvalWaitingBody: string; viewRuns: string;
    timelineTitle: string;
  };
  skillsView: {
    count: string; skillsTitle: string; toolsetsTitle: string; noSkills: string;
    noToolsets: string; loadFailed: string; enabled: string; disabled: string; tools: string;
    curationTitle: string; archivedTitle: string; pinSkill: string; unpinSkill: string;
    archiveSkill: string; restoreSkill: string; archiveConfirm: string; curationFailed: string;
  };
  browserPanel: {
    title: string; configured: string; backend: string; mode: string; source: string;
    endpoint: string; available: string; vnc: string; managedRunning: string;
  };
  logsPanel: { title: string; allLevels: string; searchPlaceholder: string; };
  mcpPanel: { title: string; none: string; oauthTokens: string; oauthPending: string; connect: string; connecting: string; openAuth: string; approved: string; failed: string; toolsCached: string; };
  kanbanPanel: { title: string; none: string; openOf: string; current: string; byStatus: string; blocked: string; };
  storagePanel: { title: string; size: string; contents: string; counts: string; path: string; optimize: string; optimizeTitle: string; optimizing: string; optimized: string; optimizeFailed: string; };
  systemPanel: { title: string; version: string; platform: string; uptime: string; contents: string; sessionsWord: string; messagesWord: string; runsWord: string; jobs: string; enabledWord: string; disabledWord: string; plugins: string; home: string; config: string; desktopManaged: string; };
  metricsPanel: { title: string; summary: string; };
  egressPanel: { title: string; };
  channelsPanel: { title: string; enabled: string; disabled: string; noneEnabled: string; test: string; stateConnected: string; stateNotConfigured: string; };
  learningPanel: { title: string; skills: string; memoryNodes: string; edges: string; skillEdgesWord: string; memoryEdgesWord: string; density: string; linked: string; isolated: string; origin: string; agentCreatedWord: string; usedWord: string; categories: string; topCategories: string; hint: string; };
  backupsPanel: { title: string; empty: string; newSnapshot: string; labelPrompt: string; created: string; createFailed: string; restore: string; restoreConfirm: string; restored: string; restoreFailed: string; prune: string; prunePrompt: string; pruned: string; pruneFailed: string; };
  checkpointsPanel: { title: string; size: string; noProjects: string; prune: string; prunePrompt: string; pruned: string; pruneFailed: string; };
  opsPanel: { title: string; securityAudit: string; promptSize: string; dump: string; running: string; auditClean: string; failed: string; };
  updatePanel: { title: string; check: string; apply: string; checking: string; applying: string; upToDate: string; behind: string; behindShallow: string; checkFailed: string; applyConfirm: string; applyDone: string; applyFailed: string; };
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
    goToProjects: string; goToJobs: string; goToUsage: string; goToConfig: string; goToDoctor: string; goToWebhooks: string; goToRuns: string; goToSkills: string; goToSessions: string; goToModels: string; goToPlugins: string; goToPairing: string; newSession: string; switchSession: string;
    findInChat: string; modelForSession: string; resumeSession: string; renameSession: string;
    deleteSession: string; exportMd: string; exportHtml: string; browseArtifacts: string; learningGraph: string;
    openSettings: string; refreshSessions: string; restartGateway: string;
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
    chatTab: "Chat", kanbanTab: "Kanban", projectsTab: "Projects", jobsTab: "Jobs", usageTab: "Usage", configTab: "Config", doctorTab: "Doctor", webhooksTab: "Webhooks", runsTab: "Runs", skillsTab: "Skills", sessionsTab: "Sessions", modelsTab: "Models", pluginsTab: "Plugins", pairingTab: "Pairing",
    newSession: "New session", settings: "Settings", gatewayStatus: "gateway status",
    hatchPet: "\u{1F95A} Hatch pet",
    selectOrStart: "Select or start a session",
    inputPlaceholder: "Message ulnclaw… (Enter to send, Shift+Enter for newline)",
    send: "Send",
    micTitle: "Voice input (record and transcribe)",
    micRecording: "Recording\u2026 click to stop",
    micFailed: "Voice input failed: {error}",
    attachTitle: "Attach a file from the gateway filesystem", fsTitle: "Attach a file", fsUpTitle: "Up one directory", fsEmpty: "Empty directory", fsFailed: "File browser failed: {error}", fsDownloadTitle: "Download this file", fsMkdirTitle: "New folder", fsMkdirPrompt: "New folder name:",
    settingsTitle: "Gateway settings", gatewayUrl: "Gateway URL",
    apiKey: "API key (optional, [gateway] key)", bearerToken: "bearer token",
    manageProcess: "Manage the gateway process (start/stop with the app)",
    replayOnboarding: "Replay onboarding", cancel: "Cancel", save: "Save", restartGateway: "Restart gateway", restartDone: "Gateway restarted.", restartFailed: "Gateway restart timed out.", restartUnavailable: "The gateway is not managed here — restart it where it runs.",
    settingsTheme: "Theme", settingsFont: "Font",
  },
  session: {
    titlePrompt: "Session title:", renamed: "Session renamed.",
    renameFailed: "Rename failed: {error}",
    deleteConfirm: "Delete session \"{label}\" and its transcript?",
    deleteFailed: "Delete failed: {error}",
    exported: "Exported {filename}", exportFailed: "Export failed: {error}",
    newTitle: "New session",
    loadFailed: "Could not load messages: {error}",
    createFailed: "Could not create a session: {error}",
    errorPrefix: "error: {error}",
    modelLockTitle: "Session model lock — click to change",
    gatewayModelTitle: "Gateway default model — click to pick a session model",
    reachable: "gateway reachable", unreachable: "gateway unreachable",
    removeAttachment: "Remove attachment",
    uploadFailed: "Clipboard upload failed: {error}", speakTitle: "Read aloud (TTS)", speakFailed: "Speech synthesis failed: {error}",
    projectBadge: "Project: {project}",
  },
  tools: { running: "running…", done: "done", thinking: "thinking", arguments: "arguments", result: "result", fallbackName: "tool" },
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
    claim: "Claim", metaAssignee: "Assignee", metaPriority: "Priority", metaCreated: "Created",
    metaStarted: "Started", metaCompleted: "Completed", metaParents: "Parents", metaChildren: "Children",
    attachmentsTitle: "Attachments",
    dispatch: "Dispatch", dispatchResult: "Dispatched: {spawned} spawned · {promoted} promoted · {reclaimed} reclaimed", dispatchFailed: "Dispatch failed: {error}",
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
    rename: "Rename", renamePrompt: "New name:", editAbout: "Edit about", aboutPrompt: "Description (empty clears):",
    descriptionLabel: "Description (optional)", iconLabel: "Icon emoji (optional)",
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
    repeatLabel: "Repeat (runs remaining; empty = forever)", create: "Create", deliverLabel: "Deliver result to",
    createFailed: "Job creation failed (gateway unreachable or invalid schedule).",
    counts: "{active} active / {total} jobs",
    empty: "No cron jobs yet — create one, or use `ulnclaw cron add` in the terminal.",
    meta: "next: {next} · last: {last}", runsLeft: " · {count} run(s) left",
    deleteConfirm: "Delete job “{name}”?",
    deliverBadge: "\u2192 {target}", deliverTitle: "Delivery target for job results",
    deliveryError: "last delivery failed (hover for details)",
    deliverPrompt: "Delivery target (local/origin/platform; empty clears):",
  },
  usage: {
    windowNote: "Token accounting · last {count} sessions",
    perSession: "Per-session breakdown",
    empty: "No sessions recorded yet.",
    totalTokens: "Total tokens (store)", input: "in", output: "out",
    sessions: "Sessions", messages: "messages", processTokens: "Gateway tokens",
    prompt: "prompt", completion: "completion", toolCalls: "Tool calls",
    requests: "API requests", runs: "Async runs", completed: "completed", failed: "failed",
    colSession: "Session", colModel: "Model", colMessages: "Msgs",
    colInput: "Input", colOutput: "Output", colTotal: "Total", colStarted: "Started",
  },
  insights: {
    title: "Insights",
    days7: "Last 7 days", days30: "Last 30 days", days90: "Last 90 days", sourcePlaceholder: "Filter by source (cli, gateway, …)",
    sessions: "Sessions", messages: "Messages", toolCalls: "Tool calls", tokens: "Tokens",
    estCost: "Est. cost", avgSession: "Avg session", activeDays: "active days:",
    topModels: "Top models", topTools: "Top tools", topSessions: "Top sessions",
    colModel: "Model", colTool: "Tool", colSession: "Session", calls: "Calls",
    empty: "No activity recorded for this window.",
    loadFailed: "Failed to load insights: {error}",
  },
  sessionsView: {
    filterPlaceholder: "Filter sessions…",
    count: "{count} sessions",
    empty: "No sessions recorded yet.",
    noMatch: "No sessions match the filter.",
    select: "Select a session to browse its transcript.",
    loading: "Loading transcript…",
    loadFailed: "Failed to load sessions: {error}",
    transcriptFailed: "Failed to load transcript: {error}",
    emptyTranscript: "This session has no messages.",
    exportTitle: "Export selected session as Markdown", exportHtmlTitle: "Export selected session as standalone HTML", msgCount: "{count} messages", project: "project", source: "source", stats: "{sessions} sessions \u00b7 {messages} messages \u00b7 {size} on disk",
    prune: "Prune…", archive: "Archive…", pruneTitle: "Delete ended sessions matching filters", archiveTitle: "Archive ended sessions matching filters",
    pruneDialogTitle: "Prune ended sessions", archiveDialogTitle: "Archive ended sessions",
    olderThanLabel: "Last activity older than (90d, 2026-01-01…)", sourceLabel: "Source filter (optional)",
    includeArchived: "Include already-archived sessions", preview: "Preview", apply: "Apply",
    previewCount: "{count} session(s) match — nothing changed yet.", previewEmpty: "No sessions match.",
    appliedPruned: "Pruned {count} session(s).", appliedArchived: "Archived {count} session(s) — recoverable, nothing deleted.",
    confirmPrune: "Really delete these {count} session(s)? This cannot be undone.", confirmArchive: "Archive these {count} session(s)?",
    failed: "Failed: {error}",
    exportFailed: "Export failed.",
    roleUser: "User", roleAssistant: "Assistant", roleTool: "Tool", roleSystem: "System",
    recap: "Recap", recapTitle: "Show or hide the gateway-built session recap", recapFailed: "Recap failed: {error}",
    forkTitle: "Fork this session into a new branch", forked: "Forked as {id}", forkFailed: "Fork failed: {error}",
    deleteTitle: "Delete this session", deleteConfirm: "Delete session {id}? This cannot be undone.", deleted: "Deleted {id}.", deleteFailed: "Delete failed: {error}",
    searchPlaceholder: "Full-text search transcripts…", noResults: "No transcript matches.", searchFailed: "Search failed: {error}",
    renameTitle: "Rename this session", renamePrompt: "New title (empty clears):", renamed: "Session renamed.", renameFailed: "Rename failed: {error}",
  },
  modelsView: {
    count: "{providers} providers", current: "Current", catalog: "catalog", providersLower: "providers", stale: "stale",
    none: "No providers configured.", loadFailed: "Failed to load model inventory: {error}",
    currentBadge: "current", authenticated: "authenticated", unauthenticated: "no credentials",
    docs: "Docs", noModels: "No models listed.",
    colModel: "Model", colFamily: "Family", colContext: "Context", colMaxOut: "Max out", colCaps: "Caps", colPrice: "$/Mtok",
    usageTitle: "Model usage (30 days)", usageEmpty: "No model usage recorded yet.",
    gatewayTitle: "Gateway model", gatewayContext: "context", gatewaySet: "Set as gateway model", gatewaySetConfirm: "Switch the gateway model to {provider}/{model}? Applies to new sessions once the gateway restarts.", gatewaySetDone: "Gateway model updated — restart the gateway to apply.", gatewaySetFailed: "Model switch failed: {error}", endpointsTitle: "Custom endpoints", endpointsEmpty: "No custom endpoints configured.", endpointsTest: "Test", endpointsActivate: "Activate", endpointsActivated: "Custom endpoint activated — restart the gateway to apply.", endpointsDeleteConfirm: "Delete the {id} endpoint and its stored key?", endpointsSaved: "Endpoint saved.", endpointsFailed: "Endpoint operation failed: {error}",
    usageSessions: "sessions", usageMessages: "msgs", usageTokens: "tokens",
  },
  pluginsView: {
    count: "{count} plugins", none: "No plugins found — install a plugin directory with a plugin.toml manifest into ~/.ulnclaw/plugins.",
    loadFailed: "Failed to load plugins: {error}", hooksWord: "hooks", toolsWord: "tools",
    disabledBadge: "disabled", enable: "Enable", disable: "Disable",
    noConfigHooks: "No [hooks] shell hooks configured.", toggleFailed: "Toggle failed: {error}",
    configHooksTitle: "Config shell hooks",
    hooksRevoke: "Revoke", hooksAcceptAll: "Accept all pending",
    hooksAllowlist: "Consent allowlist: {count} entries",
  },
  pairingView: {
    count: "{platforms} platform(s) · {pending} pending", none: "No pairing activity yet — unknown senders who DM an enabled bot receive a pairing code.",
    loadFailed: "Pairing request failed: {error}", clearPending: "Clear pending", lockedOut: "locked out",
    pendingTitle: "Pending", approvedTitle: "Approved", age: "{minutes}m old", approve: "Approve", revoke: "Revoke",
    emptyPlatform: "No pending or approved pairings.", approvedNote: "Approved {code}.",
    approveFailed: "Approve failed: {error}", revokedNote: "Revoked {user}.",
    revokeFailed: "Revoke failed: {error}", clearedNote: "Cleared {count} pending code(s).",
  },
  config: {
    loading: "Loading config…", notConnected: "Gateway not connected.",
    loadFailed: "Failed to load config: {error}",
    save: "Save", reload: "Reload", saving: "Saving…",
    saved: "Saved {count} change(s). Restart the gateway to apply.",
    saveFailed: "Save failed: {error}",
    addKey: "Add key", keyPlaceholder: "dotted.key.path",
    valuePlaceholder: "value (JSON or text)", add: "Add",
    removeTitle: "Remove this key",
    redactedNote: "Masked values are secrets; saving an unchanged masked value keeps the original.",
    envKeys: "Environment keys (.env)", envKeysNote: "Names only — edit the .env file to change values.",
    restartNote: "Edits apply to new processes; restart the gateway to apply them here.",
    noKeys: "config.toml is empty.", noChanges: "Nothing to save.",
    pending: "{count} unsaved change(s)",
    rawButton: "Raw TOML\u2026", rawTitle: "Raw config.toml", rawSave: "Save raw",
    rawConfirm: "Replace config.toml with this exact text? Restart the gateway to apply.",
    rawSaved: "Raw config saved. Restart the gateway to apply.",
    rawFailed: "Raw save failed: {error}",
    envAddLabel: "Add env key", envValuePlaceholder: "value (stored in .env)",
    envEmpty: "No environment keys found.", envFile: ".env", envProcess: "process env",
    envBoth: ".env + process env", envRemoveTitle: "Remove this key from .env",
    envRemoveConfirm: "Remove {key} from .env?",
    envRevealTitle: "Reveal value (5 per 30 s)",
    envSaved: "Environment updated. Restart the gateway to apply.",
    envFailed: "Environment change failed: {error}",
    memoryTitle: "Persistent memory",
    memoryTargetAll: "All (MEMORY.md + USER.md)", memoryTargetMemory: "MEMORY.md only",
    memoryTargetUser: "USER.md only", memoryReset: "Reset\u2026",
    memoryNote: "Bullet entries the agent keeps across sessions; reset deletes the files irreversibly.",
    memoryMissing: "not created yet", memoryEntries: "entries", memoryLimit: "limit",
    memoryResetConfirm: "Delete the selected memory files? This cannot be undone.",
    memoryResetDone: "Deleted: {files}", memoryResetNone: "Nothing to delete.",
    memoryResetFailed: "Reset failed: {error}",
    poolTitle: "Credential pool", poolAddLabel: "Add pool key", poolEmpty: "No pooled credentials", poolRemoveConfirm: "Remove this pooled key?", poolSaved: "Credential pool updated", poolFailed: "Credential pool failed: {error}", poolNote: "Pooled keys rotate round-robin per request and take precedence over environment variables for their provider; remove every entry to fall back to env keys.", oauthTitle: "OAuth (device flow)", oauthLoggedIn: "Logged in", oauthLoggedOut: "Logged out", oauthPortal: "Open portal", oauthNote: "Read-only posture of the [oauth] device-flow login (ulnclaw oauth CLI); tokens live in oauth_tokens.json.", schemaTitle: "Config schema (defaults)", schemaNote: "Every config leaf with its type and default value — edit via the rows above or Raw TOML.", messagingTitle: "Messaging platforms", messagingNote: "Enable/disable toggles write [messaging.<id>].enabled; restart the gateway to apply. Credentials live in config.toml (telegram/discord/slack also honor env keys).", messagingEnable: "Enable", messagingDisable: "Disable", messagingTest: "Test", messagingFailed: "Platform update failed: {error}", messagingSaveEnv: "Save", messagingClearEnv: "Clear", ttsTitle: "Text-to-speech", ttsNote: "Provider and voice for the 🔊 read-aloud action ([tts] config). Voice list needs ELEVENLABS_API_KEY; synthesis needs the provider key.", ttsPreview: "Preview", ttsSample: "Hello! This is your ulnclaw gateway speaking.", ttsPreviewFailed: "Preview failed: {error}", ttsVoicesUnavailable: "voice list unavailable (no key?)", ttsVoicesUnauthorized: "voice list unauthorized — check ELEVENLABS_API_KEY",
  },
  doctor: {
    run: "Run doctor", running: "Running checks…",
    online: "Include provider connectivity probes (slow)",
    issues: "Issues found", noIssues: "✓ No issues found.",
    failed: "Doctor run failed: {error}", empty: "No checks reported.",
  },
  webhooks: {
    count: "{count} subscription(s)", empty: "No dynamic webhook subscriptions yet.",
    loadFailed: "Failed to load subscriptions: {error}", createTitle: "New subscription",
    name: "Name", namePh: "build-events", description: "Description", descriptionPh: "CI notifications",
    events: "Events", eventsPh: "push, ci (empty = all)", deliver: "Deliver target",
    deliverChat: "Deliver chat id (optional)",
    deliverOnly: "Direct delivery (no agent, zero LLM cost)",
    prompt: "Prompt / message", promptPh: "Summarize this event…",
    skills: "Skills (comma-separated)", script: "Script (optional)", scriptPh: "./handle.sh",
    secret: "Secret (blank = auto-mint)", create: "Create",
    test: "Test", copy: "Copy URL", delete: "Delete", direct: "direct", allEvents: "(all)",
    copied: "URL copied to clipboard.", copyFailed: "Clipboard copy failed.",
    removed: "Removed subscription {name}.", removeFailed: "Remove failed: {error}",
    testing: "Firing signed test payload…", testFailed: "Test failed: {error}",
    createFailed: "Create failed: {error}",
  },
  monitoring: {
    title: "Gateway monitoring", healthExport: "Health export",
    metrics: "Metrics", diagnosticEvents: "Diagnostic events",
    warningLogs: "Warning/error logs", otlpEndpoint: "OTLP endpoint",
    otlpNotConfigured: "not configured", queueDepth: "Emitter queue depth",
    installId: "Install id", on: "on", off: "off",
  },
  runs: {
    count: "{count} runs · {active} active", empty: "No async runs tracked yet.",
    loadFailed: "Failed to load runs: {error}", stop: "Stop", stopping: "Stopping…",
    result: "Result", approvalTitle: "Approval requested",
    approveOnce: "Once", approveSession: "Session", approveAlways: "Always", deny: "Deny",
    approveFailed: "Approval failed: {error}", stopFailed: "Stop failed: {error}",
    delegationsTitle: "Delegations", noDelegations: "No async delegations dispatched yet.",
    approvalWaitingTitle: "Approval needed", approvalWaitingBody: "Run {id} is waiting for approval: {command}", viewRuns: "Open Runs",
    timelineTitle: "Live status timeline (SSE)",
    loading: "Loading…", noResult: "No result recorded.",
  },
  skillsView: {
    count: "{skills} skills · toolsets enabled {toolsets}",
    skillsTitle: "Installed skills", toolsetsTitle: "Toolsets",
    noSkills: "No skills installed in ~/.ulnclaw/skills yet.",
    noToolsets: "No toolsets reported.", loadFailed: "Failed to load: {error}",
    curationTitle: "Curation", archivedTitle: "Archived skills (recoverable):",
    pinSkill: "Pin", unpinSkill: "Unpin", archiveSkill: "Archive", restoreSkill: "Restore",
    archiveConfirm: "Archive skill {name}? It can be restored later.",
    curationFailed: "Curation action failed: {error}",
    enabled: "enabled", disabled: "disabled", tools: "Tools",
  },
  browserPanel: {
    title: "Browser (CDP)", configured: "Configured", backend: "Backend",
    mode: "Mode", source: "Source", endpoint: "Endpoint",
    available: "Available", vnc: "VNC URL", managedRunning: "Managed browser running",
  },
  logsPanel: { title: "Gateway log", allLevels: "All levels", searchPlaceholder: "search\u2026" },
  mcpPanel: { title: "MCP servers", none: "No MCP servers configured ([mcp] section).", oauthTokens: "oauth (tokens stored)", oauthPending: "oauth (not authorized)", connect: "Connect", connecting: "Starting…", openAuth: "Open authorization page", approved: "Authorized ✓", failed: "OAuth flow failed.", toolsCached: "{count} cached tools" },
  kanbanPanel: { title: "Kanban diagnostics", none: "No kanban boards configured.", openOf: "{open} open · {total} total", current: "current", byStatus: "Status counts", blocked: "Blocked tasks" },
  storagePanel: { title: "Session store", size: "Database size", contents: "Contents", counts: "{sessions} sessions · {messages} messages", path: "Path", optimize: "Optimize", optimizeTitle: "Merge FTS segments and VACUUM the session store (ulnclaw sessions optimize)", optimizing: "Optimizing…", optimized: "{indexes} index(es) merged · {before} → {after}", optimizeFailed: "Optimize failed: {error}" },
  systemPanel: { title: "System", version: "Version", platform: "Platform", uptime: "Uptime", contents: "Store", sessionsWord: "sessions", messagesWord: "messages", runsWord: "active runs", jobs: "Cron jobs", enabledWord: "enabled", disabledWord: "disabled", plugins: "Plugins", home: "Home", config: "Config", desktopManaged: "desktop-managed" },
  metricsPanel: { title: "Prometheus metrics", summary: "Show raw /metrics exposition" },
  egressPanel: { title: "Egress proxy" },
  channelsPanel: { title: "Messaging channels", enabled: "Enabled", disabled: "Disabled", noneEnabled: "(none)", test: "Test", stateConnected: "connected", stateNotConfigured: "not configured" },
  learningPanel: { title: "Learning graph", skills: "Learned skills", memoryNodes: "Memory chunks", edges: "Graph edges", skillEdgesWord: "skill\u2194skill", memoryEdgesWord: "memory\u2194skill", density: "Edge density", linked: "Linked nodes", isolated: "isolated", origin: "Origin", agentCreatedWord: "agent-created", usedWord: "used", categories: "Categories", topCategories: "Top categories", hint: "Open \u2728 Learning graph from the chat toolbar to browse, edit, and archive nodes." },
  backupsPanel: {
    title: "State snapshots", empty: "No quick snapshots yet.", newSnapshot: "New snapshot",
    labelPrompt: "Optional label for the snapshot:", created: "Snapshot {id} created.",
    createFailed: "Snapshot failed: {error}", restore: "Restore",
    restoreConfirm: "Restore snapshot {id}? Current state files are overwritten.",
    restored: "Snapshot {id} restored. Restart the gateway to pick up restored state.",
    restoreFailed: "Restore failed: {error}", prune: "Prune\u2026",
    prunePrompt: "Keep how many newest snapshots?", pruned: "Pruned {count} snapshot(s).",
    pruneFailed: "Prune failed: {error}",
  },
  checkpointsPanel: {
    title: "Checkpoints", size: "Store size",
    noProjects: "No checkpointed projects yet (checkpoints are opt-in: `[checkpoints] enabled = true`).",
    prune: "Prune\u2026", prunePrompt: "Retention window in days:",
    pruned: "Pruned: {orphan} orphan, {stale} stale; freed {bytes}.",
    pruneFailed: "Prune failed: {error}",
  },
  opsPanel: {
    title: "Ops actions", securityAudit: "Security audit", promptSize: "Prompt size",
    dump: "Debug dump", running: "Running {action}\u2026",
    auditClean: "No findings ({total} component(s) scanned).",
    failed: "{action} failed: {error}",
  },
  updatePanel: {
    title: "Update", check: "Check for updates", apply: "Apply update",
    checking: "Checking for updates\u2026", applying: "Applying update (fetch + rebuild)\u2026",
    upToDate: "Up to date", behind: "{count} commit(s) behind upstream (current: {version})",
    behindShallow: "Behind upstream by an unknown count (shallow clone)",
    checkFailed: "Update check failed: {error}",
    applyConfirm: "Apply the update now? This fast-forwards the checkout and rebuilds.",
    applyDone: "Updated: {commits} new commit(s), now at {sha}",
    applyFailed: "Update failed: {error}",
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
    goToProjects: "Go to Projects", goToJobs: "Go to Jobs (cron)", goToUsage: "Go to Usage", goToConfig: "Go to Config", goToDoctor: "Go to Doctor", goToWebhooks: "Go to Webhooks", goToRuns: "Go to Runs", goToSkills: "Go to Skills", goToSessions: "Go to Sessions", goToModels: "Go to Models", goToPlugins: "Go to Plugins", goToPairing: "Go to Pairing",
    newSession: "New session", switchSession: "Switch session",
    findInChat: "Find in chat", modelForSession: "Model for this session…",
    resumeSession: "Resume session… (/resume)",
    renameSession: "Rename session…", deleteSession: "Delete session…",
    exportMd: "Export session (Markdown)", exportHtml: "Export session (HTML)",
    browseArtifacts: "Browse artifacts…", learningGraph: "Learning graph…",
    openSettings: "Open gateway settings…", refreshSessions: "Refresh session list", restartGateway: "Restart gateway",
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
    title: "Sessions", searchPlaceholder: "Search sessions and messages…",
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
    chatTab: "聊天", kanbanTab: "看板", projectsTab: "项目", jobsTab: "任务", usageTab: "用量", configTab: "配置", doctorTab: "诊断", webhooksTab: "Webhooks", runsTab: "运行", skillsTab: "技能", sessionsTab: "会话记录", modelsTab: "模型", pluginsTab: "插件", pairingTab: "配对",
    newSession: "新建会话", settings: "设置", gatewayStatus: "网关状态",
    hatchPet: "\u{1F95A} 孵化宠物",
    selectOrStart: "选择或开始一个会话",
    inputPlaceholder: "给 ulnclaw 发消息…（Enter 发送，Shift+Enter 换行）",
    send: "发送",
    micTitle: "语音输入（录音并转写）",
    micRecording: "录音中…点击停止",
    micFailed: "语音输入失败：{error}",
    attachTitle: "从网关文件系统附加文件", fsTitle: "附加文件", fsUpTitle: "上一级目录", fsEmpty: "空目录", fsFailed: "文件浏览失败：{error}", fsDownloadTitle: "下载此文件", fsMkdirTitle: "新建文件夹", fsMkdirPrompt: "新文件夹名称：",
    settingsTitle: "网关设置", gatewayUrl: "网关 URL",
    apiKey: "API 密钥（可选，[gateway] key）", bearerToken: "bearer 令牌",
    manageProcess: "管理网关进程（随应用启动/停止）",
    replayOnboarding: "重放引导", cancel: "取消", save: "保存", restartGateway: "重启网关", restartDone: "网关已重启。", restartFailed: "网关重启超时。", restartUnavailable: "此环境不管理网关——请在网关运行处重启。",
    settingsTheme: "主题", settingsFont: "字体",
  },
  session: {
    titlePrompt: "会话标题：", renamed: "会话已重命名。",
    renameFailed: "重命名失败：{error}",
    deleteConfirm: "删除会话「{label}」及其对话记录？",
    deleteFailed: "删除失败：{error}",
    exported: "已导出 {filename}", exportFailed: "导出失败：{error}",
    newTitle: "新会话",
    loadFailed: "无法加载消息：{error}",
    createFailed: "无法创建会话：{error}",
    errorPrefix: "错误：{error}",
    modelLockTitle: "会话模型锁定 — 点击更改",
    gatewayModelTitle: "网关默认模型 — 点击挑选会话模型",
    reachable: "网关可达", unreachable: "网关不可达",
    removeAttachment: "移除附件",
    uploadFailed: "剪贴板上传失败：{error}", speakTitle: "朗读（TTS）", speakFailed: "语音合成失败：{error}",
    projectBadge: "项目：{project}",
  },
  tools: { running: "运行中…", done: "完成", thinking: "思考中", arguments: "参数", result: "结果", fallbackName: "工具" },
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
    claim: "认领", metaAssignee: "负责人", metaPriority: "优先级", metaCreated: "创建于",
    metaStarted: "开始于", metaCompleted: "完成于", metaParents: "父任务", metaChildren: "子任务",
    attachmentsTitle: "附件",
    dispatch: "派发", dispatchResult: "派发完成：生成 {spawned} · 提升 {promoted} · 回收 {reclaimed}", dispatchFailed: "派发失败:{error}",
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
    rename: "重命名", renamePrompt: "新名称：", editAbout: "编辑简介", aboutPrompt: "描述（留空清除）：",
    descriptionLabel: "描述（可选）", iconLabel: "图标 emoji（可选）",
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
    repeatLabel: "重复（剩余运行次数；留空 = 永远）", create: "创建", deliverLabel: "结果投递到",
    createFailed: "任务创建失败（网关不可达或调度无效）。",
    counts: "{active} 活跃 / 共 {total} 个任务",
    empty: "还没有定时任务 —— 创建一个，或在终端使用 `ulnclaw cron add`。",
    meta: "下次：{next} · 上次：{last}", runsLeft: " · 剩余 {count} 次运行",
    deleteConfirm: "删除任务「{name}」？",
    deliverBadge: "\u2192 {target}", deliverTitle: "定时任务结果的投递目标",
    deliveryError: "上次投递失败（悬停查看详情）",
    deliverPrompt: "投递目标（local/origin/平台名；留空清除）：",
  },
  usage: {
    windowNote: "令牌核算 · 最近 {count} 个会话",
    perSession: "按会话明细",
    empty: "尚无会话记录。",
    totalTokens: "总令牌（存储）", input: "入", output: "出",
    sessions: "会话数", messages: "条消息", processTokens: "网关令牌",
    prompt: "提示", completion: "补全", toolCalls: "工具调用",
    requests: "API 请求", runs: "异步运行", completed: "完成", failed: "失败",
    colSession: "会话", colModel: "模型", colMessages: "消息",
    colInput: "输入", colOutput: "输出", colTotal: "总计", colStarted: "开始时间",
  },
  insights: {
    title: "洞察",
    days7: "最近 7 天", days30: "最近 30 天", days90: "最近 90 天", sourcePlaceholder: "按来源过滤（cli、gateway…）",
    sessions: "会话数", messages: "消息数", toolCalls: "工具调用", tokens: "令牌数",
    estCost: "估算费用", avgSession: "平均会话时长", activeDays: "活跃天数:",
    topModels: "热门模型", topTools: "热门工具", topSessions: "热门会话",
    colModel: "模型", colTool: "工具", colSession: "会话", calls: "调用次数",
    empty: "该时间段内暂无活动记录。",
    loadFailed: "加载洞察失败:{error}",
  },
  sessionsView: {
    filterPlaceholder: "过滤会话…",
    count: "{count} 个会话",
    empty: "暂无会话记录。",
    noMatch: "没有匹配过滤条件的会话。",
    select: "选择一个会话以浏览其转录。",
    loading: "正在加载转录…",
    loadFailed: "加载会话列表失败:{error}",
    transcriptFailed: "加载转录失败:{error}",
    emptyTranscript: "该会话没有消息。",
    exportTitle: "将选中会话导出为 Markdown", exportHtmlTitle: "将选中会话导出为独立 HTML", msgCount: "{count} 条消息", project: "项目", source: "来源", stats: "{sessions} 个会话 \u00b7 {messages} 条消息 \u00b7 磁盘占用 {size}",
    prune: "清理…", archive: "归档…", pruneTitle: "按过滤条件删除已结束的会话", archiveTitle: "按过滤条件归档已结束的会话",
    pruneDialogTitle: "清理已结束的会话", archiveDialogTitle: "归档已结束的会话",
    olderThanLabel: "最后活动早于（90d、2026-01-01…）", sourceLabel: "来源过滤（可选）",
    includeArchived: "包含已归档会话", preview: "预览", apply: "应用",
    previewCount: "匹配 {count} 个会话——尚未更改。", previewEmpty: "没有匹配的会话。",
    appliedPruned: "已清理 {count} 个会话。", appliedArchived: "已归档 {count} 个会话——可恢复，未删除任何内容。",
    confirmPrune: "确定删除这 {count} 个会话？此操作不可撤销。", confirmArchive: "归档这 {count} 个会话？",
    failed: "失败：{error}",
    exportFailed: "导出失败。",
    roleUser: "用户", roleAssistant: "助手", roleTool: "工具", roleSystem: "系统",
    recap: "回顾", recapTitle: "显示/隐藏网关生成的会话回顾", recapFailed: "生成回顾失败:{error}",
    forkTitle: "将此会话分叉为新分支", forked: "已分叉为 {id}", forkFailed: "分叉失败:{error}",
    deleteTitle: "删除此会话", deleteConfirm: "删除会话 {id}？此操作无法撤销。", deleted: "已删除 {id}。", deleteFailed: "删除失败:{error}",
    searchPlaceholder: "全文搜索转录…", noResults: "没有匹配的转录。", searchFailed: "搜索失败:{error}",
    renameTitle: "重命名此会话", renamePrompt: "新标题（留空清除）：", renamed: "会话已重命名。", renameFailed: "重命名失败:{error}",
  },
  modelsView: {
    count: "{providers} 个 provider", current: "当前", catalog: "目录", providersLower: "个 provider", stale: "已过期",
    none: "未配置 provider。", loadFailed: "加载模型清单失败:{error}",
    currentBadge: "当前", authenticated: "已认证", unauthenticated: "无凭据",
    docs: "文档", noModels: "未列出模型。",
    colModel: "模型", colFamily: "家族", colContext: "上下文", colMaxOut: "最大输出", colCaps: "能力", colPrice: "$/Mtok",
    usageTitle: "模型用量（30 天）", usageEmpty: "尚无模型用量记录。",
    gatewayTitle: "网关模型", gatewayContext: "上下文", gatewaySet: "设为网关模型", gatewaySetConfirm: "将网关模型切换为 {provider}/{model}？网关重启后对新会话生效。", gatewaySetDone: "网关模型已更新——重启网关后生效。", gatewaySetFailed: "模型切换失败：{error}", endpointsTitle: "自定义端点", endpointsEmpty: "尚未配置自定义端点。", endpointsTest: "测试", endpointsActivate: "启用", endpointsActivated: "自定义端点已启用——重启网关后生效。", endpointsDeleteConfirm: "删除 {id} 端点及其存储的密钥？", endpointsSaved: "端点已保存。", endpointsFailed: "端点操作失败：{error}",
    usageSessions: "会话", usageMessages: "消息", usageTokens: "令牌",
  },
  pluginsView: {
    count: "{count} 个插件", none: "未发现插件——请将带 plugin.toml 清单的插件目录安装到 ~/.ulnclaw/plugins。",
    loadFailed: "加载插件失败:{error}", hooksWord: "钩子", toolsWord: "工具",
    disabledBadge: "已禁用", enable: "启用", disable: "禁用",
    noConfigHooks: "未配置 [hooks] 外壳钩子。", toggleFailed: "切换失败:{error}",
    configHooksTitle: "配置外壳钩子",
    hooksRevoke: "吹销", hooksAcceptAll: "批准全部待批",
    hooksAllowlist: "同意白名单：{count} 条",
  },
  pairingView: {
    count: "{platforms} 个平台 · {pending} 个待批", none: "暂无配对活动——向已启用机器人私聊的陌生发送者会收到配对码。",
    loadFailed: "配对请求失败:{error}", clearPending: "清除待批", lockedOut: "已锁定",
    pendingTitle: "待批准", approvedTitle: "已批准", age: "{minutes} 分钟前", approve: "批准", revoke: "吊销",
    emptyPlatform: "无待批或已批准的配对。", approvedNote: "已批准 {code}。",
    approveFailed: "批准失败:{error}", revokedNote: "已吊销 {user}。",
    revokeFailed: "吊销失败:{error}", clearedNote: "已清除 {count} 个待批配对码。",
  },
  config: {
    loading: "加载配置…", notConnected: "未连接网关。",
    loadFailed: "加载配置失败：{error}",
    save: "保存", reload: "重新加载", saving: "保存中…",
    saved: "已保存 {count} 项更改，重启网关后生效。",
    saveFailed: "保存失败：{error}",
    addKey: "添加键", keyPlaceholder: "点分键路径",
    valuePlaceholder: "值（JSON 或文本）", add: "添加",
    removeTitle: "删除此键",
    redactedNote: "打码的值是密钥；保存未改动的打码值将保留原密钥。",
    envKeys: "环境变量键（.env）", envKeysNote: "仅显示名称——修改 .env 文件才能改值。",
    restartNote: "修改对新进程生效；重启网关后在此生效。",
    noKeys: "config.toml 为空。", noChanges: "没有可保存的更改。",
    pending: "{count} 项未保存更改",
    rawButton: "原始 TOML\u2026", rawTitle: "原始 config.toml", rawSave: "保存原文",
    rawConfirm: "用此原文替换 config.toml？重启网关后生效。",
    rawSaved: "已保存原始配置。重启网关后生效。",
    rawFailed: "原文保存失败：{error}",
    envAddLabel: "添加环境变量", envValuePlaceholder: "值（存入 .env）",
    envEmpty: "未发现环境变量。", envFile: ".env", envProcess: "进程环境",
    envBoth: ".env + 进程环境", envRemoveTitle: "从 .env 移除此键",
    envRemoveConfirm: "从 .env 移除 {key}？",
    envRevealTitle: "显示值（每 30 秒限 5 次）",
    envSaved: "环境变量已更新。重启网关后生效。",
    envFailed: "环境变量修改失败：{error}",
    memoryTitle: "持久记忆",
    memoryTargetAll: "全部（MEMORY.md + USER.md）", memoryTargetMemory: "仅 MEMORY.md",
    memoryTargetUser: "仅 USER.md", memoryReset: "重置…",
    memoryNote: "智能体跨会话保留的条目记忆；重置会不可恢复地删除文件。",
    memoryMissing: "尚未创建", memoryEntries: "条", memoryLimit: "上限",
    memoryResetConfirm: "删除选定的记忆文件？此操作不可撤销。",
    memoryResetDone: "已删除：{files}", memoryResetNone: "无可删除内容。",
    memoryResetFailed: "重置失败：{error}",
    poolTitle: "凭证池", poolAddLabel: "添加池密钥", poolEmpty: "暂无池凭证", poolRemoveConfirm: "移除该池密钥？", poolSaved: "凭证池已更新", poolFailed: "凭证池操作失败：{error}", poolNote: "池内密钥按请求轮转，且对其 provider 优先于环境变量；删光条目即回落到环境密钥。", oauthTitle: "OAuth（设备流程）", oauthLoggedIn: "已登录", oauthLoggedOut: "未登录", oauthPortal: "打开门户", oauthNote: "[oauth] 设备流程登录的只读状态（ulnclaw oauth CLI）；令牌存于 oauth_tokens.json。", schemaTitle: "配置模式（默认值）", schemaNote: "列出每个配置叶子的类型与默认值——可通过上方字段或 Raw TOML 编辑。", messagingTitle: "消息平台", messagingNote: "启用/禁用开关写入 [messaging.<id>].enabled；重启网关后生效。凭证存于 config.toml（telegram/discord/slack 同时支持 env 键）。", messagingEnable: "启用", messagingDisable: "禁用", messagingTest: "测试", messagingFailed: "平台更新失败：{error}", messagingSaveEnv: "保存", messagingClearEnv: "清除", ttsTitle: "语音合成", ttsNote: "🔊 朗读动作所用 provider 与音色（[tts] 配置）。音色列表需 ELEVENLABS_API_KEY；合成需对应 provider 密钥。", ttsPreview: "试听", ttsSample: "你好！这里是你的 ulnclaw 网关在说话。", ttsPreviewFailed: "试听失败：{error}", ttsVoicesUnavailable: "音色列表不可用（未配置密钥？）", ttsVoicesUnauthorized: "音色列表鉴权失败——请检查 ELEVENLABS_API_KEY",
  },
  doctor: {
    run: "运行诊断", running: "检查中…",
    online: "包含 provider 连通性探测（较慢）",
    issues: "发现的问题", noIssues: "✓ 未发现问题。",
    failed: "诊断失败：{error}", empty: "没有检查项。",
  },
  webhooks: {
    count: "{count} 个订阅", empty: "还没有动态 webhook 订阅。",
    loadFailed: "加载订阅失败：{error}", createTitle: "新建订阅",
    name: "名称", namePh: "build-events", description: "描述", descriptionPh: "CI 通知",
    events: "事件", eventsPh: "push, ci（空 = 全部）", deliver: "投递目标",
    deliverChat: "投递会话 id（可选）",
    deliverOnly: "直接投递（不走 agent，零 LLM 成本）",
    prompt: "提示词 / 消息", promptPh: "总结这个事件…",
    skills: "技能（逗号分隔）", script: "脚本（可选）", scriptPh: "./handle.sh",
    secret: "密钥（留空自动生成）", create: "创建",
    test: "测试", copy: "复制 URL", delete: "删除", direct: "直接", allEvents: "（全部）",
    copied: "URL 已复制到剪贴板。", copyFailed: "复制到剪贴板失败。",
    removed: "已删除订阅 {name}。", removeFailed: "删除失败：{error}",
    testing: "发送签名测试载荷…", testFailed: "测试失败：{error}",
    createFailed: "创建失败：{error}",
  },
  monitoring: {
    title: "网关监控", healthExport: "健康导出",
    metrics: "指标", diagnosticEvents: "诊断事件",
    warningLogs: "警告/错误日志", otlpEndpoint: "OTLP 端点",
    otlpNotConfigured: "未配置", queueDepth: "发射队列深度",
    installId: "安装 ID", on: "开", off: "关",
  },
  runs: {
    count: "{count} 个运行 · {active} 活跃", empty: "还没有跟踪的异步运行。",
    loadFailed: "加载运行失败：{error}", stop: "停止", stopping: "停止中…",
    result: "结果", approvalTitle: "请求批准",
    approveOnce: "一次", approveSession: "本会话", approveAlways: "始终", deny: "拒绝",
    approveFailed: "批准失败：{error}", stopFailed: "停止失败：{error}",
    delegationsTitle: "委派", noDelegations: "还没有派发的异步委派。",
    approvalWaitingTitle: "需要批准", approvalWaitingBody: "运行 {id} 正在等待批准：{command}", viewRuns: "打开运行",
    timelineTitle: "实时状态时间线（SSE）",
    loading: "加载中…", noResult: "没有记录结果。",
  },
  skillsView: {
    count: "{skills} 个技能 · 已启用工具集 {toolsets}",
    skillsTitle: "已安装技能", toolsetsTitle: "工具集",
    noSkills: "~/.ulnclaw/skills 中还没有安装技能。",
    noToolsets: "没有报告的工具集。", loadFailed: "加载失败：{error}",
    curationTitle: "策展", archivedTitle: "已归档技能（可恢复）：",
    pinSkill: "固定", unpinSkill: "取消固定", archiveSkill: "归档", restoreSkill: "恢复",
    archiveConfirm: "归档技能 {name}？之后可以恢复。",
    curationFailed: "策展操作失败：{error}",
    enabled: "已启用", disabled: "未启用", tools: "工具",
  },
  browserPanel: {
    title: "浏览器（CDP）", configured: "已配置", backend: "后端",
    mode: "模式", source: "来源", endpoint: "端点",
    available: "可用", vnc: "VNC 地址", managedRunning: "托管浏览器运行中",
  },
  logsPanel: { title: "网关日志", allLevels: "全部级别", searchPlaceholder: "搜索…" },
  mcpPanel: { title: "MCP 服务器", none: "未配置 MCP 服务器（[mcp] 段）。", oauthTokens: "oauth（已存令牌）", oauthPending: "oauth（未授权）", connect: "连接", connecting: "启动中…", openAuth: "打开授权页面", approved: "已授权 ✓", failed: "OAuth 流程失败。", toolsCached: "{count} 个缓存工具" },
  kanbanPanel: { title: "看板诊断", none: "未配置看板。", openOf: "{open} 进行中 · 共 {total}", current: "当前", byStatus: "状态计数", blocked: "受阻任务" },
  storagePanel: { title: "会话存储", size: "数据库大小", contents: "内容", counts: "{sessions} 个会话 · {messages} 条消息", path: "路径", optimize: "优化", optimizeTitle: "合并 FTS 段并 VACUUM 会话存储（等同 ulnclaw sessions optimize）", optimizing: "优化中…", optimized: "已合并 {indexes} 个索引 · {before} → {after}", optimizeFailed: "优化失败:{error}" },
  systemPanel: { title: "系统", version: "版本", platform: "平台", uptime: "运行时长", contents: "存储", sessionsWord: "会话", messagesWord: "消息", runsWord: "活动运行", jobs: "定时任务", enabledWord: "启用", disabledWord: "禁用", plugins: "插件", home: "主目录", config: "配置", desktopManaged: "桌面托管" },
  metricsPanel: { title: "Prometheus 指标", summary: "显示 /metrics 原始输出" },
  egressPanel: { title: "出站代理" },
  channelsPanel: { title: "消息通道", enabled: "已启用", disabled: "未启用", noneEnabled: "（无）", test: "测试", stateConnected: "已连接", stateNotConfigured: "未配置" },
  learningPanel: { title: "学习图谱", skills: "已学技能", memoryNodes: "记忆条目", edges: "图谱边", skillEdgesWord: "技能↔技能", memoryEdgesWord: "记忆↔技能", density: "边密度", linked: "有关联节点", isolated: "孤立", origin: "来源", agentCreatedWord: "Agent 创建", usedWord: "已使用", categories: "分类", topCategories: "热门分类", hint: "在聊天工具栏打开 ✨ 学习图谱，可浏览、编辑和归档节点。" },
  backupsPanel: {
    title: "状态快照", empty: "还没有快速快照。", newSnapshot: "新建快照",
    labelPrompt: "快照标签（可选）：", created: "已创建快照 {id}。",
    createFailed: "创建快照失败：{error}", restore: "恢复",
    restoreConfirm: "恢复快照 {id}？当前状态文件将被覆盖。",
    restored: "已恢复快照 {id}。请重启网关以加载恢复的状态。",
    restoreFailed: "恢复失败：{error}", prune: "清理\u2026",
    prunePrompt: "保留多少个最新快照？", pruned: "已清理 {count} 个快照。",
    pruneFailed: "清理失败：{error}",
  },
  checkpointsPanel: {
    title: "检查点", size: "存储大小",
    noProjects: "尚无有检查点的项目（检查点为可选功能：`[checkpoints] enabled = true`）。",
    prune: "清理\u2026", prunePrompt: "保留窗口（天）：",
    pruned: "已清理：{orphan} 个孤儿、{stale} 个过期；释放 {bytes}。",
    pruneFailed: "清理失败：{error}",
  },
  opsPanel: {
    title: "运维操作", securityAudit: "安全审计", promptSize: "提示词体积",
    dump: "调试转储", running: "正在运行 {action}…",
    auditClean: "未发现问题（已扫描 {total} 个组件）。",
    failed: "{action} 失败：{error}",
  },
  updatePanel: {
    title: "更新", check: "检查更新", apply: "应用更新",
    checking: "正在检查更新…", applying: "正在应用更新（拉取 + 重新构建）…",
    upToDate: "已是最新版本", behind: "落后上游 {count} 个提交（当前：{version}）",
    behindShallow: "落后上游的提交数未知（浅克隆）",
    checkFailed: "更新检查失败：{error}",
    applyConfirm: "现在应用更新？将快进检出并重新构建。",
    applyDone: "已更新：新增 {commits} 个提交，当前 {sha}",
    applyFailed: "更新失败：{error}",
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
    goToProjects: "前往项目", goToJobs: "前往任务（cron）", goToUsage: "前往用量", goToConfig: "前往配置", goToDoctor: "前往诊断", goToWebhooks: "前往 Webhooks", goToRuns: "前往运行", goToSkills: "前往技能", goToSessions: "前往会话记录", goToModels: "前往模型", goToPlugins: "前往插件", goToPairing: "前往配对",
    newSession: "新建会话", switchSession: "切换会话",
    findInChat: "聊天内查找", modelForSession: "本会话模型…",
    resumeSession: "恢复会话…（/resume）",
    renameSession: "重命名会话…", deleteSession: "删除会话…",
    exportMd: "导出会话（Markdown）", exportHtml: "导出会话（HTML）",
    browseArtifacts: "浏览工件…", learningGraph: "学习图谱…",
    openSettings: "打开网关设置…", refreshSessions: "刷新会话列表", restartGateway: "重启网关",
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
    title: "会话", searchPlaceholder: "搜索会话与消息…",
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
    chatTab: "聊天", kanbanTab: "看板", projectsTab: "專案", jobsTab: "工作", usageTab: "用量", configTab: "設定", doctorTab: "診斷", webhooksTab: "Webhooks", runsTab: "執行", skillsTab: "技能", sessionsTab: "會話記錄", modelsTab: "模型", pluginsTab: "外掛", pairingTab: "配對",
    newSession: "新增工作階段", settings: "設定", gatewayStatus: "閘道狀態",
    hatchPet: "\u{1F95A} 孵化寵物",
    selectOrStart: "選擇或開始工作階段",
    inputPlaceholder: "傳送訊息給 ulnclaw…（Enter 傳送，Shift+Enter 換行）",
    send: "傳送",
    micTitle: "語音輸入（錄音並轉寫）",
    micRecording: "錄音中…點選停止",
    micFailed: "語音輸入失敗：{error}",
    attachTitle: "從閘道檔案系統附加檔案", fsTitle: "附加檔案", fsUpTitle: "上一層目錄", fsEmpty: "空目錄", fsFailed: "檔案瀏覽失敗：{error}", fsDownloadTitle: "下載此檔案", fsMkdirTitle: "新增資料夾", fsMkdirPrompt: "新資料夾名稱：",
    settingsTitle: "閘道設定", gatewayUrl: "閘道 URL",
    apiKey: "API 金鑰（選填，[gateway] key）", bearerToken: "bearer 權杖",
    manageProcess: "管理閘道程序（隨應用程式啟動/停止）",
    replayOnboarding: "重播引導", cancel: "取消", save: "儲存", restartGateway: "重啟閘道", restartDone: "閘道已重啟。", restartFailed: "閘道重啟逾時。", restartUnavailable: "此環境不管理閘道——請在閘道執行處重啟。",
    settingsTheme: "主題", settingsFont: "字型",
  },
  session: {
    titlePrompt: "工作階段標題：", renamed: "已重新命名工作階段。",
    renameFailed: "重新命名失敗：{error}",
    deleteConfirm: "刪除工作階段「{label}」及其對話紀錄？",
    deleteFailed: "刪除失敗：{error}",
    exported: "已匯出 {filename}", exportFailed: "匯出失敗：{error}",
    newTitle: "新工作階段",
    loadFailed: "無法載入訊息：{error}",
    createFailed: "無法建立工作階段：{error}",
    errorPrefix: "錯誤：{error}",
    modelLockTitle: "工作階段模型鎖定 — 點選以變更",
    gatewayModelTitle: "閘道預設模型 — 點選以挑選工作階段模型",
    reachable: "閘道可達", unreachable: "閘道不可達",
    removeAttachment: "移除附件",
    uploadFailed: "剪貼簿上傳失敗：{error}", speakTitle: "朗讀（TTS）", speakFailed: "語音合成失敗：{error}",
    projectBadge: "專案：{project}",
  },
  tools: { running: "執行中…", done: "完成", thinking: "思考中", arguments: "參數", result: "結果", fallbackName: "工具" },
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
    claim: "認領", metaAssignee: "負責人", metaPriority: "優先級", metaCreated: "建立於",
    metaStarted: "開始於", metaCompleted: "完成於", metaParents: "父工作", metaChildren: "子工作",
    attachmentsTitle: "附件",
    dispatch: "派發", dispatchResult: "派發完成：產生 {spawned} · 晉升 {promoted} · 回收 {reclaimed}", dispatchFailed: "派發失敗:{error}",
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
    rename: "重新命名", renamePrompt: "新名稱：", editAbout: "編輯簡介", aboutPrompt: "描述（留空清除）：",
    descriptionLabel: "描述（選填）", iconLabel: "圖示 emoji（選填）",
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
    repeatLabel: "重複（剩餘執行次數；留空 = 永遠）", create: "建立", deliverLabel: "結果投遞到",
    createFailed: "工作建立失敗（閘道不可達或排程無效）。",
    counts: "{active} 活躍 / 共 {total} 個工作",
    empty: "還沒有定時工作 —— 建立一個，或在終端機使用 `ulnclaw cron add`。",
    meta: "下次：{next} · 上次：{last}", runsLeft: " · 剩餘 {count} 次執行",
    deleteConfirm: "刪除工作「{name}」？",
    deliverBadge: "\u2192 {target}", deliverTitle: "定時任務結果的投遞目標",
    deliveryError: "上次投遞失敗（懸停檢視詳情）",
    deliverPrompt: "投遞目標（local/origin/平台名；留空清除）：",
  },
  usage: {
    windowNote: "令牌核算 · 最近 {count} 個工作階段",
    perSession: "按工作階段明細",
    empty: "尚無工作階段記錄。",
    totalTokens: "總令牌（儲存）", input: "入", output: "出",
    sessions: "工作階段數", messages: "則訊息", processTokens: "閘道令牌",
    prompt: "提示", completion: "補全", toolCalls: "工具呼叫",
    requests: "API 請求", runs: "非同步執行", completed: "完成", failed: "失敗",
    colSession: "工作階段", colModel: "模型", colMessages: "訊息",
    colInput: "輸入", colOutput: "輸出", colTotal: "總計", colStarted: "開始時間",
  },
  insights: {
    title: "洞察",
    days7: "最近 7 天", days30: "最近 30 天", days90: "最近 90 天", sourcePlaceholder: "按來源過濾（cli、gateway…）",
    sessions: "會話數", messages: "訊息數", toolCalls: "工具呼叫", tokens: "權杖數",
    estCost: "估算費用", avgSession: "平均會話時長", activeDays: "活躍天數:",
    topModels: "熱門模型", topTools: "熱門工具", topSessions: "熱門會話",
    colModel: "模型", colTool: "工具", colSession: "會話", calls: "呼叫次數",
    empty: "該時間段內暫無活動記錄。",
    loadFailed: "載入洞察失敗:{error}",
  },
  sessionsView: {
    filterPlaceholder: "過濾會話…",
    count: "{count} 個會話",
    empty: "暫無會話記錄。",
    noMatch: "沒有符合過濾條件的會話。",
    select: "選擇一個會話以瀏覽其轉錄。",
    loading: "正在載入轉錄…",
    loadFailed: "載入會話列表失敗:{error}",
    transcriptFailed: "載入轉錄失敗:{error}",
    emptyTranscript: "該會話沒有訊息。",
    exportTitle: "將選取會話匯出為 Markdown", exportHtmlTitle: "將選取會話匯出為獨立 HTML", msgCount: "{count} 則訊息", project: "專案", source: "來源", stats: "{sessions} 個會話 \u00b7 {messages} 則訊息 \u00b7 磁碟佔用 {size}",
    prune: "清理…", archive: "封存…", pruneTitle: "按過濾條件刪除已結束的會話", archiveTitle: "按過濾條件封存已結束的會話",
    pruneDialogTitle: "清理已結束的會話", archiveDialogTitle: "封存已結束的會話",
    olderThanLabel: "最後活動早於（90d、2026-01-01…）", sourceLabel: "來源過濾（選填）",
    includeArchived: "包含已封存會話", preview: "預覽", apply: "套用",
    previewCount: "匹配 {count} 個會話——尚未變更。", previewEmpty: "沒有匹配的會話。",
    appliedPruned: "已清理 {count} 個會話。", appliedArchived: "已封存 {count} 個會話——可復原，未刪除任何內容。",
    confirmPrune: "確定刪除這 {count} 個會話？此操作無法復原。", confirmArchive: "封存這 {count} 個會話？",
    failed: "失敗：{error}",
    exportFailed: "匯出失敗。",
    roleUser: "使用者", roleAssistant: "助理", roleTool: "工具", roleSystem: "系統",
    recap: "回顧", recapTitle: "顯示/隱藏閘道產生的會話回顧", recapFailed: "產生回顧失敗:{error}",
    forkTitle: "將此會話分叉為新分支", forked: "已分叉為 {id}", forkFailed: "分叉失敗:{error}",
    deleteTitle: "刪除此會話", deleteConfirm: "刪除會話 {id}？此操作無法復原。", deleted: "已刪除 {id}。", deleteFailed: "刪除失敗:{error}",
    searchPlaceholder: "全文搜尋轉錄…", noResults: "沒有符合的轉錄。", searchFailed: "搜尋失敗:{error}",
    renameTitle: "重新命名此會話", renamePrompt: "新標題（留空清除）：", renamed: "會話已重新命名。", renameFailed: "重新命名失敗:{error}",
  },
  modelsView: {
    count: "{providers} 個 provider", current: "目前", catalog: "目錄", providersLower: "個 provider", stale: "已過期",
    none: "未設定 provider。", loadFailed: "載入模型清單失敗:{error}",
    currentBadge: "目前", authenticated: "已認證", unauthenticated: "無憑證",
    docs: "文件", noModels: "未列出模型。",
    colModel: "模型", colFamily: "家族", colContext: "上下文", colMaxOut: "最大輸出", colCaps: "能力", colPrice: "$/Mtok",
    usageTitle: "模型用量（30 天）", usageEmpty: "尚無模型用量紀錄。",
    gatewayTitle: "閘道模型", gatewayContext: "內文窗口", gatewaySet: "設為閘道模型", gatewaySetConfirm: "將閘道模型切換為 {provider}/{model}？閘道重啟後對新工作階段生效。", gatewaySetDone: "閘道模型已更新——重啟閘道後生效。", gatewaySetFailed: "模型切換失敗：{error}", endpointsTitle: "自訂端點", endpointsEmpty: "尚未配置自訂端點。", endpointsTest: "測試", endpointsActivate: "啟用", endpointsActivated: "自訂端點已啟用——重啟閘道後生效。", endpointsDeleteConfirm: "刪除 {id} 端點及其儲存的密鑰？", endpointsSaved: "端點已儲存。", endpointsFailed: "端點操作失敗：{error}",
    usageSessions: "工作階段", usageMessages: "訊息", usageTokens: "記號",
  },
  pluginsView: {
    count: "{count} 個外掛", none: "未發現外掛——請將帶 plugin.toml 清單的外掛目錄安裝到 ~/.ulnclaw/plugins。",
    loadFailed: "載入外掛失敗:{error}", hooksWord: "掛鉤", toolsWord: "工具",
    disabledBadge: "已停用", enable: "啟用", disable: "停用",
    noConfigHooks: "未設定 [hooks] 殼層掛鉤。", toggleFailed: "切換失敗:{error}",
    configHooksTitle: "設定殼層掛鉤",
    hooksRevoke: "撤銷", hooksAcceptAll: "批准全部待批",
    hooksAllowlist: "同意白名單：{count} 條",
  },
  pairingView: {
    count: "{platforms} 個平台 · {pending} 個待批", none: "暫無配對活動——向已啟用機器人私訊的陌生傳送者會收到配對碼。",
    loadFailed: "配對請求失敗:{error}", clearPending: "清除待批", lockedOut: "已鎖定",
    pendingTitle: "待核准", approvedTitle: "已核准", age: "{minutes} 分鐘前", approve: "核准", revoke: "撤銷",
    emptyPlatform: "無待批或已核准的配對。", approvedNote: "已核准 {code}。",
    approveFailed: "核准失敗:{error}", revokedNote: "已撤銷 {user}。",
    revokeFailed: "撤銷失敗:{error}", clearedNote: "已清除 {count} 個待批配對碼。",
  },
  config: {
    loading: "載入設定…", notConnected: "未連線閘道。",
    loadFailed: "載入設定失敗：{error}",
    save: "儲存", reload: "重新載入", saving: "儲存中…",
    saved: "已儲存 {count} 項變更，重啟閘道後生效。",
    saveFailed: "儲存失敗：{error}",
    addKey: "新增鍵", keyPlaceholder: "點分鍵路徑",
    valuePlaceholder: "值（JSON 或文字）", add: "新增",
    removeTitle: "刪除此鍵",
    redactedNote: "遮罩的值是金鑰；儲存未改動的遮罩值將保留原金鑰。",
    envKeys: "環境變數鍵（.env）", envKeysNote: "僅顯示名稱——修改 .env 檔案才能改值。",
    restartNote: "修改對新程序生效；重啟閘道後在此生效。",
    noKeys: "config.toml 為空。", noChanges: "沒有可儲存的變更。",
    pending: "{count} 項未儲存變更",
    rawButton: "原始 TOML\u2026", rawTitle: "原始 config.toml", rawSave: "儲存原文",
    rawConfirm: "用此原文取代 config.toml？重啟閘道後生效。",
    rawSaved: "已儲存原始組態。重啟閘道後生效。",
    rawFailed: "原文儲存失敗：{error}",
    envAddLabel: "新增環境變數", envValuePlaceholder: "值（存入 .env）",
    envEmpty: "未發現環境變數。", envFile: ".env", envProcess: "程序環境",
    envBoth: ".env + 程序環境", envRemoveTitle: "從 .env 移除此鍵",
    envRemoveConfirm: "從 .env 移除 {key}？",
    envRevealTitle: "顯示值（每 30 秒限 5 次）",
    envSaved: "環境變數已更新。重啟閘道後生效。",
    envFailed: "環境變數修改失敗：{error}",
    memoryTitle: "持久記憶",
    memoryTargetAll: "全部（MEMORY.md + USER.md）", memoryTargetMemory: "僅 MEMORY.md",
    memoryTargetUser: "僅 USER.md", memoryReset: "重置…",
    memoryNote: "智慧體跨工作階段保留的條目記憶；重置會不可適回地刪除檔案。",
    memoryMissing: "尚未建立", memoryEntries: "條", memoryLimit: "上限",
    memoryResetConfirm: "刪除選定的記憶檔案？此操作無法復原。",
    memoryResetDone: "已刪除：{files}", memoryResetNone: "無可刪除內容。",
    memoryResetFailed: "重置失敗：{error}",
    poolTitle: "憑證池", poolAddLabel: "新增池金鑰", poolEmpty: "無池憑證", poolRemoveConfirm: "移除此池金鑰？", poolSaved: "憑證池已更新", poolFailed: "憑證池操作失敗：{error}", poolNote: "池內金鑰按請求輪換，且對其 provider 優先於環境變數；刪光條目即回落到環境密鑰。", oauthTitle: "OAuth（裝置流程）", oauthLoggedIn: "已登入", oauthLoggedOut: "未登入", oauthPortal: "開啟入口", oauthNote: "[oauth] 裝置流程登入的唯讀狀態（ulnclaw oauth CLI）；憑證存於 oauth_tokens.json。", schemaTitle: "組態結構（預設值）", schemaNote: "列出每個組態葉子的類型與預設值——可透過上方欄位或 Raw TOML 編輯。", messagingTitle: "訊息平台", messagingNote: "啟用/停用開關寫入 [messaging.<id>].enabled；重啟閘道後生效。憑證存於 config.toml（telegram/discord/slack 同時支援 env 鍵）。", messagingEnable: "啟用", messagingDisable: "停用", messagingTest: "測試", messagingFailed: "平台更新失敗：{error}", messagingSaveEnv: "儲存", messagingClearEnv: "清除", ttsTitle: "語音合成", ttsNote: "🔊 朗讀動作所用 provider 與音色（[tts] 設定）。音色清單需 ELEVENLABS_API_KEY；合成需對應 provider 金鑰。", ttsPreview: "試聽", ttsSample: "你好！這裡是你的 ulnclaw 閘道在說話。", ttsPreviewFailed: "試聽失敗：{error}", ttsVoicesUnavailable: "音色清單不可用（未設定金鑰？）", ttsVoicesUnauthorized: "音色清單驗證失敗——請檢查 ELEVENLABS_API_KEY",
  },
  doctor: {
    run: "執行診斷", running: "檢查中…",
    online: "包含 provider 連通性探測（較慢）",
    issues: "發現的問題", noIssues: "✓ 未發現問題。",
    failed: "診斷失敗：{error}", empty: "沒有檢查項。",
  },
  webhooks: {
    count: "{count} 個訂閱", empty: "還沒有動態 webhook 訂閱。",
    loadFailed: "載入訂閱失敗：{error}", createTitle: "新建訂閱",
    name: "名稱", namePh: "build-events", description: "描述", descriptionPh: "CI 通知",
    events: "事件", eventsPh: "push, ci（空 = 全部）", deliver: "投遞目標",
    deliverChat: "投遞會話 id（可選）",
    deliverOnly: "直接投遞（不走 agent，零 LLM 成本）",
    prompt: "提示詞 / 訊息", promptPh: "總結這個事件…",
    skills: "技能（逗號分隔）", script: "指令碼（可選）", scriptPh: "./handle.sh",
    secret: "金鑰（留空自動產生）", create: "建立",
    test: "測試", copy: "複製 URL", delete: "刪除", direct: "直接", allEvents: "（全部）",
    copied: "URL 已複製到剪貼簿。", copyFailed: "複製到剪貼簿失敗。",
    removed: "已刪除訂閱 {name}。", removeFailed: "刪除失敗：{error}",
    testing: "發送簽名測試負載…", testFailed: "測試失敗：{error}",
    createFailed: "建立失敗：{error}",
  },
  monitoring: {
    title: "閘道監控", healthExport: "健康匯出",
    metrics: "指標", diagnosticEvents: "診斷事件",
    warningLogs: "警告/錯誤日誌", otlpEndpoint: "OTLP 端點",
    otlpNotConfigured: "未設定", queueDepth: "發射佇列深度",
    installId: "安裝 ID", on: "開", off: "關",
  },
  runs: {
    count: "{count} 個執行 · {active} 活躍", empty: "還沒有追蹤的非同步執行。",
    loadFailed: "載入執行失敗：{error}", stop: "停止", stopping: "停止中…",
    result: "結果", approvalTitle: "請求核准",
    approveOnce: "一次", approveSession: "本工作階段", approveAlways: "始終", deny: "拒絕",
    approveFailed: "核准失敗：{error}", stopFailed: "停止失敗：{error}",
    delegationsTitle: "委派", noDelegations: "還沒有派發的非同步委派。",
    approvalWaitingTitle: "需要核准", approvalWaitingBody: "執行 {id} 正在等待核准：{command}", viewRuns: "開啟執行",
    timelineTitle: "即時狀態時間軸（SSE）",
    loading: "載入中…", noResult: "沒有記錄結果。",
  },
  skillsView: {
    count: "{skills} 個技能 · 已啟用工具集 {toolsets}",
    skillsTitle: "已安裝技能", toolsetsTitle: "工具集",
    noSkills: "~/.ulnclaw/skills 中還沒有安裝技能。",
    noToolsets: "沒有報告的工具集。", loadFailed: "載入失敗：{error}",
    curationTitle: "策展", archivedTitle: "已封存技能（可復原）：",
    pinSkill: "釘選", unpinSkill: "取消釘選", archiveSkill: "封存", restoreSkill: "還原",
    archiveConfirm: "封存技能 {name}？之後可以還原。",
    curationFailed: "策展操作失敗：{error}",
    enabled: "已啟用", disabled: "未啟用", tools: "工具",
  },
  browserPanel: {
    title: "瀏覽器（CDP）", configured: "已設定", backend: "後端",
    mode: "模式", source: "來源", endpoint: "端點",
    available: "可用", vnc: "VNC 位址", managedRunning: "託管瀏覽器執行中",
  },
  logsPanel: { title: "閘道日誌", allLevels: "全部層級", searchPlaceholder: "搜尋…" },
  mcpPanel: { title: "MCP 伺服器", none: "未設定 MCP 伺服器（[mcp] 段）。", oauthTokens: "oauth（已存權杖）", oauthPending: "oauth（未授權）", connect: "連線", connecting: "啟動中…", openAuth: "開啟授權頁面", approved: "已授權 ✓", failed: "OAuth 流程失敗。", toolsCached: "{count} 個快取工具" },
  kanbanPanel: { title: "看板診斷", none: "未設定看板。", openOf: "{open} 進行中 · 共 {total}", current: "目前", byStatus: "狀態計數", blocked: "受阻工作" },
  storagePanel: { title: "會話儲存", size: "資料庫大小", contents: "內容", counts: "{sessions} 個會話 · {messages} 則訊息", path: "路徑", optimize: "最佳化", optimizeTitle: "合併 FTS 段並 VACUUM 會話儲存（等同 ulnclaw sessions optimize）", optimizing: "最佳化中…", optimized: "已合併 {indexes} 個索引 · {before} → {after}", optimizeFailed: "最佳化失敗:{error}" },
  systemPanel: { title: "系統", version: "版本", platform: "平台", uptime: "執行時長", contents: "儲存", sessionsWord: "會話", messagesWord: "訊息", runsWord: "活動執行", jobs: "排程工作", enabledWord: "啟用", disabledWord: "停用", plugins: "外掛", home: "主目錄", config: "設定", desktopManaged: "桌面託管" },
  metricsPanel: { title: "Prometheus 指標", summary: "顯示 /metrics 原始輸出" },
  egressPanel: { title: "出站代理" },
  channelsPanel: { title: "訊息通道", enabled: "已啟用", disabled: "未啟用", noneEnabled: "（無）", test: "測試", stateConnected: "已連線", stateNotConfigured: "未設定" },
  learningPanel: { title: "學習圖譜", skills: "已學技能", memoryNodes: "記憶條目", edges: "圖譜邊", skillEdgesWord: "技能↔技能", memoryEdgesWord: "記憶↔技能", density: "邊密度", linked: "有關聯節點", isolated: "孤立", origin: "來源", agentCreatedWord: "Agent 建立", usedWord: "已使用", categories: "分類", topCategories: "熱門分類", hint: "在聊天工具列開啟 ✨ 學習圖譜，可瀏覽、編輯和封存節點。" },
  backupsPanel: {
    title: "狀態快照", empty: "還沒有快速快照。", newSnapshot: "新建快照",
    labelPrompt: "快照標籤（選填）：", created: "已建立快照 {id}。",
    createFailed: "建立快照失敗：{error}", restore: "還原",
    restoreConfirm: "還原快照 {id}？目前狀態檔案將被覆寫。",
    restored: "已還原快照 {id}。請重啟閘道以載入還原的狀態。",
    restoreFailed: "還原失敗：{error}", prune: "清理\u2026",
    prunePrompt: "保留多少個最新快照？", pruned: "已清理 {count} 個快照。",
    pruneFailed: "清理失敗：{error}",
  },
  checkpointsPanel: {
    title: "檢查點", size: "儲存大小",
    noProjects: "尚無有檢查點的專案（檢查點為選填功能：`[checkpoints] enabled = true`）。",
    prune: "清理\u2026", prunePrompt: "保留窗口（天）：",
    pruned: "已清理：{orphan} 個孤兒、{stale} 個過期；釋放 {bytes}。",
    pruneFailed: "清理失敗：{error}",
  },
  opsPanel: {
    title: "維運操作", securityAudit: "安全稽核", promptSize: "提示詞體積",
    dump: "除錯傾印", running: "正在執行 {action}…",
    auditClean: "未發現問題（已掃描 {total} 個元件）。",
    failed: "{action} 失敗：{error}",
  },
  updatePanel: {
    title: "更新", check: "檢查更新", apply: "套用更新",
    checking: "正在檢查更新…", applying: "正在套用更新（拉取 + 重新建置）…",
    upToDate: "已是最新版本", behind: "落後上游 {count} 個提交（目前：{version}）",
    behindShallow: "落後上游的提交數未知（淺層複製）",
    checkFailed: "更新檢查失敗：{error}",
    applyConfirm: "現在套用更新？將快進簽出並重新建置。",
    applyDone: "已更新：新增 {commits} 個提交，目前 {sha}",
    applyFailed: "更新失敗：{error}",
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
    goToProjects: "前往專案", goToJobs: "前往工作（cron）", goToUsage: "前往用量", goToConfig: "前往設定", goToDoctor: "前往診斷", goToWebhooks: "前往 Webhooks", goToRuns: "前往執行", goToSkills: "前往技能", goToSessions: "前往會話記錄", goToModels: "前往模型", goToPlugins: "前往外掛", goToPairing: "前往配對",
    newSession: "新增工作階段", switchSession: "切換工作階段",
    findInChat: "聊天內尋找", modelForSession: "本工作階段模型…",
    resumeSession: "恢復工作階段…（/resume）",
    renameSession: "重新命名工作階段…", deleteSession: "刪除工作階段…",
    exportMd: "匯出工作階段（Markdown）", exportHtml: "匯出工作階段（HTML）",
    browseArtifacts: "瀏覽工件…", learningGraph: "學習圖譜…",
    openSettings: "開啟閘道設定…", refreshSessions: "重新整理工作階段清單", restartGateway: "重啟閘道",
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
    title: "工作階段", searchPlaceholder: "搜尋工作階段與訊息…",
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
    chatTab: "チャット", kanbanTab: "カンバン", projectsTab: "プロジェクト", jobsTab: "ジョブ", usageTab: "使用量", configTab: "設定", doctorTab: "ドクター", webhooksTab: "Webhook", runsTab: "実行", skillsTab: "スキル", sessionsTab: "履歴", modelsTab: "モデル", pluginsTab: "プラグイン", pairingTab: "ペアリング",
    newSession: "新規セッション", settings: "設定", gatewayStatus: "ゲートウェイ状態",
    hatchPet: "\u{1F95A} ペットをふ化",
    selectOrStart: "セッションを選択または開始",
    inputPlaceholder: "ulnclaw にメッセージ…（Enter で送信、Shift+Enter で改行）",
    send: "送信",
    micTitle: "音声入力（録音して文字起こし）",
    micRecording: "録音中…クリックで停止",
    micFailed: "音声入力に失敗しました: {error}",
    attachTitle: "ゲートウェイのファイルシステムからファイルを添付", fsTitle: "ファイルを添付", fsUpTitle: "一つ上のディレクトリへ", fsEmpty: "空のディレクトリ", fsFailed: "ファイルブラウザーに失敗しました: {error}", fsDownloadTitle: "このファイルをダウンロード", fsMkdirTitle: "新しいフォルダー", fsMkdirPrompt: "新しいフォルダー名:",
    settingsTitle: "ゲートウェイ設定", gatewayUrl: "ゲートウェイ URL",
    apiKey: "API キー（任意、[gateway] key）", bearerToken: "bearer トークン",
    manageProcess: "ゲートウェイプロセスを管理（アプリと連動して起動/停止）",
    replayOnboarding: "オンボーディングを再生", cancel: "キャンセル", save: "保存", restartGateway: "ゲートウェイを再起動", restartDone: "ゲートウェイを再起動しました。", restartFailed: "ゲートウェイの再起動がタイムアウトしました。", restartUnavailable: "この環境はゲートウェイを管理していません — 実行場所で再起動してください。",
    settingsTheme: "テーマ", settingsFont: "フォント",
  },
  session: {
    titlePrompt: "セッションタイトル：", renamed: "セッション名を変更しました。",
    renameFailed: "名前の変更に失敗：{error}",
    deleteConfirm: "セッション「{label}」とその会話を削除しますか？",
    deleteFailed: "削除に失敗：{error}",
    exported: "{filename} をエクスポートしました", exportFailed: "エクスポートに失敗：{error}",
    newTitle: "新規セッション",
    loadFailed: "メッセージを読み込めません：{error}",
    createFailed: "セッションを作成できません：{error}",
    errorPrefix: "エラー：{error}",
    modelLockTitle: "セッションモデルのロック — クリックで変更",
    gatewayModelTitle: "ゲートウェイ既定モデル — クリックでセッションモデルを選択",
    reachable: "ゲートウェイ到達可能", unreachable: "ゲートウェイ到達不能",
    removeAttachment: "添付を削除",
    uploadFailed: "クリップボードのアップロードに失敗：{error}", speakTitle: "読み上げ（TTS）", speakFailed: "音声合成に失敗しました：{error}",
    projectBadge: "プロジェクト：{project}",
  },
  tools: { running: "実行中…", done: "完了", thinking: "思考中", arguments: "引数", result: "結果", fallbackName: "ツール" },
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
    claim: "クレーム", metaAssignee: "担当者", metaPriority: "優先度", metaCreated: "作成",
    metaStarted: "開始", metaCompleted: "完了", metaParents: "親タスク", metaChildren: "子タスク",
    attachmentsTitle: "添付",
    dispatch: "ディスパッチ", dispatchResult: "ディスパッチ完了: 起動 {spawned} · 昇格 {promoted} · 回収 {reclaimed}", dispatchFailed: "ディスパッチに失敗しました: {error}",
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
    rename: "名前を変更", renamePrompt: "新しい名前:", editAbout: "概要を編集", aboutPrompt: "説明（空でクリア）:",
    descriptionLabel: "説明（任意）", iconLabel: "アイコン絵文字（任意）",
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
    repeatLabel: "繰り返し（残り実行回数；空 = 無期限）", create: "作成", deliverLabel: "結果の配信先",
    createFailed: "ジョブの作成に失敗（ゲートウェイ到達不能またはスケジュールが無効）。",
    counts: "有効 {active} / 合計 {total} ジョブ",
    empty: "まだ cron ジョブがありません — 作成するか、ターミナルで `ulnclaw cron add` を使用してください。",
    meta: "次回：{next} · 前回：{last}", runsLeft: " · 残り {count} 回",
    deleteConfirm: "ジョブ「{name}」を削除しますか？",
    deliverBadge: "\u2192 {target}", deliverTitle: "ジョブ結果の配信先",
    deliveryError: "前回の配信に失敗（ホバーで詳細）",
    deliverPrompt: "配信先（local/origin/プラットフォーム名；空でクリア）:",
  },
  usage: {
    windowNote: "トークン集計 · 直近 {count} セッション",
    perSession: "セッション別内訳",
    empty: "まだセッションがありません。",
    totalTokens: "総トークン（ストア）", input: "入力", output: "出力",
    sessions: "セッション", messages: "メッセージ", processTokens: "ゲートウェイ トークン",
    prompt: "プロンプト", completion: "補完", toolCalls: "ツール呼び出し",
    requests: "API リクエスト", runs: "非同期実行", completed: "完了", failed: "失敗",
    colSession: "セッション", colModel: "モデル", colMessages: "件数",
    colInput: "入力", colOutput: "出力", colTotal: "合計", colStarted: "開始",
  },
  insights: {
    title: "インサイト",
    days7: "過去 7 日間", days30: "過去 30 日間", days90: "過去 90 日間", sourcePlaceholder: "ソースで絞り込み（cli、gateway…）",
    sessions: "セッション", messages: "メッセージ", toolCalls: "ツール呼び出し", tokens: "トークン",
    estCost: "推定コスト", avgSession: "平均セッション時間", activeDays: "アクティブ日数:",
    topModels: "上位モデル", topTools: "上位ツール", topSessions: "上位セッション",
    colModel: "モデル", colTool: "ツール", colSession: "セッション", calls: "呼び出し回数",
    empty: "この期間のアクティビティはありません。",
    loadFailed: "インサイトの読み込みに失敗しました: {error}",
  },
  sessionsView: {
    filterPlaceholder: "セッションを絞り込み…",
    count: "{count} 件のセッション",
    empty: "セッションはまだ記録されていません。",
    noMatch: "フィルタに一致するセッションはありません。",
    select: "セッションを選択するとトランスクリプトを閲覧できます。",
    loading: "トランスクリプトを読み込み中…",
    loadFailed: "セッションの読み込みに失敗しました: {error}",
    transcriptFailed: "トランスクリプトの読み込みに失敗しました: {error}",
    emptyTranscript: "このセッションにはメッセージがありません。",
    exportTitle: "選択したセッションを Markdown でエクスポート", exportHtmlTitle: "選択したセッションを単体 HTML でエクスポート", msgCount: "{count} メッセージ", project: "プロジェクト", source: "ソース", stats: "{sessions} セッション \u00b7 {messages} メッセージ \u00b7 ディスク {size}",
    prune: "枝切り…", archive: "アーカイブ…", pruneTitle: "フィルタに一致する終了セッションを削除", archiveTitle: "フィルタに一致する終了セッションをアーカイブ",
    pruneDialogTitle: "終了セッションを枝切り", archiveDialogTitle: "終了セッションをアーカイブ",
    olderThanLabel: "最終アクティビティがこれより古い（90d、2026-01-01…）", sourceLabel: "ソースフィルタ（任意）",
    includeArchived: "アーカイブ済みセッションを含む", preview: "プレビュー", apply: "適用",
    previewCount: "{count} 件のセッションが一致——まだ変更されていません。", previewEmpty: "一致するセッションはありません。",
    appliedPruned: "{count} 件のセッションを削除しました。", appliedArchived: "{count} 件のセッションをアーカイブしました——復元可能、何も削除されていません。",
    confirmPrune: "これら {count} 件のセッションを本当に削除しますか？元に戻せません。", confirmArchive: "これら {count} 件のセッションをアーカイブしますか？",
    failed: "失敗しました: {error}",
    exportFailed: "エクスポートに失敗しました。",
    roleUser: "ユーザー", roleAssistant: "アシスタント", roleTool: "ツール", roleSystem: "システム",
    recap: "リキャップ", recapTitle: "ゲートウェイ生成のセッション要約を表示/非表示", recapFailed: "リキャップに失敗しました: {error}",
    forkTitle: "このセッションを新しいブランチにフォーク", forked: "{id} としてフォークしました", forkFailed: "フォークに失敗しました: {error}",
    deleteTitle: "このセッションを削除", deleteConfirm: "セッション {id} を削除しますか？元に戻せません。", deleted: "{id} を削除しました。", deleteFailed: "削除に失敗しました: {error}",
    searchPlaceholder: "トランスクリプト全文検索…", noResults: "一致するトランスクリプトはありません。", searchFailed: "検索に失敗しました: {error}",
    renameTitle: "このセッションの名前を変更", renamePrompt: "新しいタイトル（空でクリア）:", renamed: "セッション名を変更しました。", renameFailed: "名前の変更に失敗しました: {error}",
  },
  modelsView: {
    count: "{providers} プロバイダ", current: "現在", catalog: "カタログ", providersLower: "プロバイダ", stale: "古い",
    none: "プロバイダは未設定です。", loadFailed: "モデル一覧の読み込みに失敗しました: {error}",
    currentBadge: "現在", authenticated: "認証済み", unauthenticated: "資格情報なし",
    docs: "ドキュメント", noModels: "モデルがリストされていません。",
    colModel: "モデル", colFamily: "ファミリー", colContext: "コンテキスト", colMaxOut: "最大出力", colCaps: "機能", colPrice: "$/Mtok",
    usageTitle: "モデル使用量（30 日）", usageEmpty: "モデル使用量はまだ記録されていません。",
    gatewayTitle: "ゲートウェイモデル", gatewayContext: "コンテキスト", gatewaySet: "ゲートウェイモデルに設定", gatewaySetConfirm: "ゲートウェイモデルを {provider}/{model} に切り替えますか？ゲートウェイ再起動後、新しいセッションに適用されます。", gatewaySetDone: "ゲートウェイモデルを更新しました——再起動後に適用されます。", gatewaySetFailed: "モデル切り替えに失敗しました: {error}", endpointsTitle: "カスタムエンドポイント", endpointsEmpty: "カスタムエンドポイントは未設定です。", endpointsTest: "テスト", endpointsActivate: "有効化", endpointsActivated: "カスタムエンドポイントを有効化しました——再起動後に適用されます。", endpointsDeleteConfirm: "エンドポイント {id} とその保存キーを削除しますか？", endpointsSaved: "エンドポイントを保存しました。", endpointsFailed: "エンドポイント操作に失敗しました: {error}",
    usageSessions: "セッション", usageMessages: "メッセージ", usageTokens: "トークン",
  },
  pluginsView: {
    count: "{count} プラグイン", none: "プラグインが見つかりません — plugin.toml マニフェスト付きのプラグインディレクトリを ~/.ulnclaw/plugins にインストールしてください。",
    loadFailed: "プラグインの読み込みに失敗しました: {error}", hooksWord: "フック", toolsWord: "ツール",
    disabledBadge: "無効", enable: "有効化", disable: "無効化",
    noConfigHooks: "[hooks] シェルフックは未設定です。", toggleFailed: "切り替えに失敗しました: {error}",
    configHooksTitle: "設定シェルフック",
    hooksRevoke: "取り消し", hooksAcceptAll: "保留中をすべて承認",
    hooksAllowlist: "同意アローリスト: {count} 件",
  },
  pairingView: {
    count: "{platforms} プラットフォーム · 保留 {pending} 件", none: "ペアリングはまだありません — 有効なボットに DM した未知の送信者にはペアリングコードが届きます。",
    loadFailed: "ペアリング要求に失敗しました: {error}", clearPending: "保留をクリア", lockedOut: "ロックアウト中",
    pendingTitle: "保留中", approvedTitle: "承認済み", age: "{minutes} 分経過", approve: "承認", revoke: "取り消し",
    emptyPlatform: "保留中・承認済みのペアリングはありません。", approvedNote: "{code} を承認しました。",
    approveFailed: "承認に失敗しました: {error}", revokedNote: "{user} を取り消しました。",
    revokeFailed: "取り消しに失敗しました: {error}", clearedNote: "{count} 件の保留コードをクリアしました。",
  },
  config: {
    loading: "設定を読み込み中…", notConnected: "ゲートウェイに未接続です。",
    loadFailed: "設定の読み込みに失敗：{error}",
    save: "保存", reload: "再読み込み", saving: "保存中…",
    saved: "{count} 件保存しました。反映にはゲートウェイの再起動が必要です。",
    saveFailed: "保存に失敗：{error}",
    addKey: "キーを追加", keyPlaceholder: "ドット区切りパス",
    valuePlaceholder: "値（JSON またはテキスト）", add: "追加",
    removeTitle: "このキーを削除",
    redactedNote: "マスクされた値は秘密です。未変更のマスク値を保存しても元の値は保持されます。",
    envKeys: "環境キー（.env）", envKeysNote: "名前のみの表示です。値は .env ファイルを編集してください。",
    restartNote: "変更は新しいプロセスに適用されます。反映にはゲートウェイを再起動してください。",
    noKeys: "config.toml は空です。", noChanges: "保存する変更がありません。",
    pending: "未保存の変更 {count} 件",
    rawButton: "生 TOML\u2026", rawTitle: "生 config.toml", rawSave: "生文を保存",
    rawConfirm: "config.toml をこのテキストで置き換えますか？反映にはゲートウェイの再起動が必要です。",
    rawSaved: "生設定を保存しました。反映にはゲートウェイを再起動してください。",
    rawFailed: "生文の保存に失敗: {error}",
    envAddLabel: "環境キーを追加", envValuePlaceholder: "値（.env に保存）",
    envEmpty: "環境キーが見つかりません。", envFile: ".env", envProcess: "プロセス環境",
    envBoth: ".env + プロセス環境", envRemoveTitle: "このキーを .env から削除",
    envRemoveConfirm: "{key} を .env から削除しますか？",
    envRevealTitle: "値を表示（30 秒に 5 回まで）",
    envSaved: "環境を更新しました。反映にはゲートウェイを再起動してください。",
    envFailed: "環境の変更に失敗しました: {error}",
    memoryTitle: "永続メモリ",
    memoryTargetAll: "すべて（MEMORY.md + USER.md）", memoryTargetMemory: "MEMORY.md のみ",
    memoryTargetUser: "USER.md のみ", memoryReset: "リセット…",
    memoryNote: "セッションを跨いで保持されるエージェントのブレット記憶；リセットはファイルを不可逆に削除します。",
    memoryMissing: "未作成", memoryEntries: "件", memoryLimit: "上限",
    memoryResetConfirm: "選択したメモリファイルを削除しますか？元に戻せません。",
    memoryResetDone: "削除済み: {files}", memoryResetNone: "削除するものがありません。",
    memoryResetFailed: "リセットに失敗しました: {error}",
    poolTitle: "認証情報プール", poolAddLabel: "プールキーを追加", poolEmpty: "プールされた認証情報はありません", poolRemoveConfirm: "このプールキーを削除しますか？", poolSaved: "認証情報プールを更新しました", poolFailed: "認証情報プールに失敗しました: {error}", poolNote: "プール内のキーはリクエストごとにラウンドロビンされ、そのプロバイダーでは環境変數より優先されます；全エントリを削除すると環境キーに戻ります。", oauthTitle: "OAuth（デバイスフロー）", oauthLoggedIn: "ログイン済み", oauthLoggedOut: "未ログイン", oauthPortal: "ポータルを開く", oauthNote: "[oauth] デバイスフローログインの読み取り専用状態（ulnclaw oauth CLI）；トークンは oauth_tokens.json に保存されます。", schemaTitle: "設定スキーマ（既定値）", schemaNote: "すべての設定リーフを型と既定値で一覧表示 — 上の行または Raw TOML から編集できます。", messagingTitle: "メッセージングプラットフォーム", messagingNote: "有効/無効の切り替えは [messaging.<id>].enabled に書き込みます；反映にはゲートウェイの再起動が必要です。認証情報は config.toml に保存（telegram/discord/slack は env キーも利用可）。", messagingEnable: "有効化", messagingDisable: "無効化", messagingTest: "テスト", messagingFailed: "プラットフォーム更新に失敗しました: {error}", messagingSaveEnv: "保存", messagingClearEnv: "消去", ttsTitle: "音声合成", ttsNote: "🔊 読み上げ動作に使うプロバイダーと声（[tts] 設定）。声一覧には ELEVENLABS_API_KEY、合成には各プロバイダーのキーが必要です。", ttsPreview: "プレビュー", ttsSample: "こんにちは！あなたの ulnclaw ゲートウェイです。", ttsPreviewFailed: "プレビューに失敗しました：{error}", ttsVoicesUnavailable: "声一覧を取得できません（キー未設定？）", ttsVoicesUnauthorized: "声一覧の認証に失敗 — ELEVENLABS_API_KEY を確認してください",
  },
  doctor: {
    run: "ドクターを実行", running: "チェック中…",
    online: "プロバイダー接続Probeを含む（低速）",
    issues: "見つかった問題", noIssues: "✓ 問題は見つかりませんでした。",
    failed: "ドクター実行に失敗：{error}", empty: "チェック項目がありません。",
  },
  webhooks: {
    count: "{count} 件の購読", empty: "動的 webhook 購読はまだありません。",
    loadFailed: "購読の読み込みに失敗：{error}", createTitle: "新規購読",
    name: "名前", namePh: "build-events", description: "説明", descriptionPh: "CI 通知",
    events: "イベント", eventsPh: "push, ci（空 = すべて）", deliver: "配信先",
    deliverChat: "配信チャット ID（任意）",
    deliverOnly: "直接配信（エージェントなし、LLM コストゼロ）",
    prompt: "プロンプト / メッセージ", promptPh: "このイベントを要約…",
    skills: "スキル（カンマ区切り）", script: "スクリプト（任意）", scriptPh: "./handle.sh",
    secret: "シークレット（空欄 = 自動生成）", create: "作成",
    test: "テスト", copy: "URL をコピー", delete: "削除", direct: "直接", allEvents: "（すべて）",
    copied: "URL をクリップボードにコピーしました。", copyFailed: "コピーに失敗しました。",
    removed: "購読 {name} を削除しました。", removeFailed: "削除に失敗：{error}",
    testing: "署名付きテストペイロードを送信中…", testFailed: "テストに失敗：{error}",
    createFailed: "作成に失敗：{error}",
  },
  monitoring: {
    title: "ゲートウェイ監視", healthExport: "ヘルスエクスポート",
    metrics: "メトリクス", diagnosticEvents: "診断イベント",
    warningLogs: "警告/エラーログ", otlpEndpoint: "OTLP エンドポイント",
    otlpNotConfigured: "未設定", queueDepth: "エミッターキュー深度",
    installId: "インストール ID", on: "オン", off: "オフ",
  },
  runs: {
    count: "{count} 件の実行 · {active} アクティブ", empty: "追跡中の非同期実行はありません。",
    loadFailed: "実行の読み込みに失敗：{error}", stop: "停止", stopping: "停止中…",
    result: "結果", approvalTitle: "承認要求",
    approveOnce: "今回", approveSession: "セッション", approveAlways: "常に", deny: "拒否",
    approveFailed: "承認に失敗：{error}", stopFailed: "停止に失敗：{error}",
    delegationsTitle: "委任", noDelegations: "ディスパッチされた非同期委任はありません。",
    approvalWaitingTitle: "承認が必要です", approvalWaitingBody: "実行 {id} が承認待ちです：{command}", viewRuns: "実行を開く",
    timelineTitle: "ライブ状態タイムライン（SSE）",
    loading: "読み込み中…", noResult: "結果は記録されていません。",
  },
  skillsView: {
    count: "{skills} 件のスキル · 有効なツールセット {toolsets}",
    skillsTitle: "インストール済みスキル", toolsetsTitle: "ツールセット",
    noSkills: "~/.ulnclaw/skills にスキルはまだありません。",
    noToolsets: "ツールセットは報告されていません。", loadFailed: "読み込みに失敗：{error}",
    curationTitle: "キュレーション", archivedTitle: "アーカイブ済みスキル（復元可能）:",
    pinSkill: "ピン留め", unpinSkill: "ピン解除", archiveSkill: "アーカイブ", restoreSkill: "復元",
    archiveConfirm: "スキル {name} をアーカイブしますか？後で復元できます。",
    curationFailed: "キュレーション操作に失敗しました: {error}",
    enabled: "有効", disabled: "無効", tools: "ツール",
  },
  browserPanel: {
    title: "ブラウザ（CDP）", configured: "設定済み", backend: "バックエンド",
    mode: "モード", source: "ソース", endpoint: "エンドポイント",
    available: "利用可能", vnc: "VNC URL", managedRunning: "管理ブラウザ実行中",
  },
  logsPanel: { title: "ゲートウェイログ", allLevels: "全レベル", searchPlaceholder: "検索…" },
  mcpPanel: { title: "MCP サーバー", none: "MCP サーバーは未設定です（[mcp] セクション）。", oauthTokens: "oauth（トークン保存済み）", oauthPending: "oauth（未承認）", connect: "接続", connecting: "開始中…", openAuth: "認証ページを開く", approved: "承認済み ✓", failed: "OAuth フローが失敗しました。", toolsCached: "キャッシュ済みツール {count} 件" },
  kanbanPanel: { title: "カンバン診断", none: "カンバンボードは未設定です。", openOf: "未完了 {open} · 全 {total}", current: "現在", byStatus: "ステータス別件数", blocked: "ブロック中タスク" },
  storagePanel: { title: "セッションストア", size: "データベースサイズ", contents: "内容", counts: "{sessions} セッション · {messages} メッセージ", path: "パス", optimize: "最適化", optimizeTitle: "FTS セグメントをマージしセッションストアを VACUUM（ulnclaw sessions optimize と同等）", optimizing: "最適化中…", optimized: "{indexes} 件のインデックスをマージ · {before} → {after}", optimizeFailed: "最適化に失敗しました: {error}" },
  systemPanel: { title: "システム", version: "バージョン", platform: "プラットフォーム", uptime: "稼働時間", contents: "ストア", sessionsWord: "セッション", messagesWord: "メッセージ", runsWord: "実行中", jobs: "cron ジョブ", enabledWord: "有効", disabledWord: "無効", plugins: "プラグイン", home: "ホーム", config: "設定", desktopManaged: "デスクトップ管理" },
  metricsPanel: { title: "Prometheus メトリクス", summary: "/metrics 生の出力を表示" },
  egressPanel: { title: "エグレスプロキシ" },
  channelsPanel: { title: "メッセージングチャネル", enabled: "有効", disabled: "無効", noneEnabled: "（なし）", test: "テスト", stateConnected: "接続済み", stateNotConfigured: "未設定" },
  learningPanel: { title: "学習グラフ", skills: "学習済みスキル", memoryNodes: "記憶チャンク", edges: "グラフエッジ", skillEdgesWord: "スキル↔スキル", memoryEdgesWord: "記憶↔スキル", density: "エッジ密度", linked: "リンク済みノード", isolated: "孤立", origin: "由来", agentCreatedWord: "エージェント作成", usedWord: "使用済み", categories: "カテゴリ", topCategories: "上位カテゴリ", hint: "チャットツールバーから ✨ 学習グラフを開き、ノードの閲覧・編集・アーカイブができます。" },
  backupsPanel: {
    title: "状態スナップショット", empty: "クイックスナップショットはまだありません。", newSnapshot: "新規スナップショット",
    labelPrompt: "スナップショットのラベル（任意）:", created: "スナップショット {id} を作成しました。",
    createFailed: "スナップショットに失敗しました: {error}", restore: "復元",
    restoreConfirm: "スナップショット {id} を復元しますか？現在の状態ファイルは上書きされます。",
    restored: "スナップショット {id} を復元しました。復元した状態の反映にはゲートウェイを再起動してください。",
    restoreFailed: "復元に失敗しました: {error}", prune: "枝切り\u2026",
    prunePrompt: "最新の何件を保持しますか？", pruned: "{count} 件のスナップショットを削除しました。",
    pruneFailed: "枝切りに失敗しました: {error}",
  },
  checkpointsPanel: {
    title: "チェックポイント", size: "ストアサイズ",
    noProjects: "チェックポイント付きプロジェクトはまだありません（オプトイン: `[checkpoints] enabled = true`）。",
    prune: "枝切り\u2026", prunePrompt: "保持期間（日数）:",
    pruned: "枝切り完了: 孤立 {orphan}、期限切れ {stale}; {bytes} 解放。",
    pruneFailed: "枝切りに失敗しました: {error}",
  },
  opsPanel: {
    title: "運用アクション", securityAudit: "セキュリティ監査", promptSize: "プロンプトサイズ",
    dump: "デバッグダンプ", running: "{action} を実行中…",
    auditClean: "所見なし（{total} 個のコンポーネントをスキャン済み）。",
    failed: "{action} に失敗しました: {error}",
  },
  updatePanel: {
    title: "アップデート", check: "更新を確認", apply: "更新を適用",
    checking: "更新を確認中…", applying: "更新を適用中（フェッチ + リビルド）…",
    upToDate: "最新です", behind: "アップストリームより {count} コミット遅れ（現在: {version}）",
    behindShallow: "遅れコミット数不明（shallow クローン）",
    checkFailed: "更新確認に失敗しました: {error}",
    applyConfirm: "今すぐ更新を適用しますか？チェックアウトを早送りしてリビルドします。",
    applyDone: "更新完了: 新規 {commits} コミット、現在 {sha}",
    applyFailed: "更新に失敗しました: {error}",
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
    goToProjects: "プロジェクトへ", goToJobs: "ジョブ（cron）へ", goToUsage: "使用量へ", goToConfig: "設定へ", goToDoctor: "ドクターへ", goToWebhooks: "Webhook へ", goToRuns: "実行へ", goToSkills: "スキルへ", goToSessions: "履歴へ", goToModels: "モデルへ", goToPlugins: "プラグインへ", goToPairing: "ペアリングへ",
    newSession: "新規セッション", switchSession: "セッションを切り替え",
    findInChat: "チャット内を検索", modelForSession: "このセッションのモデル…",
    resumeSession: "セッションを再開…（/resume）",
    renameSession: "セッション名を変更…", deleteSession: "セッションを削除…",
    exportMd: "セッションをエクスポート（Markdown）", exportHtml: "セッションをエクスポート（HTML）",
    browseArtifacts: "成果物を閲覧…", learningGraph: "学習グラフ…",
    openSettings: "ゲートウェイ設定を開く…", refreshSessions: "セッション一覧を更新", restartGateway: "ゲートウェイを再起動",
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
    title: "セッション", searchPlaceholder: "セッションとメッセージを検索…",
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
    chatTab: "الدردشة", kanbanTab: "كانبان", projectsTab: "المشاريع", jobsTab: "المهام", usageTab: "الاستخدام", configTab: "الإعدادات", doctorTab: "التشخيص", webhooksTab: "ويب هوكس", runsTab: "التشغيلات", skillsTab: "المهارات", sessionsTab: "السجلات", modelsTab: "النماذج", pluginsTab: "الإضافات", pairingTab: "الاقتران",
    newSession: "جلسة جديدة", settings: "الإعدادات", gatewayStatus: "حالة البوابة",
    hatchPet: "\u{1F95A} فقّس حيوانًا أليفًا",
    selectOrStart: "اختر جلسة أو ابدأ واحدة",
    inputPlaceholder: "راسل ulnclaw… (Enter للإرسال، Shift+Enter لسطر جديد)",
    send: "إرسال",
    micTitle: "إدخال صوتي (تسجيل وتحويل إلى نص)",
    micRecording: "جارٍ التسجيل… انقر للإيقاف",
    micFailed: "فشل الإدخال الصوتي: {error}",
    attachTitle: "إرفاق ملف من نظام ملفات البوابة", fsTitle: "إرفاق ملف", fsUpTitle: "مجلد واحد للأعلى", fsEmpty: "مجلد فارغ", fsFailed: "فشل متصفح الملفات: {error}", fsDownloadTitle: "تنزيل هذا الملف", fsMkdirTitle: "مجلد جديد", fsMkdirPrompt: "اسم المجلد الجديد:",
    settingsTitle: "إعدادات البوابة", gatewayUrl: "عنوان البوابة",
    apiKey: "مفتاح API (اختياري، [gateway] key)", bearerToken: "رمز bearer",
    manageProcess: "إدارة عملية البوابة (تشغيل/إيقاف مع التطبيق)",
    replayOnboarding: "إعادة عرض التهيئة", cancel: "إلغاء", save: "حفظ", restartGateway: "إعادة تشغيل البوابة", restartDone: "تمت إعادة تشغيل البوابة.", restartFailed: "انتهت مهلة إعادة تشغيل البوابة.", restartUnavailable: "البوابة غير مُدارة هنا — أعد تشغيلها حيث تعمل.",
    settingsTheme: "السمة", settingsFont: "الخط",
  },
  session: {
    titlePrompt: "عنوان الجلسة:", renamed: "أُعيدت تسمية الجلسة.",
    renameFailed: "فشلت إعادة التسمية: {error}",
    deleteConfirm: "حذف الجلسة «{label}» وسجل محادثتها؟",
    deleteFailed: "فشل الحذف: {error}",
    exported: "تم تصدير {filename}", exportFailed: "فشل التصدير: {error}",
    newTitle: "جلسة جديدة",
    loadFailed: "تعذّر تحميل الرسائل: {error}",
    createFailed: "تعذّر إنشاء الجلسة: {error}",
    errorPrefix: "خطأ: {error}",
    modelLockTitle: "قفل نموذج الجلسة — انقر للتغيير",
    gatewayModelTitle: "نموذج البوابة الافتراضي — انقر لاختيار نموذج الجلسة",
    reachable: "البوابة متاحة", unreachable: "البوابة غير متاحة",
    removeAttachment: "إزالة المرفق",
    uploadFailed: "فشل رفع الحافظة: {error}", speakTitle: "قراءة بصوت عالٍ (TTS)", speakFailed: "فشل تركيب الكلام: {error}",
    projectBadge: "المشروع: {project}",
  },
  tools: { running: "قيد التشغيل…", done: "تم", thinking: "يفكر", arguments: "المعاملات", result: "النتيجة", fallbackName: "أداة" },
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
    claim: "استلام", metaAssignee: "المسند إليه", metaPriority: "الأولوية", metaCreated: "أُنشئت",
    metaStarted: "بدأت", metaCompleted: "اكتملت", metaParents: "المهام الأم", metaChildren: "المهام الفرعية",
    attachmentsTitle: "المرفقات",
    dispatch: "توزيع", dispatchResult: "تم التوزيع: {spawned} انطلاق · {promoted} ترقية · {reclaimed} استرداد", dispatchFailed: "فشل التوزيع: {error}",
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
    rename: "إعادة تسمية", renamePrompt: "الاسم الجديد:", editAbout: "تحرير الوصف", aboutPrompt: "الوصف (فارغ للمسح):",
    descriptionLabel: "الوصف (اختياري)", iconLabel: "رمز تعبيري (اختياري)",
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
    repeatLabel: "التكرار (مرات التشغيل المتبقية؛ فارغ = للأبد)", create: "إنشاء", deliverLabel: "تسليم النتيجة إلى",
    createFailed: "فشل إنشاء المهمة (البوابة غير متاحة أو الجدولة غير صالحة).",
    counts: "{active} نشطة / {total} مهام",
    empty: "لا مهام cron بعد — أنشئ واحدة أو استخدم `ulnclaw cron add` في الطرفي.",
    meta: "التالية: {next} · الأخيرة: {last}", runsLeft: " · {count} تشغيلات متبقية",
    deleteConfirm: "حذف المهمة «{name}»؟",
    deliverBadge: "\u2192 {target}", deliverTitle: "وجهة تسليم نتائج المهمة",
    deliveryError: "فشل آخر تسليم (مرّر للتفاصيل)",
    deliverPrompt: "وجهة التسليم (local/origin/المنصة؛ فارغ للمسح):",
  },
  usage: {
    windowNote: "إحصاء الرموز · آخر {count} جلسة",
    perSession: "التفصيل حسب الجلسة",
    empty: "لا توجد جلسات بعد.",
    totalTokens: "إجمالي الرموز (المخزن)", input: "دخل", output: "خرج",
    sessions: "الجلسات", messages: "رسالة", processTokens: "رموز البوابة",
    prompt: "مطالبات", completion: "إكمالات", toolCalls: "استدعاءات الأدوات",
    requests: "طلبات API", runs: "تشغيلات غير متزامنة", completed: "اكتملت", failed: "فشلت",
    colSession: "الجلسة", colModel: "النموذج", colMessages: "رسائل",
    colInput: "دخل", colOutput: "خرج", colTotal: "الإجمالي", colStarted: "البدء",
  },
  insights: {
    title: "الرؤى",
    days7: "آخر 7 أيام", days30: "آخر 30 يومًا", days90: "آخر 90 يومًا", sourcePlaceholder: "تصفية حسب المصدر (cli، gateway…)",
    sessions: "الجلسات", messages: "الرسائل", toolCalls: "استدعاءات الأدوات", tokens: "الرموز",
    estCost: "التكلفة التقديرية", avgSession: "متوسط مدة الجلسة", activeDays: "أيام النشاط:",
    topModels: "أكثر النماذج استخدامًا", topTools: "أكثر الأدوات استخدامًا", topSessions: "أكثر الجلسات نشاطًا",
    colModel: "النموذج", colTool: "الأداة", colSession: "الجلسة", calls: "الاستدعاءات",
    empty: "لا يوجد نشاط مسجل في هذه الفترة.",
    loadFailed: "فشل تحميل الرؤى: {error}",
  },
  sessionsView: {
    filterPlaceholder: "تصفية الجلسات…",
    count: "{count} جلسة",
    empty: "لا توجد جلسات مسجلة بعد.",
    noMatch: "لا توجد جلسات مطابقة للتصفية.",
    select: "اختر جلسة لتصفح نصها.",
    loading: "جارٍ تحميل النص…",
    loadFailed: "فشل تحميل الجلسات: {error}",
    transcriptFailed: "فشل تحميل النص: {error}",
    emptyTranscript: "لا توجد رسائل في هذه الجلسة.",
    exportTitle: "تصدير الجلسة المحددة بصيغة Markdown", exportHtmlTitle: "تصدير الجلسة المحددة بصيغة HTML مستقلة", msgCount: "{count} رسالة", project: "المشروع", source: "المصدر", stats: "{sessions} جلسة \u00b7 {messages} رسالة \u00b7 {size} على القرص",
    prune: "تشذيب…", archive: "أرشفة…", pruneTitle: "حذف الجلسات المنتهية المطابقة للمرشحات", archiveTitle: "أرشفة الجلسات المنتهية المطابقة للمرشحات",
    pruneDialogTitle: "تشذيب الجلسات المنتهية", archiveDialogTitle: "أرشفة الجلسات المنتهية",
    olderThanLabel: "النشاط الأحدث أقدم من (90d، 2026-01-01…)", sourceLabel: "مرشح المصدر (اختياري)",
    includeArchived: "تضمين الجلسات المؤرشفة", preview: "معاينة", apply: "تطبيق",
    previewCount: "{count} جلسة مطابقة — لم يتغير شيء بعد.", previewEmpty: "لا جلسات مطابقة.",
    appliedPruned: "تم تشذيب {count} جلسة.", appliedArchived: "تمت أرشفة {count} جلسة — قابلة للاستعادة، لم يُحذف شيء.",
    confirmPrune: "هل تريد فعلاً حذف هذه {count} جلسة؟ لا يمكن التراجع.", confirmArchive: "أرشفة هذه {count} جلسة؟",
    failed: "فشل: {error}",
    exportFailed: "فشل التصدير.",
    roleUser: "المستخدم", roleAssistant: "المساعد", roleTool: "أداة", roleSystem: "النظام",
    recap: "ملخص", recapTitle: "إظهار أو إخفاء ملخص الجلسة المُنشأ من البوابة", recapFailed: "فشل الملخص: {error}",
    forkTitle: "تفريع هذه الجلسة إلى فرع جديد", forked: "تم التفريع باسم {id}", forkFailed: "فشل التفريع: {error}",
    deleteTitle: "حذف هذه الجلسة", deleteConfirm: "حذف الجلسة {id}؟ لا يمكن التراجع عن هذا.", deleted: "تم حذف {id}.", deleteFailed: "فشل الحذف: {error}",
    searchPlaceholder: "بحث كامل في النصوص…", noResults: "لا توجد نصوص مطابقة.", searchFailed: "فشل البحث: {error}",
    renameTitle: "إعادة تسمية هذه الجلسة", renamePrompt: "العنوان الجديد (فارغ للمسح):", renamed: "تمت إعادة تسمية الجلسة.", renameFailed: "فشل إعادة التسمية: {error}",
  },
  modelsView: {
    count: "{providers} مزود", current: "الحالي", catalog: "الفهرس", providersLower: "مزودًا", stale: "قديم",
    none: "لا يوجد مزود مهيأ.", loadFailed: "فشل تحميل قائمة النماذج: {error}",
    currentBadge: "الحالي", authenticated: "موثق", unauthenticated: "بدون بيانات اعتماد",
    docs: "الوثائق", noModels: "لا توجد نماذج مدرجة.",
    colModel: "النموذج", colFamily: "العائلة", colContext: "السياق", colMaxOut: "أقصى إخراج", colCaps: "القدرات", colPrice: "$/Mtok",
    usageTitle: "استخدام النماذج (30 يومًا)", usageEmpty: "لا يوجد استخدام للنماذج بعد.",
    gatewayTitle: "نموذج البوابة", gatewayContext: "السياق", gatewaySet: "تعيين كنموذج للبوابة", gatewaySetConfirm: "تبديل نموذج البوابة إلى {provider}/{model}؟ يُطبق على الجلسات الجديدة بعد إعادة تشغيل البوابة.", gatewaySetDone: "تم تحديث نموذج البوابة — أعد تشغيل البوابة للتطبيق.", gatewaySetFailed: "فشل تبديل النموذج: {error}", endpointsTitle: "نقاط نهاية مخصصة", endpointsEmpty: "لا توجد نقاط نهاية مخصصة.", endpointsTest: "اختبار", endpointsActivate: "تفعيل", endpointsActivated: "تم تفعيل نقطة النهاية — أعد تشغيل البوابة للتطبيق.", endpointsDeleteConfirm: "حذف نقطة النهاية {id} ومفتاحها المخزن؟", endpointsSaved: "تم حفظ نقطة النهاية.", endpointsFailed: "فشلت عملية نقطة النهاية: {error}",
    usageSessions: "جلسات", usageMessages: "رسائل", usageTokens: "رموز",
  },
  pluginsView: {
    count: "{count} إضافة", none: "لا توجد إضافات — ثبّت دليل إضافة ببيان plugin.toml في ~/.ulnclaw/plugins.",
    loadFailed: "فشل تحميل الإضافات: {error}", hooksWord: "خطافات", toolsWord: "أدوات",
    disabledBadge: "معطلة", enable: "تفعيل", disable: "تعطيل",
    noConfigHooks: "لا توجد خطافات [hooks] مهيأة.", toggleFailed: "فشل التبديل: {error}",
    configHooksTitle: "خطافات الصدفة المهيأة",
    hooksRevoke: "إلغاء", hooksAcceptAll: "قبول الكل المعلق",
    hooksAllowlist: "قائمة الموافقة: {count} إدخالات",
  },
  pairingView: {
    count: "{platforms} منصة · {pending} معلقة", none: "لا يوجد نشاط اقتران بعد — المرسلون غير المعروفين الذين يراسلون بوتًا مفعّلًا يتلقون رمز اقتران.",
    loadFailed: "فشل طلب الاقتران: {error}", clearPending: "مسح المعلق", lockedOut: "مقفل",
    pendingTitle: "معلق", approvedTitle: "معتمد", age: "منذ {minutes} دقيقة", approve: "اعتماد", revoke: "إلغاء",
    emptyPlatform: "لا توجد اقترانات معلقة أو معتمدة.", approvedNote: "تم اعتماد {code}.",
    approveFailed: "فشل الاعتماد: {error}", revokedNote: "تم إلغاء {user}.",
    revokeFailed: "فشل الإلغاء: {error}", clearedNote: "تم مسح {count} رمز اقتران معلق.",
  },
  config: {
    loading: "جارٍ تحميل الإعدادات…", notConnected: "البوابة غير متصلة.",
    loadFailed: "فشل تحميل الإعدادات: {error}",
    save: "حفظ", reload: "إعادة تحميل", saving: "جارٍ الحفظ…",
    saved: "تم حفظ {count} تغيير(ات). أعد تشغيل البوابة للتطبيق.",
    saveFailed: "فشل الحفظ: {error}",
    addKey: "إضافة مفتاح", keyPlaceholder: "مسار منقط",
    valuePlaceholder: "القيمة (JSON أو نص)", add: "إضافة",
    removeTitle: "إزالة هذا المفتاح",
    redactedNote: "القيم المموهة أسرار؛ حفظ قيمة مموهة دون تغيير يحافظ على الأصل.",
    envKeys: "مفاتيح البيئة (.env)", envKeysNote: "الأسماء فقط — حرّر ملف .env لتغيير القيم.",
    restartNote: "تسري التعديلات على العمليات الجديدة؛ أعد تشغيل البوابة لتطبيقها هنا.",
    noKeys: "config.toml فارغ.", noChanges: "لا شيء للحفظ.",
    pending: "{count} تغيير(ات) غير محفوظة",
    rawButton: "TOML الخام\u2026", rawTitle: "config.toml الخام", rawSave: "حفظ النص",
    rawConfirm: "استبدال config.toml بهذا النص حرفياً؟ أعد تشغيل البوابة للتطبيق.",
    rawSaved: "تم حفظ الإعداد الخام. أعد تشغيل البوابة للتطبيق.",
    rawFailed: "فشل حفظ النص: {error}",
    envAddLabel: "إضافة مفتاح بيئة", envValuePlaceholder: "القيمة (تُحفظ في .env)",
    envEmpty: "لا مفاتيح بيئة.", envFile: ".env", envProcess: "بيئة العملية",
    envBoth: ".env + بيئة العملية", envRemoveTitle: "إزالة هذا المفتاح من .env",
    envRemoveConfirm: "إزالة {key} من .env؟",
    envRevealTitle: "إظهار القيمة (5 مرات لكل 30 ثانية)",
    envSaved: "تم تحديث البيئة. أعد تشغيل البوابة للتطبيق.",
    envFailed: "فشل تغيير البيئة: {error}",
    memoryTitle: "الذاكرة الدائمة",
    memoryTargetAll: "الكل (MEMORY.md + USER.md)", memoryTargetMemory: "MEMORY.md فقط",
    memoryTargetUser: "USER.md فقط", memoryReset: "إعادة تعيين…",
    memoryNote: "ذكريات الوكيل المحتفظ بها عبر الجلسات؛ إعادة التعيين تحذف الملفات نهائيًا.",
    memoryMissing: "غير منشأ بعد", memoryEntries: "إدخالات", memoryLimit: "الحد",
    memoryResetConfirm: "حذف ملفات الذاكرة المحددة؟ لا يمكن التراجع.",
    memoryResetDone: "تم الحذف: {files}", memoryResetNone: "لا شيء لحذفه.",
    memoryResetFailed: "فشلت إعادة التعيين: {error}",
    poolTitle: "مجمع بيانات الاعتماد", poolAddLabel: "إضافة مفتاح إلى المجمع", poolEmpty: "لا توجد بيانات اعتماد في المجمع", poolRemoveConfirm: "إزالة هذا المفتاح من المجمع؟", poolSaved: "تم تحديث مجمع بيانات الاعتماد", poolFailed: "فشل مجمع بيانات الاعتماد: {error}", poolNote: "تتداول مفاتيح المجمع بالتناوب لكل طلب وتتقدم على متغيرات البيئة لموفرها؛ إزالة جميع الإدخالات تعيد الاستخدام إلى مفاتيح البيئة.", oauthTitle: "OAuth (تدفق الأجهزة)", oauthLoggedIn: "تم تسجيل الدخول", oauthLoggedOut: "غير مسجل الدخول", oauthPortal: "فتح البوابة", oauthNote: "حالة قراءة فقط لتسجيل دخول [oauth] عبر الأجهزة (ulnclaw oauth CLI)؛ الرموز مخزنة في oauth_tokens.json.", schemaTitle: "مخطط الإعدادات (القيم الافتراضية)", schemaNote: "يعرض كل ورقة إعدادات مع نوعها وقيمتها الافتراضية — عدّل عبر الحقول أعلاه أو TOML الخام.", messagingTitle: "منصات المراسلة", messagingNote: "تكتب مفاتيح التفعيل/التعطيل في [messaging.<id>].enabled؛ أعد تشغيل البوابة للتطبيق. بيانات الاعتماد في config.toml (تدعم telegram/discord/slack مفاتيح البيئة أيضًا).", messagingEnable: "تفعيل", messagingDisable: "تعطيل", messagingTest: "اختبار", messagingFailed: "فشل تحديث المنصة: {error}", messagingSaveEnv: "حفظ", messagingClearEnv: "مسح", ttsTitle: "تركيب الكلام", ttsNote: "المزود والصوت المستخدمان لإجراء القراءة بصوت عالٍ (إعدادات [tts]). قائمة الأصوات تتطلب ELEVENLABS_API_KEY؛ يتطلب التركيب مفتاح المزود.", ttsPreview: "معاينة", ttsSample: "مرحبًا! هذه بوابتك ulnclaw تتحدث.", ttsPreviewFailed: "فشل تشغيل المعاينة: {error}", ttsVoicesUnavailable: "قائمة الأصوات غير متاحة (لا يوجد مفتاح؟)", ttsVoicesUnauthorized: "فشل تفويض قائمة الأصوات — تحقق من ELEVENLABS_API_KEY",
  },
  doctor: {
    run: "تشغيل الفحص", running: "جارٍ الفحص…",
    online: "تضمين اختبارات اتصال المزودين (بطيء)",
    issues: "المشاكل المكتشفة", noIssues: "✓ لم يتم العثور على مشاكل.",
    failed: "فشل الفحص: {error}", empty: "لا توجد فحوصات.",
  },
  webhooks: {
    count: "{count} اشتراك(ات)", empty: "لا توجد اشتراكات ويب هوك ديناميكية بعد.",
    loadFailed: "فشل تحميل الاشتراكات: {error}", createTitle: "اشتراك جديد",
    name: "الاسم", namePh: "build-events", description: "الوصف", descriptionPh: "إشعارات CI",
    events: "الأحداث", eventsPh: "push, ci (فارغ = الكل)", deliver: "هدف التسليم",
    deliverChat: "معرّف دردشة التسليم (اختياري)",
    deliverOnly: "تسليم مباشر (بدون وكيل، صفر تكلفة LLM)",
    prompt: "الموجّه / الرسالة", promptPh: "لخّص هذا الحدث…",
    skills: "المهارات (مفصولة بفواصل)", script: "النص البرمجي (اختياري)", scriptPh: "./handle.sh",
    secret: "السر (فارغ = توليد تلقائي)", create: "إنشاء",
    test: "اختبار", copy: "نسخ الرابط", delete: "حذف", direct: "مباشر", allEvents: "(الكل)",
    copied: "تم نسخ الرابط إلى الحافظة.", copyFailed: "فشل النسخ إلى الحافظة.",
    removed: "تم حذف الاشتراك {name}.", removeFailed: "فشل الحذف: {error}",
    testing: "إرسال حمولة اختبار موقعة…", testFailed: "فشل الاختبار: {error}",
    createFailed: "فشل الإنشاء: {error}",
  },
  monitoring: {
    title: "مراقبة البوابة", healthExport: "تصدير الصحة",
    metrics: "المقاييس", diagnosticEvents: "الأحداث التشخيصية",
    warningLogs: "سجلات التحذير/الخطأ", otlpEndpoint: "نقطة OTLP",
    otlpNotConfigured: "غير مهيأة", queueDepth: "عمق قائمة الإرسال",
    installId: "معرّف التثبيت", on: "مفعّل", off: "معطّل",
  },
  runs: {
    count: "{count} تشغيل(ات) · {active} نشط", empty: "لا توجد تشغيلات غير متزامنة متتبعة بعد.",
    loadFailed: "فشل تحميل التشغيلات: {error}", stop: "إيقاف", stopping: "جارٍ الإيقاف…",
    result: "النتيجة", approvalTitle: "طلب موافقة",
    approveOnce: "مرة", approveSession: "الجلسة", approveAlways: "دائمًا", deny: "رفض",
    approveFailed: "فشل الاعتماد: {error}", stopFailed: "فشل الإيقاف: {error}",
    delegationsTitle: "التفويضات", noDelegations: "لا توجد تفويضات غير متزامنة مرسلة بعد.",
    approvalWaitingTitle: "مطلوب موافقة", approvalWaitingBody: "التشغيل {id} بانتظار الموافقة: {command}", viewRuns: "فتح التشغيلات",
    timelineTitle: "الجدول الزمني المباشر للحالة (SSE)",
    loading: "جارٍ التحميل…", noResult: "لم تسجل نتيجة.",
  },
  skillsView: {
    count: "{skills} مهارة · مجموعات الأدوات المفعلة {toolsets}",
    skillsTitle: "المهارات المثبتة", toolsetsTitle: "مجموعات الأدوات",
    noSkills: "لا توجد مهارات مثبتة في ~/.ulnclaw/skills بعد.",
    noToolsets: "لا توجد مجموعات أدوات مبلغ عنها.", loadFailed: "فشل التحميل: {error}",
    curationTitle: "التنسيق", archivedTitle: "مهارات مؤرشفة (قابلة للاستعادة):",
    pinSkill: "تثبيت", unpinSkill: "إلغاء التثبيت", archiveSkill: "أرشفة", restoreSkill: "استعادة",
    archiveConfirm: "أرشفة المهارة {name}؟ يمكن استعادتها لاحقاً.",
    curationFailed: "فشلت عملية التنسيق: {error}",
    enabled: "مفعلة", disabled: "معطلة", tools: "الأدوات",
  },
  browserPanel: {
    title: "المتصفح (CDP)", configured: "مهيأ", backend: "الخلفية",
    mode: "الوضع", source: "المصدر", endpoint: "نقطة النهاية",
    available: "متاح", vnc: "عنوان VNC", managedRunning: "المتصفح المُدار يعمل",
  },
  logsPanel: { title: "سجل البوابة", allLevels: "كل المستويات", searchPlaceholder: "بحث…" },
  mcpPanel: { title: "خوادم MCP", none: "لا توجد خوادم MCP مهيأة (قسم [mcp]).", oauthTokens: "oauth (الرموز محفوظة)", oauthPending: "oauth (غير مصرح)", connect: "اتصال", connecting: "جارٍ البدء…", openAuth: "فتح صفحة التفويض", approved: "تم التفويض ✓", failed: "فشل تدفق OAuth.", toolsCached: "{count} أداة مخزنة" },
  kanbanPanel: { title: "تشخيصات كانبان", none: "لا توجد لوحات كانبان مهيأة.", openOf: "{open} مفتوحة · {total} الإجمالي", current: "الحالية", byStatus: "أعداد الحالات", blocked: "المهام المحظورة" },
  storagePanel: { title: "مخزن الجلسات", size: "حجم قاعدة البيانات", contents: "المحتويات", counts: "{sessions} جلسة · {messages} رسالة", path: "المسار", optimize: "تحسين", optimizeTitle: "دمج مقاطع FTS وتفريغ مخزن الجلسات (يعادل ulnclaw sessions optimize)", optimizing: "جارٍ التحسين…", optimized: "تم دمج {indexes} فهرسًا · {before} ← {after}", optimizeFailed: "فشل التحسين: {error}" },
  systemPanel: { title: "النظام", version: "الإصدار", platform: "المنصة", uptime: "مدة التشغيل", contents: "المخزن", sessionsWord: "جلسة", messagesWord: "رسالة", runsWord: "تشغيل نشط", jobs: "مهام cron", enabledWord: "مفعلة", disabledWord: "معطلة", plugins: "الإضافات", home: "المجلد الرئيسي", config: "الإعدادات", desktopManaged: "بإدارة سطح المكتب" },
  metricsPanel: { title: "مقاييس Prometheus", summary: "عرض إخراج /metrics الخام" },
  egressPanel: { title: "وكيل الخروج" },
  channelsPanel: { title: "قنوات المراسلة", enabled: "مفعلة", disabled: "معطلة", noneEnabled: "(لا شيء)", test: "اختبار", stateConnected: "متصل", stateNotConfigured: "غير مهيأ" },
  learningPanel: { title: "رسم التعلم", skills: "مهارات مكتسبة", memoryNodes: "قطع الذاكرة", edges: "حواف الرسم", skillEdgesWord: "مهارة↔مهارة", memoryEdgesWord: "ذاكرة↔مهارة", density: "كثافة الحواف", linked: "عقد مترابطة", isolated: "معزولة", origin: "المصدر", agentCreatedWord: "أنشأها الوكيل", usedWord: "مستخدمة", categories: "الفئات", topCategories: "أهم الفئات", hint: "افتح ✨ رسم التعلم من شريط أدوات الدردشة لتصفح العقد وتحريرها وأرشفتها." },
  backupsPanel: {
    title: "لقطات الحالة", empty: "لا لقطات سريعة بعد.", newSnapshot: "لقطة جديدة",
    labelPrompt: "تسمية اللقطة (اختياري):", created: "تم إنشاء اللقطة {id}.",
    createFailed: "فشل إنشاء اللقطة: {error}", restore: "استعادة",
    restoreConfirm: "استعادة اللقطة {id}؟ ستُستبدل ملفات الحالة الحالية.",
    restored: "تمت استعادة اللقطة {id}. أعد تشغيل البوابة لتحميل الحالة المستعادة.",
    restoreFailed: "فشل الاستعادة: {error}", prune: "تشذيب\u2026",
    prunePrompt: "كم لقطة أحدث تريد الإبقاء عليها؟", pruned: "تم تشذيب {count} لقطة.",
    pruneFailed: "فشل التشذيب: {error}",
  },
  checkpointsPanel: {
    title: "نقاط التفتيش", size: "حجم المخزن",
    noProjects: "لا مشاريع ذات نقاط تفتيش بعد (الميزة اختيارية: `[checkpoints] enabled = true`).",
    prune: "تشذيب\u2026", prunePrompt: "نافذة الاحتفاظ بالأيام:",
    pruned: "تم التشذيب: {orphan} يتيمة، {stale} قديمة؛ حُرر {bytes}.",
    pruneFailed: "فشل التشذيب: {error}",
  },
  opsPanel: {
    title: "إجراءات التشغيل", securityAudit: "تدقيق الأمان", promptSize: "حجم الموجه",
    dump: "تفريغ التشخيص", running: "جارٍ تشغيل {action}…",
    auditClean: "لا نتائج (تم فحص {total} مكوّنًا).",
    failed: "فشل {action}: {error}",
  },
  updatePanel: {
    title: "التحديث", check: "التحقق من التحديثات", apply: "تطبيق التحديث",
    checking: "جارٍ التحقق من التحديثات…", applying: "جارٍ تطبيق التحديث (جلب + إعادة بناء)…",
    upToDate: "محدّث", behind: "متأخر {count} التزامًا عن المنبع (الحالي: {version})",
    behindShallow: "عدد الالتزامات المتأخرة غير معروف (استنساخ ضحل)",
    checkFailed: "فشل التحقق من التحديث: {error}",
    applyConfirm: "تطبيق التحديث الآن؟ سيتم تقديم السحب وإعادة البناء.",
    applyDone: "تم التحديث: {commits} التزامات جديدة، الآن عند {sha}",
    applyFailed: "فشل التحديث: {error}",
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
    goToProjects: "إلى المشاريع", goToJobs: "إلى المهام (cron)", goToUsage: "إلى الاستخدام", goToConfig: "إلى الإعدادات", goToDoctor: "إلى التشخيص", goToWebhooks: "إلى ويب هوكس", goToRuns: "إلى التشغيلات", goToSkills: "إلى المهارات", goToSessions: "إلى السجلات", goToModels: "إلى النماذج", goToPlugins: "إلى الإضافات", goToPairing: "إلى الاقتران",
    newSession: "جلسة جديدة", switchSession: "تبديل الجلسة",
    findInChat: "البحث في الدردشة", modelForSession: "نموذج هذه الجلسة…",
    resumeSession: "استئناف جلسة… (/resume)",
    renameSession: "إعادة تسمية الجلسة…", deleteSession: "حذف الجلسة…",
    exportMd: "تصدير الجلسة (Markdown)", exportHtml: "تصدير الجلسة (HTML)",
    browseArtifacts: "تصفح المخرجات…", learningGraph: "رسم التعلم…",
    openSettings: "فتح إعدادات البوابة…", refreshSessions: "تحديث قائمة الجلسات", restartGateway: "إعادة تشغيل البوابة",
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
    title: "الجلسات", searchPlaceholder: "البحث في الجلسات والرسائل…",
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
