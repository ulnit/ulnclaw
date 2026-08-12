import { contextBridge, ipcRenderer, webUtils } from 'electron'

contextBridge.exposeInMainWorld('ulnclawDesktop', {
  getConnection: profile => ipcRenderer.invoke('ulnclaw:connection', profile),
  revalidateConnection: () => ipcRenderer.invoke('ulnclaw:connection:revalidate'),
  touchBackend: profile => ipcRenderer.invoke('ulnclaw:backend:touch', profile),
  getGatewayWsUrl: profile => ipcRenderer.invoke('ulnclaw:gateway:ws-url', profile),
  openSessionWindow: (sessionId, opts) => ipcRenderer.invoke('ulnclaw:window:openSession', sessionId, opts),
  openWindow: () => ipcRenderer.invoke('ulnclaw:window:openInstance'),
  claimAmbientCue: key => ipcRenderer.invoke('ulnclaw:ambient:claim', key),
  wakeIndicator: {
    getState: () => ipcRenderer.invoke('ulnclaw:wake-indicator:get'),
    setState: state => ipcRenderer.send('ulnclaw:wake-indicator:set', state),
    onState: callback => {
      const listener = (_event, state) => callback(state)
      ipcRenderer.on('ulnclaw:wake-indicator:state', listener)

      return () => ipcRenderer.removeListener('ulnclaw:wake-indicator:state', listener)
    }
  },
  petOverlay: {
    // Main renderer → main process: window lifecycle + drag. `request` is
    // `{ bounds, screen }`; resolves with the screen bounds it actually used.
    open: request => ipcRenderer.invoke('ulnclaw:pet-overlay:open', request),
    close: () => ipcRenderer.invoke('ulnclaw:pet-overlay:close'),
    setBounds: bounds => ipcRenderer.send('ulnclaw:pet-overlay:set-bounds', bounds),
    setIgnoreMouse: ignore => ipcRenderer.send('ulnclaw:pet-overlay:ignore-mouse', ignore),
    // Flip the overlay focusable (and focus it) while the composer needs keys.
    setFocusable: focusable => ipcRenderer.send('ulnclaw:pet-overlay:set-focusable', focusable),
    // Main renderer → overlay (forwarded by main): push the latest pet state.
    pushState: payload => ipcRenderer.send('ulnclaw:pet-overlay:state', payload),
    // Overlay → main renderer (forwarded by main): pop back in / composer submit.
    control: payload => ipcRenderer.send('ulnclaw:pet-overlay:control', payload),
    // Overlay subscribes to state pushes.
    onState: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:pet-overlay:state', listener)

      return () => ipcRenderer.removeListener('ulnclaw:pet-overlay:state', listener)
    },
    // Main renderer subscribes to overlay control messages.
    onControl: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:pet-overlay:control', listener)

      return () => ipcRenderer.removeListener('ulnclaw:pet-overlay:control', listener)
    }
  },
  // Quick Entry: the global-hotkey mini composer window. Main owns the OS
  // shortcut + the persisted preference; the quick window only captures text
  // and hands it back, and the primary renderer submits it through the normal
  // prompt path.
  quickEntry: {
    getSettings: () => ipcRenderer.invoke('ulnclaw:quick-entry:settings:get'),
    setSettings: patch => ipcRenderer.invoke('ulnclaw:quick-entry:settings:set', patch),
    submit: payload => ipcRenderer.send('ulnclaw:quick-entry:submit', payload),
    dismiss: () => ipcRenderer.send('ulnclaw:quick-entry:dismiss'),
    // Primary renderer → main → quick window: gateway connection state + the
    // recent-session options the target picker offers. Main caches the latest
    // payload so a freshly spawned quick window starts from truth.
    pushState: payload => ipcRenderer.send('ulnclaw:quick-entry:state', payload),
    // Quick window subscribes to those pushes.
    onState: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:quick-entry:state', listener)

      return () => ipcRenderer.removeListener('ulnclaw:quick-entry:state', listener)
    },
    // Main → primary renderer: a submit captured by the quick window.
    onSubmit: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:quick-entry:submit', listener)

      return () => ipcRenderer.removeListener('ulnclaw:quick-entry:submit', listener)
    },
    // Main → quick window: you were just summoned (reset draft + refocus).
    onShown: callback => {
      const listener = () => callback()
      ipcRenderer.on('ulnclaw:quick-entry:shown', listener)

      return () => ipcRenderer.removeListener('ulnclaw:quick-entry:shown', listener)
    }
  },
  getBootProgress: () => ipcRenderer.invoke('ulnclaw:boot-progress:get'),
  getConnectionConfig: profile => ipcRenderer.invoke('ulnclaw:connection-config:get', profile),
  saveConnectionConfig: payload => ipcRenderer.invoke('ulnclaw:connection-config:save', payload),
  applyConnectionConfig: payload => ipcRenderer.invoke('ulnclaw:connection-config:apply', payload),
  testConnectionConfig: payload => ipcRenderer.invoke('ulnclaw:connection-config:test', payload),
  sshConfigHosts: () => ipcRenderer.invoke('ulnclaw:ssh-config:hosts'),
  sshResolveHost: host => ipcRenderer.invoke('ulnclaw:ssh-config:resolve', host),
  probeConnectionConfig: remoteUrl => ipcRenderer.invoke('ulnclaw:connection-config:probe', remoteUrl),
  oauthLoginConnectionConfig: remoteUrl => ipcRenderer.invoke('ulnclaw:connection-config:oauth-login', remoteUrl),
  oauthLogoutConnectionConfig: remoteUrl => ipcRenderer.invoke('ulnclaw:connection-config:oauth-logout', remoteUrl),
  // ulnclaw Cloud: one portal login powers discovery + silent per-agent sign-in
  // (cloud-auto-discovery Phase 3).
  cloud: {
    status: () => ipcRenderer.invoke('ulnclaw:cloud:status'),
    login: () => ipcRenderer.invoke('ulnclaw:cloud:login'),
    logout: () => ipcRenderer.invoke('ulnclaw:cloud:logout'),
    discover: org => ipcRenderer.invoke('ulnclaw:cloud:discover', org),
    agentSignIn: dashboardUrl => ipcRenderer.invoke('ulnclaw:cloud:agent-sign-in', dashboardUrl)
  },
  profile: {
    get: () => ipcRenderer.invoke('ulnclaw:profile:get'),
    set: name => ipcRenderer.invoke('ulnclaw:profile:set', name)
  },
  api: request => ipcRenderer.invoke('ulnclaw:api', request),
  notify: payload => ipcRenderer.invoke('ulnclaw:notify', payload),
  requestMicrophoneAccess: () => ipcRenderer.invoke('ulnclaw:requestMicrophoneAccess'),
  readFileDataUrl: filePath => ipcRenderer.invoke('ulnclaw:readFileDataUrl', filePath),
  readFileDataUrlForAttach: filePath => ipcRenderer.invoke('ulnclaw:readFileDataUrlForAttach', filePath),
  dataUrlReadMax: {
    get: () => ipcRenderer.invoke('ulnclaw:data-url-read-max:get'),
    set: maxMb => ipcRenderer.invoke('ulnclaw:data-url-read-max:set', maxMb)
  },
  readFileText: filePath => ipcRenderer.invoke('ulnclaw:readFileText', filePath),
  selectPaths: options => ipcRenderer.invoke('ulnclaw:selectPaths', options),
  writeClipboard: text => ipcRenderer.invoke('ulnclaw:writeClipboard', text),
  readClipboard: () => ipcRenderer.invoke('ulnclaw:readClipboard'),
  saveImageFromUrl: url => ipcRenderer.invoke('ulnclaw:saveImageFromUrl', url),
  saveImageBuffer: (data, ext) => ipcRenderer.invoke('ulnclaw:saveImageBuffer', { data, ext }),
  saveClipboardImage: () => ipcRenderer.invoke('ulnclaw:saveClipboardImage'),
  getPathForFile: file => {
    try {
      return webUtils.getPathForFile(file) || ''
    } catch {
      return ''
    }
  },
  normalizePreviewTarget: (target, baseDir) => ipcRenderer.invoke('ulnclaw:normalizePreviewTarget', target, baseDir),
  watchPreviewFile: url => ipcRenderer.invoke('ulnclaw:watchPreviewFile', url),
  watchDirectory: dir => ipcRenderer.invoke('ulnclaw:watchDirectory', dir),
  stopPreviewFileWatch: id => ipcRenderer.invoke('ulnclaw:stopPreviewFileWatch', id),
  setActiveWork: payload => ipcRenderer.send('ulnclaw:active-work', payload),
  setTitleBarTheme: payload => ipcRenderer.send('ulnclaw:titlebar-theme', payload),
  setNativeTheme: mode => ipcRenderer.send('ulnclaw:native-theme', mode),
  setTranslucency: payload => ipcRenderer.send('ulnclaw:translucency', payload),
  setKeepAwake: on => ipcRenderer.send('ulnclaw:keep-awake', on),
  setPreviewShortcutActive: active => ipcRenderer.send('ulnclaw:previewShortcutActive', Boolean(active)),
  openExternal: url => ipcRenderer.invoke('ulnclaw:openExternal', url),
  openPreviewInBrowser: url => ipcRenderer.invoke('ulnclaw:openPreviewInBrowser', url),
  fetchLinkTitle: url => ipcRenderer.invoke('ulnclaw:fetchLinkTitle', url),
  sanitizeWorkspaceCwd: cwd => ipcRenderer.invoke('ulnclaw:workspace:sanitize', cwd),
  settings: {
    getDefaultProjectDir: () => ipcRenderer.invoke('ulnclaw:setting:defaultProjectDir:get'),
    setDefaultProjectDir: dir => ipcRenderer.invoke('ulnclaw:setting:defaultProjectDir:set', dir),
    pickDefaultProjectDir: () => ipcRenderer.invoke('ulnclaw:setting:defaultProjectDir:pick')
  },
  zoom: {
    // Current zoom of this window, as { level, percent }.
    get: () => ipcRenderer.invoke('ulnclaw:zoom:get'),
    setPercent: percent => ipcRenderer.send('ulnclaw:zoom:set-percent', percent),
    // Fires on every zoom change, including the Ctrl/Cmd +/-/0 shortcuts,
    // so the settings UI can stay in sync with the keyboard.
    onChanged: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:zoom:changed', listener)

      return () => ipcRenderer.removeListener('ulnclaw:zoom:changed', listener)
    }
  },
  revealLogs: () => ipcRenderer.invoke('ulnclaw:logs:reveal'),
  getRecentLogs: () => ipcRenderer.invoke('ulnclaw:logs:recent'),
  readDir: dirPath => ipcRenderer.invoke('ulnclaw:fs:readDir', dirPath),
  gitRoot: startPath => ipcRenderer.invoke('ulnclaw:fs:gitRoot', startPath),
  revealPath: targetPath => ipcRenderer.invoke('ulnclaw:fs:reveal', targetPath),
  openDir: dirPath => ipcRenderer.invoke('ulnclaw:fs:openDir', dirPath),
  desktopPluginsRoot: () => ipcRenderer.invoke('ulnclaw:fs:desktopPluginsRoot'),
  renamePath: (targetPath, newName) => ipcRenderer.invoke('ulnclaw:fs:rename', targetPath, newName),
  writeTextFile: (filePath, content) => ipcRenderer.invoke('ulnclaw:fs:writeText', filePath, content),
  trashPath: targetPath => ipcRenderer.invoke('ulnclaw:fs:trash', targetPath),
  git: {
    worktreeList: repoPath => ipcRenderer.invoke('ulnclaw:git:worktreeList', repoPath),
    worktreeAdd: (repoPath, options) => ipcRenderer.invoke('ulnclaw:git:worktreeAdd', repoPath, options),
    worktreeRemove: (repoPath, worktreePath, options) =>
      ipcRenderer.invoke('ulnclaw:git:worktreeRemove', repoPath, worktreePath, options),
    branchSwitch: (repoPath, branch) => ipcRenderer.invoke('ulnclaw:git:branchSwitch', repoPath, branch),
    branchList: repoPath => ipcRenderer.invoke('ulnclaw:git:branchList', repoPath),
    baseBranchList: repoPath => ipcRenderer.invoke('ulnclaw:git:baseBranchList', repoPath),
    repoStatus: repoPath => ipcRenderer.invoke('ulnclaw:git:repoStatus', repoPath),
    fileDiff: (repoPath, filePath) => ipcRenderer.invoke('ulnclaw:git:fileDiff', repoPath, filePath),
    scanRepos: (roots, options) => ipcRenderer.invoke('ulnclaw:git:scanRepos', roots, options),
    review: {
      list: (repoPath, scope, baseRef) => ipcRenderer.invoke('ulnclaw:git:review:list', repoPath, scope, baseRef),
      diff: (repoPath, filePath, scope, baseRef, staged) =>
        ipcRenderer.invoke('ulnclaw:git:review:diff', repoPath, filePath, scope, baseRef, staged),
      stage: (repoPath, filePath) => ipcRenderer.invoke('ulnclaw:git:review:stage', repoPath, filePath),
      unstage: (repoPath, filePath) => ipcRenderer.invoke('ulnclaw:git:review:unstage', repoPath, filePath),
      revert: (repoPath, filePath) => ipcRenderer.invoke('ulnclaw:git:review:revert', repoPath, filePath),
      revParse: (repoPath, ref) => ipcRenderer.invoke('ulnclaw:git:review:revParse', repoPath, ref),
      commit: (repoPath, message, push) => ipcRenderer.invoke('ulnclaw:git:review:commit', repoPath, message, push),
      commitContext: repoPath => ipcRenderer.invoke('ulnclaw:git:review:commitContext', repoPath),
      push: repoPath => ipcRenderer.invoke('ulnclaw:git:review:push', repoPath),
      shipInfo: repoPath => ipcRenderer.invoke('ulnclaw:git:review:shipInfo', repoPath),
      createPr: repoPath => ipcRenderer.invoke('ulnclaw:git:review:createPr', repoPath)
    }
  },
  terminal: {
    cwd: id => ipcRenderer.invoke('ulnclaw:terminal:cwd', id),
    dispose: id => ipcRenderer.invoke('ulnclaw:terminal:dispose', id),
    resize: (id, size) => ipcRenderer.invoke('ulnclaw:terminal:resize', id, size),
    start: options => ipcRenderer.invoke('ulnclaw:terminal:start', options),
    write: (id, data) => ipcRenderer.invoke('ulnclaw:terminal:write', id, data),
    onData: (id, callback) => {
      const channel = `ulnclaw:terminal:${id}:data`
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on(channel, listener)

      return () => ipcRenderer.removeListener(channel, listener)
    },
    onExit: (id, callback) => {
      const channel = `ulnclaw:terminal:${id}:exit`
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on(channel, listener)

      return () => ipcRenderer.removeListener(channel, listener)
    }
  },
  onClosePreviewRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('ulnclaw:close-preview-requested', listener)

    return () => ipcRenderer.removeListener('ulnclaw:close-preview-requested', listener)
  },
  onOpenFolderRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('ulnclaw:open-folder-requested', listener)

    return () => ipcRenderer.removeListener('ulnclaw:open-folder-requested', listener)
  },
  onOpenUpdatesRequested: callback => {
    const listener = () => callback()
    ipcRenderer.on('ulnclaw:open-updates', listener)

    return () => ipcRenderer.removeListener('ulnclaw:open-updates', listener)
  },
  onDeepLink: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:deep-link', listener)

    return () => ipcRenderer.removeListener('ulnclaw:deep-link', listener)
  },
  signalDeepLinkReady: () => ipcRenderer.invoke('ulnclaw:deep-link-ready'),
  onWindowStateChanged: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:window-state-changed', listener)

    return () => ipcRenderer.removeListener('ulnclaw:window-state-changed', listener)
  },
  onFocusSession: callback => {
    const listener = (_event, sessionId) => callback(sessionId)
    ipcRenderer.on('ulnclaw:focus-session', listener)

    return () => ipcRenderer.removeListener('ulnclaw:focus-session', listener)
  },
  onNotificationAction: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:notification-action', listener)

    return () => ipcRenderer.removeListener('ulnclaw:notification-action', listener)
  },
  onPreviewFileChanged: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:preview-file-changed', listener)

    return () => ipcRenderer.removeListener('ulnclaw:preview-file-changed', listener)
  },
  onBackendExit: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:backend-exit', listener)

    return () => ipcRenderer.removeListener('ulnclaw:backend-exit', listener)
  },
  // Soft gateway-mode apply finished tearing down the primary backend. Renderer
  // should wipe session lists + re-dial without a window reload.
  onConnectionApplied: callback => {
    const listener = () => callback()
    ipcRenderer.on('ulnclaw:connection:applied', listener)

    return () => ipcRenderer.removeListener('ulnclaw:connection:applied', listener)
  },
  onPowerResume: callback => {
    const listener = () => callback()
    ipcRenderer.on('ulnclaw:power-resume', listener)

    return () => ipcRenderer.removeListener('ulnclaw:power-resume', listener)
  },
  // AC ↔ battery transitions; renderers slow their backstop polls on battery.
  getOnBattery: () => ipcRenderer.invoke('ulnclaw:power-battery:get'),
  onBatteryChanged: callback => {
    const listener = (_event, onBattery) => callback(Boolean(onBattery))
    ipcRenderer.on('ulnclaw:power-battery', listener)

    return () => ipcRenderer.removeListener('ulnclaw:power-battery', listener)
  },
  onBootProgress: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:boot-progress', listener)

    return () => ipcRenderer.removeListener('ulnclaw:boot-progress', listener)
  },
  // First-launch bootstrap progress -- emitted by the install.ps1 stage
  // runner in main.ts (apps/desktop/electron/bootstrap-runner.ts).
  // Renderer's install overlay subscribes to live events and queries the
  // current snapshot via getBootstrapState() to recover after a devtools
  // reload mid-bootstrap.
  getBootstrapState: () => ipcRenderer.invoke('ulnclaw:bootstrap:get'),
  continueBootstrapLocal: () => ipcRenderer.invoke('ulnclaw:bootstrap:continue-local'),
  resetBootstrap: () => ipcRenderer.invoke('ulnclaw:bootstrap:reset'),
  repairBootstrap: () => ipcRenderer.invoke('ulnclaw:bootstrap:repair'),
  cancelBootstrap: () => ipcRenderer.invoke('ulnclaw:bootstrap:cancel'),
  onBootstrapEvent: callback => {
    const listener = (_event, payload) => callback(payload)
    ipcRenderer.on('ulnclaw:bootstrap:event', listener)

    return () => ipcRenderer.removeListener('ulnclaw:bootstrap:event', listener)
  },
  getVersion: () => ipcRenderer.invoke('ulnclaw:version'),
  getRemoteDisplayReason: () => ipcRenderer.invoke('ulnclaw:get-remote-display-reason'),
  uninstall: {
    summary: () => ipcRenderer.invoke('ulnclaw:uninstall:summary'),
    run: mode => ipcRenderer.invoke('ulnclaw:uninstall:run', { mode })
  },
  updates: {
    check: () => ipcRenderer.invoke('ulnclaw:updates:check'),
    apply: opts => ipcRenderer.invoke('ulnclaw:updates:apply', opts),
    getBranch: () => ipcRenderer.invoke('ulnclaw:updates:branch:get'),
    setBranch: name => ipcRenderer.invoke('ulnclaw:updates:branch:set', name),
    onProgress: callback => {
      const listener = (_event, payload) => callback(payload)
      ipcRenderer.on('ulnclaw:updates:progress', listener)

      return () => ipcRenderer.removeListener('ulnclaw:updates:progress', listener)
    }
  },
  themes: {
    fetchMarketplace: id => ipcRenderer.invoke('ulnclaw:vscode-theme:fetch', id),
    searchMarketplace: query => ipcRenderer.invoke('ulnclaw:vscode-theme:search', query)
  },
  // Find-in-page (Ctrl/Cmd+F): delegates to Electron's
  // webContents.findInPage on the IPC sender's window so a Cmd+F pressed
  // in a secondary session window searches THAT window, not the primary.
  // `onFoundInPage` returns the unsubscribe fn; the renderer wires it via
  // `initFindInPageListener` in store/find-in-page.ts and tears it down
  // when the FindBar unmounts.
  findInPage: (query, options) => ipcRenderer.invoke('ulnclaw:find-in-page', query, options),
  stopFindInPage: () => ipcRenderer.invoke('ulnclaw:stop-find-in-page'),
  onFoundInPage: callback => {
    const listener = (_event, result) => callback(result)
    ipcRenderer.on('ulnclaw:found-in-page', listener)

    return () => ipcRenderer.removeListener('ulnclaw:found-in-page', listener)
  }
})
