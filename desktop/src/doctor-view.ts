// Doctor view — runs the gateway doctor checks over `GET /api/doctor`
// (the same report `ulnclaw doctor` prints) and renders ✓/⚠/✗/ℹ rows
// grouped by section, with an issues panel up top. Online provider
// probes are opt-in since they are slow.

import type { BrowserStatus, CheckpointEntry, DoctorPayload, GatewayClient, DoctorCheck, McpCatalogEntry, McpOAuthFlow, McpServerRow, MonitoringPayload, MessagingPlatform } from "./gateway";
import { t } from "./i18n";

const LEVEL_ICON: Record<DoctorCheck["level"], string> = {
  ok: "✓",
  warn: "⚠",
  fail: "✗",
  info: "ℹ",
};

const LOGS_REFRESH_MS = 10_000;
const LOGS_LINES = 150;

function escapeHtmlDoctor(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export class DoctorWidget {
  private busy = false;
  private lastReport: DoctorPayload | null = null;
  /** P527: severity filter applied to the rendered check rows. */
  private levelFilter: "all" | "warn" | "fail" = "all";
  private logsTimer: number | null = null;
  private mcpPollers: number[] = [];

  constructor(
    private root: HTMLElement,
    private client: () => GatewayClient | null,
  ) {}

  mount(): void {
    this.root.innerHTML = `
      <header id="doctor-header">
        <button id="doctor-run" class="primary" data-i18n="doctor.run">Run doctor</button>
        <label class="check doctor-online">
          <input id="doctor-online-check" type="checkbox" />
          <span data-i18n="doctor.online">Include provider connectivity probes (slow)</span>
        </label>
        <select id="doctor-level-filter" data-i18n-title="doctor.levelTitle">
          <option value="all" data-i18n="doctor.levelAll">All levels</option>
          <option value="warn" data-i18n="doctor.levelWarn">Warnings + failures</option>
          <option value="fail" data-i18n="doctor.levelFail">Failures only</option>
        </select>
        <input id="doctor-settings-search" type="search" data-i18n-ph="doctor.settingsSearchPh" data-i18n-title="doctor.settingsSearchTitle" placeholder="Filter panels…" />
        <button id="doctor-export" class="ghost" data-i18n="doctor.exportJson" hidden>JSON</button>
        <button id="doctor-gateway-restart" class="ghost" data-i18n="doctor.gatewayRestart">Restart gateway</button>
        <button id="doctor-gateway-stop" class="ghost" data-i18n="doctor.gatewayStop">Stop gateway</button>
        <span class="spacer"></span>
        <span id="doctor-status" class="jobs-counts"></span>
      </header>
      <div id="doctor-body" class="doctor-body"></div>
      <section id="doctor-monitoring" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="monitoring.title">Gateway monitoring</h3>
        <div id="monitoring-rows"></div>
      </section>
      <section id="doctor-gateway-health" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="gatewayHealth.title">Gateway health</h3>
        <div id="gateway-health-rows"></div>
      </section>
      <section id="doctor-delivery-ledger" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="deliveryLedger.title">Delivery ledger</h3>
        <div id="delivery-ledger-rows"></div>
      </section>
      <section id="doctor-dead-targets" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="deadTargets.title">Dead targets</h3>
        <div id="dead-target-rows"></div>
      </section>
      <section id="doctor-stall-watch" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="stallWatch.title">Stall watch</h3>
        <div id="stall-watch-rows"></div>
      </section>
      <section id="doctor-gateway-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="gatewaySettingsPanel.title">Gateway settings</h3>
        <div id="gateway-settings-rows"></div>
        <div id="gateway-settings-editor" class="monitoring-row" hidden>
          <select id="gateway-settings-key" class="ghost"></select>
          <input id="gateway-settings-value" class="ghost" />
          <button id="gateway-settings-apply" class="ghost mcp-add-btn" data-i18n="gatewaySettingsPanel.apply">Apply</button>
          <button id="gateway-settings-clear" class="ghost mcp-add-btn" data-i18n="gatewaySettingsPanel.clear">Clear</button>
          <span id="gateway-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-agent-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="agentSettingsPanel.title">Agent settings</h3>
        <div id="agent-settings-rows"></div>
        <div id="agent-settings-editor" class="monitoring-row" hidden>
          <select id="agent-settings-key" class="ghost"></select>
          <input id="agent-settings-value" class="ghost" />
          <button id="agent-settings-apply" class="ghost mcp-add-btn" data-i18n="agentSettingsPanel.apply">Apply</button>
          <button id="agent-settings-clear" class="ghost mcp-add-btn" data-i18n="agentSettingsPanel.clear">Clear</button>
          <span id="agent-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-web-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="webSettingsPanel.title">Web search</h3>
        <div id="web-settings-rows"></div>
        <div id="web-settings-editor" class="monitoring-row" hidden>
          <select id="web-settings-key" class="ghost"></select>
          <input id="web-settings-value" class="ghost" />
          <button id="web-settings-apply" class="ghost mcp-add-btn" data-i18n="webSettingsPanel.apply">Apply</button>
          <button id="web-settings-clear" class="ghost mcp-add-btn" data-i18n="webSettingsPanel.clear">Clear</button>
          <span id="web-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-delegation-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="delegationPanel.title">Delegation</h3>
        <div id="delegation-settings-rows"></div>
        <div id="delegation-settings-editor" class="monitoring-row" hidden>
          <select id="delegation-settings-key" class="ghost"></select>
          <input id="delegation-settings-value" class="ghost" />
          <button id="delegation-settings-apply" class="ghost mcp-add-btn" data-i18n="delegationPanel.apply">Apply</button>
          <button id="delegation-settings-clear" class="ghost mcp-add-btn" data-i18n="delegationPanel.clear">Clear</button>
          <span id="delegation-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-memory-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="memoryPanel.title">Memory limits</h3>
        <div id="memory-settings-rows"></div>
        <div id="memory-settings-editor" class="monitoring-row" hidden>
          <select id="memory-settings-key" class="ghost"></select>
          <input id="memory-settings-value" class="ghost" />
          <button id="memory-settings-apply" class="ghost mcp-add-btn" data-i18n="memoryPanel.apply">Apply</button>
          <button id="memory-settings-clear" class="ghost mcp-add-btn" data-i18n="memoryPanel.clear">Clear</button>
          <span id="memory-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-model-catalog" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="modelCatalogPanel.title">Model catalog</h3>
        <div id="model-catalog-rows"></div>
        <div id="model-catalog-editor" class="monitoring-row" hidden>
          <span class="monitoring-label" data-i18n="modelCatalogPanel.excluded">Excluded providers</span>
          <input id="model-catalog-value" class="ghost" />
          <button id="model-catalog-apply" class="ghost mcp-add-btn" data-i18n="modelCatalogPanel.apply">Apply</button>
          <button id="model-catalog-clear" class="ghost mcp-add-btn" data-i18n="modelCatalogPanel.clear">Clear</button>
          <span id="model-catalog-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-checkpoint-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="checkpointSettingsPanel.title">Checkpoint settings</h3>
        <div id="checkpoint-settings-rows"></div>
        <div id="checkpoint-settings-editor" class="monitoring-row" hidden>
          <select id="checkpoint-settings-key" class="ghost"></select>
          <input id="checkpoint-settings-value" class="ghost" />
          <button id="checkpoint-settings-apply" class="ghost mcp-add-btn" data-i18n="checkpointSettingsPanel.apply">Apply</button>
          <button id="checkpoint-settings-clear" class="ghost mcp-add-btn" data-i18n="checkpointSettingsPanel.clear">Clear</button>
          <span id="checkpoint-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-security-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="securitySettingsPanel.title">Security settings</h3>
        <div id="security-settings-rows"></div>
        <div id="security-settings-editor" class="monitoring-row" hidden>
          <select id="security-settings-key" class="ghost"></select>
          <input id="security-settings-value" class="ghost" />
          <button id="security-settings-apply" class="ghost mcp-add-btn" data-i18n="securitySettingsPanel.apply">Apply</button>
          <button id="security-settings-clear" class="ghost mcp-add-btn" data-i18n="securitySettingsPanel.clear">Clear</button>
          <span id="security-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-tool-output-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="toolOutputPanel.title">Tool output limits</h3>
        <div id="tool-output-rows"></div>
        <div id="tool-output-editor" class="monitoring-row" hidden>
          <select id="tool-output-key" class="ghost"></select>
          <input id="tool-output-value" class="ghost" />
          <button id="tool-output-apply" class="ghost mcp-add-btn" data-i18n="toolOutputPanel.apply">Apply</button>
          <button id="tool-output-clear" class="ghost mcp-add-btn" data-i18n="toolOutputPanel.clear">Clear</button>
          <span id="tool-output-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-logging-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="loggingPanel.title">Logging</h3>
        <div id="logging-settings-rows"></div>
        <div id="logging-settings-editor" class="monitoring-row" hidden>
          <select id="logging-settings-key" class="ghost"></select>
          <input id="logging-settings-value" class="ghost" />
          <button id="logging-settings-apply" class="ghost mcp-add-btn" data-i18n="loggingPanel.apply">Apply</button>
          <button id="logging-settings-clear" class="ghost mcp-add-btn" data-i18n="loggingPanel.clear">Clear</button>
          <span id="logging-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-cron-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="cronSettingsPanel.title">Cron delivery</h3>
        <div id="cron-settings-rows"></div>
        <div id="cron-settings-editor" class="monitoring-row" hidden>
          <select id="cron-settings-key" class="ghost"></select>
          <select id="cron-settings-value" class="ghost">
            <option value="true">true</option>
            <option value="false">false</option>
          </select>
          <button id="cron-settings-apply" class="ghost mcp-add-btn" data-i18n="cronSettingsPanel.apply">Apply</button>
          <button id="cron-settings-clear" class="ghost mcp-add-btn" data-i18n="cronSettingsPanel.clear">Clear</button>
          <span id="cron-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-voice-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="voicePanel.title">Voice pipeline</h3>
        <div id="voice-settings-rows"></div>
        <div id="voice-settings-editor" class="monitoring-row" hidden>
          <select id="voice-settings-key" class="ghost"></select>
          <input id="voice-settings-value" class="ghost" />
          <button id="voice-settings-apply" class="ghost mcp-add-btn" data-i18n="voicePanel.apply">Apply</button>
          <button id="voice-settings-clear" class="ghost mcp-add-btn" data-i18n="voicePanel.clear">Clear</button>
          <span id="voice-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-kanban-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="kanbanSettingsPanel.title">Kanban dispatcher</h3>
        <div id="kanban-settings-rows"></div>
        <div id="kanban-settings-editor" class="monitoring-row" hidden>
          <select id="kanban-settings-key" class="ghost"></select>
          <input id="kanban-settings-value" class="ghost" />
          <button id="kanban-settings-apply" class="ghost mcp-add-btn" data-i18n="kanbanSettingsPanel.apply">Apply</button>
          <button id="kanban-settings-clear" class="ghost mcp-add-btn" data-i18n="kanbanSettingsPanel.clear">Clear</button>
          <span id="kanban-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-x-search-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="xSearchPanel.title">X search</h3>
        <div id="x-search-rows"></div>
        <div id="x-search-editor" class="monitoring-row" hidden>
          <select id="x-search-key" class="ghost"></select>
          <input id="x-search-value" class="ghost" />
          <button id="x-search-apply" class="ghost mcp-add-btn" data-i18n="xSearchPanel.apply">Apply</button>
          <button id="x-search-clear" class="ghost mcp-add-btn" data-i18n="xSearchPanel.clear">Clear</button>
          <span id="x-search-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-video-gen-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="videoGenPanel.title">Video generation</h3>
        <div id="video-gen-rows"></div>
        <div id="video-gen-editor" class="monitoring-row" hidden>
          <select id="video-gen-key" class="ghost"></select>
          <input id="video-gen-value" class="ghost" />
          <button id="video-gen-apply" class="ghost mcp-add-btn" data-i18n="videoGenPanel.apply">Apply</button>
          <button id="video-gen-clear" class="ghost mcp-add-btn" data-i18n="videoGenPanel.clear">Clear</button>
          <span id="video-gen-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-pets-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="petsPanel.title">Pet images</h3>
        <div id="pets-rows"></div>
        <div id="pets-editor" class="monitoring-row" hidden>
          <select id="pets-key" class="ghost"></select>
          <input id="pets-value" class="ghost" />
          <button id="pets-apply" class="ghost mcp-add-btn" data-i18n="petsPanel.apply">Apply</button>
          <button id="pets-clear" class="ghost mcp-add-btn" data-i18n="petsPanel.clear">Clear</button>
          <span id="pets-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-discord-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="discordPanel.title">Discord tool</h3>
        <div id="discord-rows"></div>
        <div id="discord-editor" class="monitoring-row" hidden>
          <select id="discord-key" class="ghost"></select>
          <input id="discord-value" class="ghost" data-i18n-ph="discordPanel.placeholder" placeholder="fetch_messages, list_pins" />
          <button id="discord-apply" class="ghost mcp-add-btn" data-i18n="discordPanel.apply">Apply</button>
          <button id="discord-clear" class="ghost mcp-add-btn" data-i18n="discordPanel.clear">Clear</button>
          <span id="discord-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-moa-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="moaPanel.title">Mixture of Agents</h3>
        <div id="moa-rows"></div>
        <div id="moa-editor" class="monitoring-row" hidden>
          <select id="moa-key" class="ghost"></select>
          <input id="moa-value" class="ghost" />
          <button id="moa-apply" class="ghost mcp-add-btn" data-i18n="moaPanel.apply">Apply</button>
          <button id="moa-clear" class="ghost mcp-add-btn" data-i18n="moaPanel.clear">Clear</button>
          <span id="moa-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-phrases" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="phrasesPanel.title">Status phrases</h3>
        <div id="phrases-rows"></div>
        <div id="phrases-preview" class="monitoring-row">
          <button id="phrases-preview-status" class="ghost mcp-add-btn" data-i18n="phrasesPanel.previewStatus">Preview status</button>
          <button id="phrases-preview-generic" class="ghost mcp-add-btn" data-i18n="phrasesPanel.previewGeneric">Preview generic</button>
          <span id="phrases-preview-out" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-portal" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="portalPanel.title">Portal auth</h3>
        <div id="portal-rows"></div>
      </section>
      <section id="doctor-hooks" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="hooksPanel.title">Event hooks</h3>
        <div id="hooks-rows"></div>
      </section>
      <section id="doctor-cgroup" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="cgroupPanel.title">Cgroup reaper</h3>
        <div id="cgroup-rows"></div>
      </section>
      <section id="doctor-terminal" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="terminalPanel.title">Terminal</h3>
        <div id="terminal-rows"></div>
        <div id="terminal-editor" class="monitoring-row" hidden>
          <select id="terminal-edit-key" class="ghost"></select>
          <input id="terminal-edit-value" class="ghost" />
          <button id="terminal-edit-apply" class="ghost mcp-add-btn" data-i18n="terminalPanel.apply">Apply</button>
          <button id="terminal-edit-clear" class="ghost mcp-add-btn" data-i18n="terminalPanel.clear">Clear</button>
          <span id="terminal-edit-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-display" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="displayPanel.title">Display settings</h3>
        <div id="display-rows"></div>
        <div id="display-editor" class="monitoring-row" hidden>
          <select id="display-edit-platform" class="ghost"></select>
          <select id="display-edit-key" class="ghost"></select>
          <input id="display-edit-value" class="ghost" />
          <button id="display-edit-apply" class="ghost mcp-add-btn" data-i18n="displayPanel.apply">Apply</button>
          <button id="display-edit-clear" class="ghost mcp-add-btn" data-i18n="displayPanel.clear">Clear</button>
          <span id="display-edit-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-approvals" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="approvalsPanel.title">Approvals</h3>
        <div id="approvals-rows"></div>
        <div id="approvals-editor" class="monitoring-row" hidden>
          <select id="approvals-edit-key" class="ghost"></select>
          <input id="approvals-edit-value" class="ghost" />
          <button id="approvals-edit-apply" class="ghost mcp-add-btn" data-i18n="approvalsPanel.apply">Apply</button>
          <button id="approvals-edit-clear" class="ghost mcp-add-btn" data-i18n="approvalsPanel.clear">Clear</button>
          <span id="approvals-edit-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-lifecycle" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="lifecyclePanel.title">Lifecycle</h3>
        <div id="lifecycle-rows"></div>
      </section>
      <section id="doctor-drain" class="doctor-monitoring" hidden>
        <h3 class="config-section"><span data-i18n="drainPanel.title">Drain control</span>
          <button id="drain-begin" class="ghost mcp-add-btn" data-i18n="drainPanel.begin">Begin drain</button>
          <button id="drain-cancel" class="ghost mcp-add-btn" data-i18n="drainPanel.cancel">Cancel drain</button>
        </h3>
        <div id="drain-rows"></div>
      </section>
      <section id="doctor-monitoring-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="monitoringSettingsPanel.title">Monitoring settings</h3>
        <div id="monitoring-settings-rows"></div>
        <div id="monitoring-settings-editor" class="monitoring-row" hidden>
          <select id="monitoring-settings-key" class="ghost"></select>
          <input id="monitoring-settings-value" class="ghost" />
          <button id="monitoring-settings-apply" class="ghost mcp-add-btn" data-i18n="monitoringSettingsPanel.apply">Apply</button>
          <button id="monitoring-settings-clear" class="ghost mcp-add-btn" data-i18n="monitoringSettingsPanel.clear">Clear</button>
          <span id="monitoring-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-timezone-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="timezonePanel.title">Timezone</h3>
        <div id="timezone-rows"></div>
        <div id="timezone-editor" class="monitoring-row" hidden>
          <select id="timezone-key" class="ghost"></select>
          <input id="timezone-value" class="ghost" placeholder="Asia/Shanghai" />
          <button id="timezone-apply" class="ghost mcp-add-btn" data-i18n="timezonePanel.apply">Apply</button>
          <button id="timezone-clear" class="ghost mcp-add-btn" data-i18n="timezonePanel.clear">Clear</button>
          <span id="timezone-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-dashboard-theme" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="dashboardThemePanel.title">Dashboard theme & font</h3>
        <div id="dashboard-theme-rows"></div>
        <div id="dashboard-theme-editor" class="monitoring-row" hidden>
          <select id="dashboard-theme-key" class="ghost"></select>
          <input id="dashboard-theme-value" class="ghost" />
          <button id="dashboard-theme-apply" class="ghost mcp-add-btn" data-i18n="dashboardThemePanel.apply">Apply</button>
          <button id="dashboard-theme-clear" class="ghost mcp-add-btn" data-i18n="dashboardThemePanel.clear">Reset</button>
          <span id="dashboard-theme-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-browser-settings" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="browserSettingsPanel.title">Browser tool settings</h3>
        <div id="browser-settings-rows"></div>
        <div id="browser-settings-editor" class="monitoring-row" hidden>
          <select id="browser-settings-key" class="ghost"></select>
          <input id="browser-settings-value" class="ghost" />
          <button id="browser-settings-apply" class="ghost mcp-add-btn" data-i18n="browserSettingsPanel.apply">Apply</button>
          <button id="browser-settings-clear" class="ghost mcp-add-btn" data-i18n="browserSettingsPanel.clear">Clear</button>
          <span id="browser-settings-status" class="monitoring-value"></span>
        </div>
      </section>
      <section id="doctor-browser" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="browserPanel.title">Browser (CDP)</h3>
        <div id="browser-rows"></div>
      </section>
      <section id="doctor-mcp" class="doctor-monitoring" hidden>
        <h3 class="config-section"><span data-i18n="mcpPanel.title">MCP servers</span>
          <button id="mcp-add" class="ghost mcp-add-btn" data-i18n="mcpPanel.addServer">Add server</button>
        </h3>
        <div id="mcp-rows"></div>
        <dialog id="mcp-add-dialog">
          <h2 data-i18n="mcpPanel.dialogTitle">Add MCP server</h2>
          <label><span data-i18n="mcpPanel.nameLabel">Name</span>
            <input id="mcp-add-name" type="text" autocomplete="off" spellcheck="false" />
          </label>
          <label><span data-i18n="mcpPanel.commandLabel">Command (stdio)</span>
            <input id="mcp-add-command" type="text" autocomplete="off" spellcheck="false" placeholder="npx" />
          </label>
          <label><span data-i18n="mcpPanel.argsLabel">Arguments</span>
            <input id="mcp-add-args" type="text" autocomplete="off" spellcheck="false" placeholder="-y @modelcontextprotocol/server-filesystem /tmp" />
          </label>
          <label><span data-i18n="mcpPanel.urlLabel">URL (http/sse)</span>
            <input id="mcp-add-url" type="text" autocomplete="off" spellcheck="false" placeholder="https://…" />
          </label>
          <p id="mcp-add-status" class="config-status" hidden></p>
          <menu>
            <button id="mcp-add-cancel" data-i18n="mcpPanel.cancelBtn">Cancel</button>
            <button id="mcp-add-save" value="default" data-i18n="mcpPanel.saveBtn">Save</button>
          </menu>
        </dialog>
        <details id="mcp-catalog" class="mcp-catalog">
          <summary data-i18n="mcpPanel.catalogTitle">Catalog</summary>
          <div id="mcp-catalog-rows"></div>
        </details>
      </section>
      <section id="doctor-system" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="systemPanel.title">System</h3>
        <div id="system-rows"></div>
      </section>
      <section id="doctor-oauth" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="oauthPanel.title">Authorization</h3>
        <div id="oauth-rows"></div>
      </section>
      <section id="doctor-sync" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="syncPanel.title">Skills sync</h3>
        <div id="sync-rows"></div>
        <div class="kanban-detail-model">
          <button id="sync-fetch-remote" type="button" data-i18n="syncPanel.fetchRemote">Fetch remote</button>
        </div>
        <div id="sync-remote"></div>
      </section>
      <section id="doctor-secrets" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="secretsPanel.title">Secret sources</h3>
        <div id="secrets-rows"></div>
      </section>
      <section id="doctor-computer-use" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="computerUsePanel.title">Computer Use</h3>
        <div id="computer-use-rows"></div>
        <div class="kanban-detail-model">
          <button id="computer-use-doctor" type="button" data-i18n="computerUsePanel.runDoctor">Run doctor</button>
        </div>
        <div id="computer-use-health"></div>
      </section>
      <section id="doctor-storage" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="storagePanel.title">Session store</h3>
        <div id="storage-rows"></div>
      </section>
      <section id="doctor-backups" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="backupsPanel.title">State snapshots</h3>
        <div id="backups-rows"></div>
      </section>
      <section id="doctor-checkpoints" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="checkpointsPanel.title">Checkpoints</h3>
        <div id="checkpoints-rows"></div>
      </section>
      <section id="doctor-kanban" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="kanbanPanel.title">Kanban diagnostics</h3>
        <div id="kanban-rows"></div>
      </section>
      <section id="doctor-channels" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="channelsPanel.title">Messaging channels</h3>
        <div id="channels-rows"></div>
      </section>
      <section id="doctor-egress" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="egressPanel.title">Egress proxy</h3>
        <pre id="egress-body" class="logs-body"></pre>
      </section>
      <section id="doctor-learning" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="learningPanel.title">Learning graph</h3>
        <div id="learning-rows"></div>
      </section>
      <section id="doctor-update" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="updatePanel.title">Update</h3>
        <div class="logs-controls">
          <button id="update-check-btn" class="ghost" data-i18n="updatePanel.check">Check for updates</button>
          <button id="update-apply-btn" class="ghost danger" data-i18n="updatePanel.apply" hidden>Apply update</button>
          <span id="update-status" class="config-note"></span>
        </div>
        <pre id="update-body" class="logs-body" hidden></pre>
      </section>
      <section id="doctor-ops" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="opsPanel.title">Ops actions</h3>
        <div class="logs-controls">
          <button id="ops-audit-btn" class="ghost" data-i18n="opsPanel.securityAudit">Security audit</button>
          <button id="ops-prompt-size-btn" class="ghost" data-i18n="opsPanel.promptSize">Prompt size</button>
          <button id="ops-dump-btn" class="ghost" data-i18n="opsPanel.dump">Debug dump</button>
          <span id="ops-status" class="config-note"></span>
        </div>
        <pre id="ops-body" class="logs-body" hidden></pre>
      </section>
      <section id="doctor-metrics" class="doctor-monitoring" hidden>
        <h3 class="config-section" data-i18n="metricsPanel.title">Prometheus metrics</h3>
        <details id="metrics-details">
          <summary data-i18n="metricsPanel.summary">Show raw /metrics exposition</summary>
          <pre id="metrics-body" class="logs-body"></pre>
        </details>
      </section>
      <section id="doctor-logs" class="doctor-monitoring doctor-logs" hidden>
        <h3 class="config-section" data-i18n="logsPanel.title">Gateway log</h3>
        <div class="logs-controls">
          <select id="logs-file">
            <option value="gateway">gateway.log</option>
            <option value="agent">agent.log</option>
            <option value="errors">errors.log</option>
          </select>
          <input id="logs-search" type="text" data-i18n-ph="logsPanel.searchPlaceholder" placeholder="search\u2026" />
          <select id="logs-level">
            <option value="" data-i18n="logsPanel.allLevels">All levels</option>
            <option value="INFO">INFO+</option>
            <option value="WARN">WARN+</option>
            <option value="ERROR">ERROR+</option>
          </select>
          <span id="logs-path" class="config-note"></span>
          <span class="spacer"></span>
          <button id="logs-refresh" class="ghost" title="Refresh" data-i18n-title="kanban.refresh">↻</button>
        </div>
        <pre id="logs-body" class="logs-body"></pre>
      </section>
    `;
    this.root.querySelector("#doctor-gateway-restart")!.addEventListener("click", () => {
      void this.restartGateway();
    });
    this.root.querySelector("#doctor-gateway-stop")!.addEventListener("click", () => {
      void this.stopGateway();
    });

    this.root.querySelector("#doctor-export")!.addEventListener("click", () => {
      this.exportJson();
    });
    this.root.querySelector("#doctor-run")!.addEventListener("click", () => {
      this.run().catch(() => undefined);
    });
    // P527: re-render the cached report through the severity filter.
    this.root.querySelector("#doctor-level-filter")!.addEventListener("change", () => {
      const select = this.root.querySelector("#doctor-level-filter") as HTMLSelectElement;
      this.levelFilter =
        select.value === "fail" ? "fail" : select.value === "warn" ? "warn" : "all";
      if (this.lastReport) {
        this.render(this.lastReport.report.sections, this.lastReport.report.issues);
      }
    });
    // P766: filter Doctor panels by text content.
    this.root.querySelector("#doctor-settings-search")!.addEventListener("input", () => {
      const input = this.root.querySelector("#doctor-settings-search") as HTMLInputElement;
      this.applySettingsSearch(input.value);
    });
    this.root.querySelector("#ops-audit-btn")!.addEventListener("click", () => {
      this.runOps("securityAudit").catch(() => undefined);
    });
    this.root.querySelector("#ops-prompt-size-btn")!.addEventListener("click", () => {
      this.runOps("promptSize").catch(() => undefined);
    });
    this.root.querySelector("#ops-dump-btn")!.addEventListener("click", () => {
      this.runOps("dump").catch(() => undefined);
    });
    this.root.querySelector("#update-check-btn")!.addEventListener("click", () => {
      this.checkUpdate().catch(() => undefined);
    });
    this.root.querySelector("#update-apply-btn")!.addEventListener("click", () => {
      this.applyUpdate().catch(() => undefined);
    });
  }

  start(): void {
    if (!this.root.querySelector(".doctor-section")) {
      this.run().catch(() => undefined);
    }
    this.loadMonitoring().catch(() => undefined);
    this.loadGatewayHealth().catch(() => undefined);
    this.loadGatewaySettings().catch(() => undefined);
    this.loadAgentSettings().catch(() => undefined);
    this.loadWebSettings().catch(() => undefined);
    this.loadDelegationSettings().catch(() => undefined);
    this.loadMemorySettings().catch(() => undefined);
    this.loadModelCatalog().catch(() => undefined);
    this.loadCheckpointSettings().catch(() => undefined);
    this.loadSecuritySettings().catch(() => undefined);
    this.loadToolOutputSettings().catch(() => undefined);
    this.loadLoggingSettings().catch(() => undefined);
    this.loadCronSettings().catch(() => undefined);
    this.loadVoiceSettings().catch(() => undefined);
    this.loadKanbanSettings().catch(() => undefined);
    this.loadXSearchSettings().catch(() => undefined);
    this.loadVideoGenSettings().catch(() => undefined);
    this.loadMoaSettings().catch(() => undefined);
    this.loadDiscordSettings().catch(() => undefined);
    this.loadPetsSettings().catch(() => undefined);
    this.loadBrowserSettings().catch(() => undefined);
    this.loadDashboardTheme().catch(() => undefined);
    this.loadTimezoneSettings().catch(() => undefined);
    this.loadMonitoringSettings().catch(() => undefined);
    this.loadDeliveryLedger().catch(() => undefined);
    this.loadDeadTargets().catch(() => undefined);
    this.loadStallWatch().catch(() => undefined);
    this.loadDrain().catch(() => undefined);
    this.loadLifecycle().catch(() => undefined);
    this.loadDisplay().catch(() => undefined);
    this.loadPhrases().catch(() => undefined);
    this.loadTerminal().catch(() => undefined);
    this.loadApprovals().catch(() => undefined);
    this.loadCgroup().catch(() => undefined);
    this.loadHooks().catch(() => undefined);
    this.loadPortal().catch(() => undefined);
    this.loadBrowser().catch(() => undefined);
    this.loadMcp().catch(() => undefined);
    this.wireMcpAdd();
    this.loadSystem().catch(() => undefined);
    this.loadComputerUse().catch(() => undefined);
    this.loadSecrets().catch(() => undefined);
    this.loadSync().catch(() => undefined);
    this.loadOAuth().catch(() => undefined);
    this.loadStorage().catch(() => undefined);
    this.loadBackups().catch(() => undefined);
    this.loadCheckpoints().catch(() => undefined);
    this.loadKanban().catch(() => undefined);
    this.loadMetrics().catch(() => undefined);
    this.loadEgress().catch(() => undefined);
    this.loadChannels().catch(() => undefined);
    this.loadLearning().catch(() => undefined);
    this.loadOps().catch(() => undefined);
    this.loadUpdate().catch(() => undefined);
    this.loadLogs().catch(() => undefined);
    if (this.logsTimer === null) {
      this.logsTimer = window.setInterval(() => {
        this.loadLogs().catch(() => undefined);
      }, LOGS_REFRESH_MS);
    }
  }

  stop(): void {
    if (this.logsTimer !== null) {
      window.clearInterval(this.logsTimer);
      this.logsTimer = null;
    }
    for (const poller of this.mcpPollers) window.clearInterval(poller);
    this.mcpPollers = [];
  }

  /** P766: hide Doctor panels whose text does not match the query;
   * an empty query restores every panel's loader-controlled state. */
  private applySettingsSearch(query: string): void {
    const needle = query.trim().toLowerCase();
    const sections = this.root.querySelectorAll<HTMLElement>("section.doctor-monitoring");
    sections.forEach((section) => {
      if (!needle) {
        section.classList.remove("doctor-search-hide");
        return;
      }
      const text = (section.textContent ?? "").toLowerCase();
      section.classList.toggle("doctor-search-hide", !text.includes(needle));
    });
  }

  /** P765: transient save confirmation for settings editors. */
  private flashSaved(statusEl: HTMLElement): void {
    const saved = t.monitoring.saved;
    statusEl.textContent = saved;
    window.setTimeout(() => {
      if (statusEl.textContent === saved) statusEl.textContent = "";
    }, 2000);
  }

  private status(message: string): void {
    (this.root.querySelector("#doctor-status") as HTMLElement).textContent = message;
  }

  /** P366: download the last doctor report as JSON for support filing. */
  private exportJson(): void {
    if (!this.lastReport) return;
    const blob = new Blob([JSON.stringify(this.lastReport, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `ulnclaw-doctor-${Date.now()}.json`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 2000);
  }

  private async run(): Promise<void> {
    const client = this.client();
    if (!client || this.busy) return;
    this.busy = true;
    const runBtn = this.root.querySelector("#doctor-run") as HTMLButtonElement;
    runBtn.disabled = true;
    this.status(t.doctor.running);
    const online = (this.root.querySelector("#doctor-online-check") as HTMLInputElement).checked;
    const body = this.root.querySelector("#doctor-body") as HTMLElement;
    try {
      const payload = await client.doctor(online);
      this.lastReport = payload;
      this.render(payload.report.sections, payload.report.issues);
      this.status("");
      const exportBtn = this.root.querySelector("#doctor-export") as HTMLButtonElement;
      exportBtn.hidden = false;
    } catch (error) {
      body.innerHTML = "";
      this.status(
        t.doctor.failed.replace("{error}", error instanceof Error ? error.message : String(error)),
      );
    } finally {
      this.busy = false;
      runBtn.disabled = false;
    }
  }

  private async loadMonitoring(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-monitoring") as HTMLElement;
    const rows = this.root.querySelector("#monitoring-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.monitoring();
      rows.innerHTML = "";
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const entries: [string, string][] = [
        [t.monitoring.healthExport, status.enabled ? on : off],
        [
          t.monitoring.metrics,
          status.metrics
            ? `${on} (${status.metrics_interval_seconds}s)`
            : off,
        ],
        [t.monitoring.diagnosticEvents, status.diagnostic_events ? on : off],
        [
          t.monitoring.warningLogs,
          status.warning_error_logs
            ? `${on} (${status.logs_interval_seconds}s)`
            : off,
        ],
        [
          t.monitoring.otlpEndpoint,
          status.otlp.endpoint
            ? `${status.otlp.endpoint} (${status.otlp.transport})`
            : t.monitoring.otlpNotConfigured,
        ],
        [t.monitoring.queueDepth, String(status.queue_depth)],
      ];
      if (status.install_id) {
        entries.push([t.monitoring.installId, status.install_id]);
      }
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      const note = document.createElement("p");
      note.className = "config-note";
      note.textContent = status.scope;
      rows.appendChild(note);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Add-server dialog wiring for the MCP panel (P603). */
  private wireMcpAdd(): void {
    const addBtn = this.root.querySelector("#mcp-add") as HTMLButtonElement | null;
    const dialog = this.root.querySelector("#mcp-add-dialog") as HTMLDialogElement | null;
    if (!addBtn || !dialog) return;
    const input = (id: string): HTMLInputElement =>
      this.root.querySelector(`#${id}`) as HTMLInputElement;
    const statusEl = this.root.querySelector("#mcp-add-status") as HTMLElement;
    addBtn.addEventListener("click", () => {
      input("mcp-add-name").value = "";
      input("mcp-add-command").value = "";
      input("mcp-add-args").value = "";
      input("mcp-add-url").value = "";
      statusEl.hidden = true;
      dialog.showModal();
      input("mcp-add-name").focus();
    });
    this.root.querySelector("#mcp-add-cancel")!.addEventListener("click", () => dialog.close());
    this.root.querySelector("#mcp-add-save")!.addEventListener("click", () => {
      const client = this.client();
      if (!client) return;
      const name = input("mcp-add-name").value.trim();
      const command = input("mcp-add-command").value.trim();
      const args = input("mcp-add-args")
        .value.trim()
        .split(/\s+/)
        .filter((entry) => entry.length > 0);
      const url = input("mcp-add-url").value.trim();
      const body: Parameters<GatewayClient["mcpServerAdd"]>[0] = { name };
      if (command) {
        body.command = command;
        body.args = args;
      }
      if (url) body.url = url;
      statusEl.hidden = true;
      void client
        .mcpServerAdd(body)
        .then((result) => {
          dialog.close();
          void this.loadMcp();
          void Promise.resolve(result);
        })
        .catch((error: unknown) => {
          statusEl.hidden = false;
          statusEl.classList.add("error");
          statusEl.textContent = t.mcpPanel.actionFailed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          );
        });
    });
  }

  private async loadMcp(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-mcp") as HTMLElement;
    const rows = this.root.querySelector("#mcp-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const servers = await client.mcpServers();
      rows.innerHTML = "";
      if (servers.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.mcpPanel.none;
        rows.appendChild(empty);
      }
      for (const server of servers) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = server.name;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        let auth: string = server.auth;
        if (server.auth === "oauth") {
          auth = server.oauth_tokens ? t.mcpPanel.oauthTokens : t.mcpPanel.oauthPending;
        }
        value.textContent = `${server.kind} · ${server.target} · ${auth}`;
        value.title = server.target;
        row.append(label, value);
        const tools = server.cached_tools || [];
        if (tools.length > 0) {
          const details = document.createElement("details");
          details.className = "mcp-tools-details";
          details.innerHTML = `<summary>${
            t.mcpPanel.toolsCached.replace("{count}", String(tools.length))
          }</summary>`;
          const list = document.createElement("div");
          list.className = "mcp-tools-list";
          for (const tool of tools) {
            const item = document.createElement("div");
            item.className = "mcp-tool";
            const name = document.createElement("code");
            name.textContent = tool.name;
            item.appendChild(name);
            if (tool.description) {
              const desc = document.createElement("span");
              desc.className = "mcp-tool-desc";
              desc.textContent = tool.description;
              item.appendChild(desc);
            }
            list.appendChild(item);
          }
          details.appendChild(list);
          row.appendChild(details);
        }
        if (server.auth === "oauth" && !server.oauth_tokens) {
          const connect = document.createElement("button");
          connect.className = "ghost mcp-connect";
          connect.textContent = t.mcpPanel.connect;
          connect.addEventListener("click", () => {
            this.startMcpOAuth(server, row, connect).catch(() => undefined);
          });
          row.appendChild(connect);
        }
        if (server.enabled === false) {
          const badge = document.createElement("span");
          badge.className = "mcp-disabled-badge";
          badge.textContent = t.mcpPanel.disabledBadge;
          row.appendChild(badge);
        }
        const actions = document.createElement("span");
        actions.className = "mcp-row-actions";
        const testBtn = document.createElement("button");
        testBtn.className = "ghost";
        testBtn.textContent = t.mcpPanel.testBtn;
        testBtn.addEventListener("click", () => {
          testBtn.disabled = true;
          testBtn.textContent = t.mcpPanel.testing;
          void client
            .mcpServerTest(server.name)
            .then((result) => {
              this.mcpFlowNote(row, t.mcpPanel.testOk.replace("{count}", String(result.count)), false);
            })
            .catch((error: unknown) => {
              this.mcpFlowNote(
                row,
                t.mcpPanel.testFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
                true,
              );
            })
            .finally(() => {
              testBtn.disabled = false;
              testBtn.textContent = t.mcpPanel.testBtn;
            });
        });
        const toggleBtn = document.createElement("button");
        toggleBtn.className = "ghost";
        toggleBtn.textContent = server.enabled === false ? t.mcpPanel.enableBtn : t.mcpPanel.disableBtn;
        toggleBtn.addEventListener("click", () => {
          void client
            .mcpServerSetEnabled(server.name, server.enabled === false)
            .then(() => this.loadMcp())
            .catch((error: unknown) => {
              this.mcpFlowNote(
                row,
                t.mcpPanel.actionFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
                true,
              );
            });
        });
        const deleteBtn = document.createElement("button");
        deleteBtn.className = "ghost";
        deleteBtn.textContent = "\u{1F5D1}";
        deleteBtn.title = t.mcpPanel.deleteConfirm.replace("{name}", server.name);
        deleteBtn.addEventListener("click", () => {
          if (!window.confirm(t.mcpPanel.deleteConfirm.replace("{name}", server.name))) return;
          void client
            .mcpServerDelete(server.name)
            .then(() => this.loadMcp())
            .catch((error: unknown) => {
              this.mcpFlowNote(
                row,
                t.mcpPanel.actionFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
                true,
              );
            });
        });
        actions.append(testBtn, toggleBtn, deleteBtn);
        row.appendChild(actions);
        rows.appendChild(row);
      }
      section.hidden = false;
      void this.loadMcpCatalog();
    } catch {
      section.hidden = true;
    }
  }

  /** Render the curated MCP catalog with install buttons (hermes
   * `optional-mcps` parity; P604). */
  private async loadMcpCatalog(): Promise<void> {
    const client = this.client();
    const rows = this.root.querySelector("#mcp-catalog-rows") as HTMLElement | null;
    if (!client || !rows) return;
    try {
      const entries = await client.mcpCatalog();
      rows.innerHTML = "";
      for (const entry of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row mcp-catalog-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = entry.name;
        const value = document.createElement("span");
        value.className = "monitoring-value mcp-catalog-desc";
        value.textContent = entry.description;
        value.title = `${entry.command} ${entry.args.join(" ")}`;
        row.append(label, value);
        if (entry.installed) {
          const badge = document.createElement("span");
          badge.className = entry.enabled ? "mcp-catalog-badge" : "mcp-catalog-badge off";
          badge.textContent = entry.enabled ? t.mcpPanel.catalogInstalled : t.mcpPanel.catalogInstalledDisabled;
          row.appendChild(badge);
        } else {
          const installBtn = document.createElement("button");
          installBtn.className = "ghost mcp-install-btn";
          installBtn.textContent = t.mcpPanel.catalogInstallBtn;
          installBtn.addEventListener("click", () => {
            void this.installCatalogEntry(entry, row, installBtn);
          });
          row.appendChild(installBtn);
        }
        rows.appendChild(row);
      }
    } catch {
      rows.innerHTML = "";
    }
  }

  /** Prompt for required env vars, install the entry, refresh (P604). */
  private async installCatalogEntry(
    entry: McpCatalogEntry,
    row: HTMLElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const client = this.client();
    if (!client) return;
    const env: Record<string, string> = {};
    for (const variable of entry.required_env) {
      const answer = window.prompt(variable.prompt, "");
      if (answer === null || answer.trim() === "") {
        this.mcpFlowNote(row, t.mcpPanel.catalogEnvMissing.replace("{name}", variable.name), true);
        return;
      }
      env[variable.name] = answer.trim();
    }
    button.disabled = true;
    try {
      await client.mcpCatalogInstall(entry.name, env);
      this.mcpFlowNote(row, t.mcpPanel.catalogInstalledNote, false);
      await this.loadMcp();
    } catch (error) {
      this.mcpFlowNote(
        row,
        t.mcpPanel.actionFailed.replace("{error}", error instanceof Error ? error.message : String(error)),
        true,
      );
      button.disabled = false;
    }
  }

  /** Restart the gateway over POST /api/gateway/restart (P609). */
  private async restartGateway(): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.doctor.restartConfirm)) return;
    const statusEl = this.root.querySelector("#doctor-status") as HTMLElement;
    try {
      await client.gatewayRestart();
      statusEl.textContent = t.doctor.restarting;
    } catch (error) {
      statusEl.textContent = t.doctor.lifecycleFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** Stop the gateway over POST /api/gateway/stop (P609). */
  private async stopGateway(): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.doctor.stopConfirm)) return;
    const statusEl = this.root.querySelector("#doctor-status") as HTMLElement;
    try {
      await client.gatewayStop();
      statusEl.textContent = t.doctor.stopping;
    } catch (error) {
      statusEl.textContent = t.doctor.lifecycleFailed.replace(
        "{error}",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** Initiate an MCP OAuth flow, surface the authorization URL, and
   * poll the flow until approved/error (hermes dashboard parity). */
  private async startMcpOAuth(
    server: McpServerRow,
    row: HTMLElement,
    button: HTMLButtonElement,
  ): Promise<void> {
    const client = this.client();
    if (!client) return;
    button.disabled = true;
    button.textContent = t.mcpPanel.connecting;
    let flow: McpOAuthFlow;
    try {
      flow = await client.mcpAuth(server.name);
    } catch (error) {
      button.disabled = false;
      button.textContent = t.mcpPanel.connect;
      this.mcpFlowNote(row, error instanceof Error ? error.message : String(error), true);
      return;
    }
    const flowId = flow.flow_id;
    const poll = window.setInterval(async () => {
      const current = this.client();
      if (!current) return;
      try {
        flow = await current.mcpFlowStatus(flowId);
      } catch (error) {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        button.disabled = false;
        button.textContent = t.mcpPanel.connect;
        this.mcpFlowNote(row, error instanceof Error ? error.message : String(error), true);
        return;
      }
      if (flow.authorization_url && !row.querySelector(".mcp-auth-link")) {
        this.mcpAuthLink(row, flow.authorization_url);
      }
      if (flow.status === "approved") {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        this.mcpFlowNote(row, t.mcpPanel.approved, false);
        window.setTimeout(() => this.loadMcp().catch(() => undefined), 500);
      } else if (flow.status === "error") {
        window.clearInterval(poll);
        this.mcpPollers = this.mcpPollers.filter((id) => id !== poll);
        button.disabled = false;
        button.textContent = t.mcpPanel.connect;
        this.mcpFlowNote(row, flow.error || t.mcpPanel.failed, true);
      }
    }, 2_000);
    this.mcpPollers.push(poll);
  }

  private mcpAuthLink(row: HTMLElement, url: string): void {
    const link = document.createElement("a");
    link.className = "mcp-auth-link";
    link.href = url;
    link.target = "_blank";
    link.rel = "noreferrer";
    link.textContent = t.mcpPanel.openAuth;
    row.appendChild(link);
  }

  private mcpFlowNote(row: HTMLElement, message: string, isError: boolean): void {
    let note = row.querySelector(".mcp-flow-note") as HTMLElement | null;
    if (!note) {
      note = document.createElement("span");
      note.className = "mcp-flow-note config-note";
      row.appendChild(note);
    }
    note.textContent = message;
    note.classList.toggle("error", isError);
  }

  private fmtUptime(seconds: number): string {
    const days = Math.floor(seconds / 86_400);
    const hours = Math.floor((seconds % 86_400) / 3_600);
    const minutes = Math.floor((seconds % 3_600) / 60);
    if (days > 0) return `${days}d ${hours}h`;
    if (hours > 0) return `${hours}h ${minutes}m`;
    return `${minutes}m`;
  }

  /** Gateway/system facts: version, platform, paths, uptime, counts. */
  private async loadSystem(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-system") as HTMLElement;
    const rows = this.root.querySelector("#system-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.systemInfo();
      const v = t.systemPanel;
      const entries: [string, string][] = [
        [v.version, `${info.service} ${info.version}`],
        [v.platform, `${info.os}/${info.arch} · pid ${info.pid}${info.desktop_managed ? ` · ${v.desktopManaged}` : ""}`],
        [v.uptime, this.fmtUptime(info.uptime_secs)],
        [v.contents, `${info.sessions} ${v.sessionsWord} · ${info.messages} ${v.messagesWord} · ${info.active_runs} ${v.runsWord}`],
        [v.jobs, `${info.cron_jobs_enabled} ${v.enabledWord} · ${info.cron_jobs_disabled} ${v.disabledWord}`],
        [v.plugins, String(info.plugins_loaded)],
        [v.home, info.home],
        [v.config, info.config_path],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Authorization (P644): sync device-flow posture + Google Chat
   * OAuth accounts. Tokens are only ever shown redacted server-side. */
  private async loadOAuth(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-oauth") as HTMLElement;
    const rows = this.root.querySelector("#oauth-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.oauthStatus();
      const v = t.oauthPanel;
      const entries: [string, string][] = [];
      let account = info.logged_in ? v.loggedIn : v.loggedOut;
      if (info.logged_in && info.expired) account += ` \u00B7 ${v.expiredTag}`;
      entries.push([v.syncAccount, account]);
      if (info.logged_in && info.scopes) entries.push([v.scopes, info.scopes]);
      if (info.expires_at > 0) {
        entries.push([v.expires, new Date(info.expires_at * 1000).toLocaleString()]);
      }
      if (info.token_preview) entries.push([v.tokenPreview, info.token_preview]);
      const gc = info.google_chat;
      if (gc) {
        entries.push([
          v.googleSecret,
          gc.client_secret_configured ? v.configured : v.notConfigured,
        ]);
        entries.push([
          v.accounts,
          gc.authorized_emails.length > 0 ? gc.authorized_emails.join(", ") : v.noneAccounts,
        ]);
      }
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Skills sync (P643): device identity, gate, opt-ins (hermes
   * `ulnclaw sync status` parity); remote manifest on demand. */
  private async loadSync(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-sync") as HTMLElement;
    const rows = this.root.querySelector("#sync-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.syncStatus();
      const v = t.syncPanel;
      const entries: [string, string][] = [
        [v.device, `${info.device_name || v.unnamed} \u00B7 ${info.device_id.slice(0, 12)}`],
        [
          v.gate,
          info.gate === "active"
            ? `${v.gateActive} \u00B7 ${info.base_url}`
            : `${v.gateInert}${info.gate_reason ? ` \u2014 ${info.gate_reason}` : ""}`,
        ],
        [v.optedIn, info.opted_in.length > 0 ? info.opted_in.join(", ") : v.noneOptedIn],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      const btn = this.root.querySelector("#sync-fetch-remote") as HTMLButtonElement;
      btn.style.display = info.gate === "active" ? "" : "none";
      btn.onclick = () => void this.fetchSyncRemote();
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** P643: on-demand remote manifest — skill names, origin devices,
   * file counts. */
  private async fetchSyncRemote(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const v = t.syncPanel;
    const btn = this.root.querySelector("#sync-fetch-remote") as HTMLButtonElement;
    const remote = this.root.querySelector("#sync-remote") as HTMLElement;
    btn.disabled = true;
    btn.textContent = v.fetching;
    try {
      const info = await client.syncStatus(true);
      remote.innerHTML = "";
      if (!info.remote || !info.remote.available) {
        const note = document.createElement("div");
        note.className = "config-note";
        note.textContent = `${v.remoteUnavailable}: ${info.remote?.error || "unknown"}`;
        remote.appendChild(note);
        return;
      }
      const skills = info.remote.skills || {};
      const names = Object.keys(skills);
      if (names.length === 0) {
        const note = document.createElement("div");
        note.className = "config-note";
        note.textContent = v.remoteEmpty;
        remote.appendChild(note);
        return;
      }
      for (const name of names) {
        const skill = skills[name];
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = name;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = `${v.fromWord} ${skill.device} \u00B7 ${skill.files} ${v.filesWord}`;
        row.append(labelEl, valueEl);
        remote.appendChild(row);
      }
    } catch (error) {
      remote.innerHTML = "";
      const note = document.createElement("div");
      note.className = "config-note";
      note.textContent = String(error);
      remote.appendChild(note);
    } finally {
      btn.disabled = false;
      btn.textContent = v.fetchRemote;
    }
  }

  /** Secret sources (P642): per-source posture (hermes secrets
   * status parity). Values are never shown — only presence. */
  private async loadSecrets(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-secrets") as HTMLElement;
    const rows = this.root.querySelector("#secrets-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.secretsStatus();
      const v = t.secretsPanel;
      const entries: [string, string][] = [];
      entries.push([
        v.order,
        info.order.length > 0 ? info.order.join(" \u2192 ") : v.noneEnabled,
      ]);
      if (info.command.enabled) {
        entries.push([
          "command",
          `${info.command.command_set ? v.configured : v.noCommand} \u00B7 timeout ${info.command.timeout_seconds}s`,
        ]);
      }
      if (info.bitwarden.enabled) {
        entries.push([
          "bitwarden",
          `${info.bitwarden.bws || v.bwsMissing} \u00B7 ${info.bitwarden.token_env}: ${
            info.bitwarden.token_present ? v.tokenPresent : v.tokenMissing
          }${info.bitwarden.project_id ? ` \u00B7 ${info.bitwarden.project_id}` : ""}`,
        ]);
      }
      if (info.onepassword.enabled) {
        entries.push([
          "onepassword",
          `${info.onepassword.op || v.opMissing} \u00B7 ${info.onepassword.bindings} ${v.bindingsWord} \u00B7 ${
            info.onepassword.token_present ? v.tokenPresent : v.tokenOptional
          }`,
        ]);
      }
      if (info.preserve_existing.length > 0) {
        entries.push([v.preserve, info.preserve_existing.join(", ")]);
      }
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Computer Use (P641): cua-driver discovery, config, and an
   * opt-in deep health report (hermes computer-use status/doctor). */
  private async loadComputerUse(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-computer-use") as HTMLElement;
    const rows = this.root.querySelector("#computer-use-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const info = await client.computerUseStatus();
      const v = t.computerUsePanel;
      const entries: [string, string][] = [
        [
          v.driver,
          info.installed
            ? `${info.driver}${info.version ? ` \u00B7 ${info.version}` : ""}`
            : v.notInstalled,
        ],
        [v.telemetry, info.config.cua_telemetry ? "on" : "off"],
        [v.maxDimension, String(info.config.max_image_dimension)],
        [v.captureAfter, info.config.capture_after_mode],
        [
          v.overlay,
          info.config.no_overlay === null
            ? v.overlayAuto
            : info.config.no_overlay
              ? v.overlayOff
              : v.overlayOn,
        ],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      if (!info.installed) {
        const hint = document.createElement("div");
        hint.className = "config-note";
        hint.textContent = info.install_hint;
        rows.appendChild(hint);
      }
      const btn = this.root.querySelector("#computer-use-doctor") as HTMLButtonElement;
      btn.onclick = () => void this.runComputerDoctor();
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** P641: deep health report — renders overall + per-check rows. */
  private async runComputerDoctor(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const v = t.computerUsePanel;
    const btn = this.root.querySelector("#computer-use-doctor") as HTMLButtonElement;
    const health = this.root.querySelector("#computer-use-health") as HTMLElement;
    btn.disabled = true;
    btn.textContent = v.running;
    try {
      const info = await client.computerUseStatus(true);
      health.innerHTML = "";
      if (info.health_error) {
        const note = document.createElement("div");
        note.className = "config-note";
        note.textContent = info.health_error;
        health.appendChild(note);
        return;
      }
      const report = info.health || {};
      const overall = document.createElement("div");
      overall.className = "monitoring-row";
      const labelEl = document.createElement("span");
      labelEl.className = "monitoring-label";
      labelEl.textContent = v.overall;
      const valueEl = document.createElement("span");
      valueEl.className = "monitoring-value";
      valueEl.textContent = String(report.overall ?? "unknown");
      overall.append(labelEl, valueEl);
      health.appendChild(overall);
      const checks = Array.isArray(report.checks) ? (report.checks as Record<string, unknown>[]) : [];
      for (const check of checks) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const name = document.createElement("span");
        name.className = "monitoring-label";
        name.textContent = `[${String(check.status ?? "?")}] ${String(check.name ?? "?")}`;
        const detail = document.createElement("span");
        detail.className = "monitoring-value";
        detail.textContent = String(check.detail ?? check.message ?? "");
        detail.title = detail.textContent;
        row.append(name, detail);
        health.appendChild(row);
      }
    } catch (error) {
      health.innerHTML = "";
      const note = document.createElement("div");
      note.className = "config-note";
      note.textContent = String(error);
      health.appendChild(note);
    } finally {
      btn.disabled = false;
      btn.textContent = v.runDoctor;
    }
  }

  /** State snapshots (P315): quick-backup inventory with
   * create/restore/prune over /api/backups (hermes `backup` parity). */
  private async loadBackups(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-backups") as HTMLElement;
    const rows = this.root.querySelector("#backups-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const snapshots = await client.backupsList();
      rows.innerHTML = "";
      if (!snapshots.length) {
        const empty = document.createElement("div");
        empty.className = "monitoring-row config-note";
        empty.textContent = t.backupsPanel.empty;
        rows.appendChild(empty);
      }
      for (const snapshot of snapshots) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = snapshot.id;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent = `${snapshot.files} files \u00b7 ${this.fmtBytes(snapshot.bytes)}`;
        const restore = document.createElement("button");
        restore.className = "ghost";
        restore.textContent = t.backupsPanel.restore;
        restore.onclick = () => this.restoreBackup(snapshot.id);
        const download = document.createElement("button");
        download.className = "ghost";
        download.textContent = t.backupsPanel.download;
        download.onclick = () => window.open(client.backupDownloadUrl(snapshot.id), "_blank");
        row.append(label, value, download, restore);
        rows.appendChild(row);
      }
      const actions = document.createElement("div");
      actions.className = "monitoring-row";
      const create = document.createElement("button");
      create.className = "ghost";
      create.textContent = t.backupsPanel.newSnapshot;
      create.onclick = () => this.createBackup();
      const prune = document.createElement("button");
      prune.className = "ghost";
      prune.textContent = t.backupsPanel.prune;
      prune.onclick = () => this.pruneBackups();
      const status = document.createElement("span");
      status.className = "config-note";
      status.id = "backups-status";
      actions.append(create, prune, status);
      rows.appendChild(actions);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Checkpoint store census (P317): sizes + per-project rows with a
   * prune action, over /api/checkpoints (hermes `checkpoint` parity). */
  private async loadCheckpoints(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-checkpoints") as HTMLElement;
    const rows = this.root.querySelector("#checkpoints-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.checkpointsStatus();
      rows.innerHTML = "";
      const sizeRow = document.createElement("div");
      sizeRow.className = "monitoring-row";
      const sizeLabel = document.createElement("span");
      sizeLabel.className = "monitoring-label";
      sizeLabel.textContent = t.checkpointsPanel.size;
      const sizeValue = document.createElement("span");
      sizeValue.className = "monitoring-value";
      sizeValue.textContent = `${this.fmtBytes(status.store_size_bytes)} / ${this.fmtBytes(status.total_size_bytes)}`;
      sizeRow.append(sizeLabel, sizeValue);
      rows.appendChild(sizeRow);

      if (!status.projects.length) {
        const empty = document.createElement("div");
        empty.className = "monitoring-row config-note";
        empty.textContent = t.checkpointsPanel.noProjects;
        rows.appendChild(empty);
      }
      for (const project of status.projects) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = project.workdir || project.hash.slice(0, 12);
        label.title = project.workdir;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent = `${project.commits} commits${project.exists ? "" : " \u00b7 missing dir"}`;
        row.append(label, value);
        if (project.exists) {
          const restore = document.createElement("button");
          restore.className = "ghost";
          restore.textContent = t.checkpointsPanel.restore;
          restore.onclick = () => void this.restoreProject(project.workdir);
          row.appendChild(restore);
        }
        rows.appendChild(row);
      }

      const actions = document.createElement("div");
      actions.className = "monitoring-row";
      const prune = document.createElement("button");
      prune.className = "ghost";
      prune.textContent = t.checkpointsPanel.prune;
      prune.onclick = () => this.pruneCheckpoints();
      const statusEl = document.createElement("span");
      statusEl.className = "config-note";
      statusEl.id = "checkpoints-status-note";
      actions.append(prune, statusEl);
      rows.appendChild(actions);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async pruneCheckpoints(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const raw = window.prompt(t.checkpointsPanel.prunePrompt, "7");
    if (raw === null) return;
    const days = Number.parseInt(raw, 10);
    if (!Number.isFinite(days) || days < 0) return;
    const note = this.root.querySelector("#checkpoints-status-note");
    try {
      const stats = await client.checkpointsPrune(days);
      await this.loadCheckpoints();
      const el = this.root.querySelector("#checkpoints-status-note");
      if (el) {
        el.textContent = t.checkpointsPanel.pruned
          .replace("{orphan}", String(stats.deleted_orphan))
          .replace("{stale}", String(stats.deleted_stale))
          .replace("{bytes}", this.fmtBytes(stats.bytes_freed));
      }
    } catch (error) {
      if (note) {
        note.textContent = t.checkpointsPanel.pruneFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        );
      }
    }
  }

  /** P347: list a project's checkpoints and restore a picked hash over
   * POST /api/checkpoints/restore. */
  private async restoreProject(workdir: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    const note = this.root.querySelector("#checkpoints-status-note");
    let checkpoints: CheckpointEntry[];
    try {
      checkpoints = await client.checkpointsList(workdir);
    } catch (error) {
      if (note) {
        note.textContent = t.checkpointsPanel.restoreFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        );
      }
      return;
    }
    if (!checkpoints.length) {
      if (note) note.textContent = t.checkpointsPanel.restoreEmpty;
      return;
    }
    const dialog = document.createElement("dialog");
    dialog.className = "learning-node-dialog";
    const title = document.createElement("h3");
    title.textContent = t.checkpointsPanel.restoreTitle;
    const sub = document.createElement("div");
    sub.className = "config-note";
    sub.textContent = workdir;
    const select = document.createElement("select");
    for (const checkpoint of checkpoints) {
      const option = document.createElement("option");
      option.value = checkpoint.hash;
      option.textContent = `${checkpoint.short_hash} \u00b7 ${checkpoint.timestamp} \u00b7 ${checkpoint.reason} (${checkpoint.files_changed} files)`;
      select.appendChild(option);
    }
    const status = document.createElement("div");
    status.className = "learning-node-status";
    const actions = document.createElement("div");
    actions.className = "learning-node-actions";
    const restoreBtn = document.createElement("button");
    restoreBtn.textContent = t.checkpointsPanel.restore;
    restoreBtn.onclick = async () => {
      const hash = select.value;
      const confirmText = t.checkpointsPanel.restoreConfirm
        .replace("{hash}", hash.slice(0, 12))
        .replace("{dir}", workdir);
      if (!window.confirm(confirmText)) return;
      try {
        await client.checkpointsRestore(workdir, hash);
        status.textContent = t.checkpointsPanel.restoreDone;
        setTimeout(() => {
          dialog.close();
          void this.loadCheckpoints();
        }, 500);
      } catch (error) {
        status.textContent = t.checkpointsPanel.restoreFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        );
      }
    };
    const closeBtn = document.createElement("button");
    closeBtn.className = "ghost";
    closeBtn.textContent = t.checkpointsPanel.close;
    closeBtn.onclick = () => dialog.close();
    actions.append(restoreBtn, closeBtn);
    dialog.append(title, sub, select, status, actions);
    this.root.appendChild(dialog);
    dialog.showModal();
    dialog.addEventListener("close", () => dialog.remove());
  }

  private backupStatus(message: string): void {
    const el = this.root.querySelector("#backups-status");
    if (el) el.textContent = message;
  }

  private async createBackup(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const label = window.prompt(t.backupsPanel.labelPrompt, "");
    if (label === null) return;
    try {
      const result = await client.backupCreate(label.trim() || undefined);
      await this.loadBackups();
      this.backupStatus(
        result.id
          ? t.backupsPanel.created.replace("{id}", result.id)
          : result.message || "",
      );
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.createFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private async restoreBackup(id: string): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.backupsPanel.restoreConfirm.replace("{id}", id))) return;
    try {
      await client.backupRestore(id);
      await this.loadBackups();
      this.backupStatus(t.backupsPanel.restored.replace("{id}", id));
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.restoreFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private async pruneBackups(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const raw = window.prompt(t.backupsPanel.prunePrompt, "20");
    if (raw === null) return;
    const keep = Number.parseInt(raw, 10);
    if (!Number.isFinite(keep) || keep < 1) return;
    try {
      const removed = await client.backupPrune(keep);
      await this.loadBackups();
      this.backupStatus(t.backupsPanel.pruned.replace("{count}", String(removed)));
    } catch (error) {
      this.backupStatus(
        t.backupsPanel.pruneFailed.replace(
          "{error}",
          error instanceof Error ? error.message : String(error),
        ),
      );
    }
  }

  private fmtBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  /** Session-store footprint + one-click optimize (FTS merge + VACUUM,
   * same work as `ulnclaw sessions optimize`). */
  private async loadStorage(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-storage") as HTMLElement;
    const rows = this.root.querySelector("#storage-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const stats = await client.storageStats();
      rows.innerHTML = "";
      const sizeRow = document.createElement("div");
      sizeRow.className = "monitoring-row";
      const sizeLabel = document.createElement("span");
      sizeLabel.className = "monitoring-label";
      sizeLabel.textContent = t.storagePanel.size;
      const sizeValue = document.createElement("span");
      sizeValue.className = "monitoring-value";
      sizeValue.textContent = `${this.fmtBytes(stats.size_bytes)}${
        stats.wal_bytes > 0 ? ` + ${this.fmtBytes(stats.wal_bytes)} WAL` : ""
      }`;
      sizeRow.append(sizeLabel, sizeValue);
      rows.appendChild(sizeRow);

      const countsRow = document.createElement("div");
      countsRow.className = "monitoring-row";
      const countsLabel = document.createElement("span");
      countsLabel.className = "monitoring-label";
      countsLabel.textContent = t.storagePanel.contents;
      const countsValue = document.createElement("span");
      countsValue.className = "monitoring-value";
      countsValue.textContent = t.storagePanel.counts
        .replace("{sessions}", String(stats.sessions))
        .replace("{messages}", String(stats.messages));
      countsRow.append(countsLabel, countsValue);
      rows.appendChild(countsRow);

      const pathRow = document.createElement("div");
      pathRow.className = "monitoring-row";
      const pathLabel = document.createElement("span");
      pathLabel.className = "monitoring-label";
      pathLabel.textContent = t.storagePanel.path;
      const pathValue = document.createElement("span");
      pathValue.className = "monitoring-value";
      pathValue.textContent = stats.db_path;
      pathValue.title = stats.db_path;
      pathRow.append(pathLabel, pathValue);
      rows.appendChild(pathRow);

      const actionRow = document.createElement("div");
      actionRow.className = "monitoring-row";
      const spacer = document.createElement("span");
      spacer.className = "monitoring-label";
      const optimize = document.createElement("button");
      optimize.className = "ghost";
      optimize.textContent = t.storagePanel.optimize;
      optimize.title = t.storagePanel.optimizeTitle;
      const note = document.createElement("span");
      note.className = "config-note storage-note";
      optimize.addEventListener("click", async () => {
        const current = this.client();
        if (!current) return;
        optimize.disabled = true;
        optimize.textContent = t.storagePanel.optimizing;
        try {
          const result = await current.storageOptimize();
          note.textContent = t.storagePanel.optimized
            .replace("{indexes}", String(result.merged_indexes))
            .replace("{before}", this.fmtBytes(result.before_bytes))
            .replace("{after}", this.fmtBytes(result.after_bytes));
          this.loadStorage().catch(() => undefined);
        } catch (error) {
          note.textContent = t.storagePanel.optimizeFailed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          );
          note.classList.add("error");
          optimize.disabled = false;
          optimize.textContent = t.storagePanel.optimize;
        }
      });
      actionRow.append(spacer, optimize, note);
      rows.appendChild(actionRow);

      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Kanban diagnostics: boards with open/total counts, current-board
   * status histogram, and the blocked-task list. */
  private async loadKanban(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-kanban") as HTMLElement;
    const rows = this.root.querySelector("#kanban-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const boards = await client.kanbanBoards();
      rows.innerHTML = "";
      if (boards.length === 0) {
        const empty = document.createElement("p");
        empty.className = "config-note";
        empty.textContent = t.kanbanPanel.none;
        rows.appendChild(empty);
        section.hidden = false;
        return;
      }
      for (const board of boards) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const label = document.createElement("span");
        label.className = "monitoring-label";
        label.textContent = `${board.name} (${board.slug})`;
        const value = document.createElement("span");
        value.className = "monitoring-value";
        value.textContent =
          t.kanbanPanel.openOf
            .replace("{open}", String(board.open_tasks))
            .replace("{total}", String(board.total_tasks)) +
          (board.current ? ` · ${t.kanbanPanel.current}` : "");
        row.append(label, value);
        rows.appendChild(row);
      }
      const current = boards.find((board) => board.current);
      if (current) {
        const tasks = await client.kanbanTasks(current.slug);
        const counts = new Map<string, number>();
        for (const task of tasks) {
          counts.set(task.status, (counts.get(task.status) || 0) + 1);
        }
        if (counts.size > 0) {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const label = document.createElement("span");
          label.className = "monitoring-label";
          label.textContent = t.kanbanPanel.byStatus;
          const value = document.createElement("span");
          value.className = "monitoring-value";
          value.textContent = Array.from(counts.entries())
            .map(([status, count]) => `${status}: ${count}`)
            .join(" · ");
          row.append(label, value);
          rows.appendChild(row);
        }
        const blocked = tasks.filter((task) => task.status === "blocked");
        if (blocked.length > 0) {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const label = document.createElement("span");
          label.className = "monitoring-label";
          label.textContent = t.kanbanPanel.blocked;
          const value = document.createElement("span");
          value.className = "monitoring-value";
          value.textContent = blocked
            .slice(0, 8)
            .map((task) => task.title || task.id.slice(0, 8))
            .join("; ");
          row.append(label, value);
          rows.appendChild(row);
        }
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Messaging-platform enabled posture (hermes ChannelsPage parity). */
  private async loadChannels(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-channels") as HTMLElement;
    const rows = this.root.querySelector("#channels-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      let platforms: MessagingPlatform[] | null = null;
      try {
        platforms = await client.messagingPlatforms();
      } catch {
        platforms = null; // older gateway: fall back to /api/channels
      }
      rows.innerHTML = "";

      const stateBadge = (state: string): string => {
        const cls = state === "connected" ? "ok" : state === "disabled" ? "" : "warn";
        const label =
          state === "connected"
            ? t.channelsPanel.stateConnected
            : state === "not_configured"
              ? t.channelsPanel.stateNotConfigured
              : state;
        return `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(label)}</span>`;
      };

      if (platforms && platforms.length) {
        const enabled = platforms.filter((platform) => platform.enabled);
        const disabled = platforms.filter((platform) => !platform.enabled);

        const enabledRow = document.createElement("div");
        enabledRow.className = "monitoring-row";
        const enabledLabel = document.createElement("span");
        enabledLabel.className = "monitoring-label";
        enabledLabel.textContent = t.channelsPanel.enabled;
        const enabledValue = document.createElement("span");
        enabledValue.className = "monitoring-value";
        enabledValue.innerHTML = enabled.length
          ? enabled
              .map((platform) => `${escapeHtmlDoctor(platform.name)} ${stateBadge(platform.state)}`)
              .join(" ")
          : escapeHtmlDoctor(t.channelsPanel.noneEnabled);
        enabledRow.append(enabledLabel, enabledValue);
        rows.appendChild(enabledRow);

        const disabledRow = document.createElement("div");
        disabledRow.className = "monitoring-row";
        const disabledLabel = document.createElement("span");
        disabledLabel.className = "monitoring-label";
        disabledLabel.textContent = t.channelsPanel.disabled;
        const disabledValue = document.createElement("span");
        disabledValue.className = "monitoring-value channels-disabled";
        disabledValue.textContent = disabled.map((platform) => platform.name).join(", ");
        disabledRow.append(disabledLabel, disabledValue);
        rows.appendChild(disabledRow);

        // Per-enabled-platform probe rows (hermes ChannelsPage test button).
        for (const platform of enabled) {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const label = document.createElement("span");
          label.className = "monitoring-label";
          label.textContent = platform.name;
          label.title = platform.description;
          const value = document.createElement("span");
          value.className = "monitoring-value";
          const note = document.createElement("span");
          note.className = "jobs-counts";
          const missingEnv = platform.env_vars.filter((envVar) => envVar.required && !envVar.is_set);
          note.textContent = platform.configured
            ? platform.state
            : `${platform.state} \u00b7 ${t.channelsPanel.stateNotConfigured}`;
          value.appendChild(note);
          const test = document.createElement("button");
          test.className = "ghost";
          test.textContent = t.channelsPanel.test;
          test.onclick = async () => {
            test.disabled = true;
            try {
              const result = await client.messagingPlatformTest(platform.id);
              note.textContent = result.message;
            } catch (error) {
              note.textContent = error instanceof Error ? error.message : String(error);
            } finally {
              test.disabled = false;
            }
          };
          if (missingEnv.length) {
            note.textContent += ` \u00b7 ${missingEnv.map((envVar) => envVar.key).join(", ")}`;
          }
          value.appendChild(test);
          row.append(label, value);
          rows.appendChild(row);
        }
      } else {
        // P720: deepened channels panel over the P699 payload — state
        // ladder, home channel and known-channel counts per enabled
        // platform, plus the newest seen-channel per platform.
        const status = await client.channelsStatus();
        const enabled = status.channels.filter((channel) => channel.enabled);
        const disabled = status.channels.filter((channel) => !channel.enabled);

        const addRow = (label: string, valueHtml: string): void => {
          const row = document.createElement("div");
          row.className = "monitoring-row";
          const labelEl = document.createElement("span");
          labelEl.className = "monitoring-label";
          labelEl.textContent = label;
          const valueEl = document.createElement("span");
          valueEl.className = "monitoring-value";
          valueEl.innerHTML = valueHtml;
          row.append(labelEl, valueEl);
          rows.appendChild(row);
        };

        addRow(
          t.channelsPanel.summary,
          escapeHtmlDoctor(
            t.channelsPanel.summaryValue
              .replace("{connected}", String(status.connected_count))
              .replace("{enabled}", String(status.enabled_count)),
          ),
        );

        if (enabled.length) {
          for (const channel of enabled) {
            const cls =
              channel.state === "connected"
                ? "ok"
                : channel.state === "not_configured"
                  ? "warn"
                  : "";
            const badge = `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(channel.state || "?")}</span>`;
            const home = channel.home_channel
              ? ` \u00b7 ${escapeHtmlDoctor(t.channelsPanel.homeChannel)}: ${escapeHtmlDoctor(channel.home_channel)}`
              : "";
            const known = ` \u00b7 ${escapeHtmlDoctor(
              t.channelsPanel.knownChannels.replace("{count}", String(channel.known_channels)),
            )}`;
            addRow(channel.name, `${badge}${home}${known}`);
          }
        } else {
          addRow(t.channelsPanel.enabled, escapeHtmlDoctor(t.channelsPanel.noneEnabled));
        }

        if (disabled.length) {
          const names = disabled.map((channel) => escapeHtmlDoctor(channel.name)).join(", ");
          addRow(t.channelsPanel.disabled, `<span class="channels-disabled">${names}</span>`);
        }

        const directoryPlatforms = Object.keys(status.directory || {}).slice(0, 6);
        for (const platform of directoryPlatforms) {
          const entries = status.directory[platform] || [];
          if (!entries.length) continue;
          const newest = entries[0];
          const label = newest.name && newest.name !== newest.id ? newest.name : newest.id;
          const kind = newest.type ? ` (${escapeHtmlDoctor(newest.type)})` : "";
          addRow(
            t.channelsPanel.recent,
            `${escapeHtmlDoctor(platform)} \u2192 ${escapeHtmlDoctor(label)}${kind} \u00b7 ${escapeHtmlDoctor(newest.updated_iso)}`,
          );
        }
      }

      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Learning-graph census (hermes star-map parity): node/edge counts,
   * density, and top clusters from GET /api/learning/graph. */
  private async loadLearning(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-learning") as HTMLElement;
    const rows = this.root.querySelector("#learning-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const graph = await client.learningGraph();
      const num = (key: string): number => {
        const value = graph.stats[key];
        return typeof value === "number" ? value : 0;
      };
      const skillNodes = graph.nodes.filter((node) => node.kind === "skill").length;
      const memoryNodes = graph.nodes.filter((node) => node.kind === "memory").length;
      const v = t.learningPanel;
      const entries: [string, string][] = [
        [v.skills, String(num("learned_skills") || skillNodes)],
        [v.memoryNodes, String(num("memory_nodes") || memoryNodes)],
        [
          v.edges,
          `${num("related_edges") + num("memory_skill_edges")} (${num("related_edges")} ${v.skillEdgesWord} \u00b7 ${num("memory_skill_edges")} ${v.memoryEdgesWord})`,
        ],
        [v.density, String(num("edges_per_node"))],
        [v.linked, `${num("linked_nodes")} (${num("isolated_pct")}% ${v.isolated})`],
        [v.origin, `${num("agent_created")} ${v.agentCreatedWord} \u00b7 ${num("used")} ${v.usedWord}`],
        [v.categories, String(num("categories") || graph.clusters.length)],
      ];
      rows.innerHTML = "";
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        valueEl.title = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      if (graph.clusters.length > 0) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = v.topCategories;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = graph.clusters
          .slice(0, 8)
          .map((cluster) => `${cluster.category} \u00d7${cluster.count}`)
          .join(" \u00b7 ");
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      const hint = document.createElement("div");
      hint.className = "config-note";
      hint.textContent = v.hint;
      rows.appendChild(hint);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Egress-proxy status text (tokens redacted server-side). */
  private async loadEgress(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-egress") as HTMLElement;
    const body = this.root.querySelector("#egress-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      body.textContent = await client.egressStatus();
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** Show the update panel when a gateway is connected (P324). */
  private async loadUpdate(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-update") as HTMLElement;
    section.hidden = !client;
  }

  /** GET /api/update/check and render the outcome (P324). */
  private async checkUpdate(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const note = this.root.querySelector("#update-status") as HTMLElement;
    const body = this.root.querySelector("#update-body") as HTMLPreElement;
    const applyBtn = this.root.querySelector("#update-apply-btn") as HTMLButtonElement;
    note.textContent = t.updatePanel.checking;
    body.hidden = true;
    applyBtn.hidden = true;
    try {
      const result = await client.updateCheck();
      let headline: string;
      if (result.error) {
        headline = t.updatePanel.checkFailed.replace("{error}", result.error);
      } else if (result.behind === 0) {
        headline = `${t.updatePanel.upToDate} (${result.current_version})`;
      } else if (result.behind === -1) {
        headline = t.updatePanel.behindShallow;
      } else {
        headline = t.updatePanel.behind
          .replace("{count}", String(result.behind ?? 0))
          .replace("{version}", result.current_version);
      }
      note.textContent = headline;
      if (result.log && result.log.length) {
        body.textContent = result.log.join("\n");
        body.hidden = false;
      }
      applyBtn.hidden = !(result.update_available && result.can_apply);
    } catch (error) {
      note.textContent = t.updatePanel.checkFailed.replace("{error}", String(error));
    }
  }

  /** POST /api/update after confirmation and render the report (P324). */
  private async applyUpdate(): Promise<void> {
    const client = this.client();
    if (!client) return;
    if (!window.confirm(t.updatePanel.applyConfirm)) return;
    const note = this.root.querySelector("#update-status") as HTMLElement;
    const body = this.root.querySelector("#update-body") as HTMLPreElement;
    note.textContent = t.updatePanel.applying;
    try {
      const report = await client.updateApply();
      note.textContent = t.updatePanel.applyDone
        .replace("{commits}", String(report.new_commits))
        .replace("{sha}", (report.new_sha || "").slice(0, 8));
      body.textContent = report.log.join("\n");
      body.hidden = false;
    } catch (error) {
      note.textContent = t.updatePanel.applyFailed.replace("{error}", String(error));
    }
  }

  /** Show the ops panel when a gateway is connected (P321). */
  private async loadOps(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-ops") as HTMLElement;
    section.hidden = !client;
  }

  /** Run an ops action over /api/ops/* and render the output (P321). */
  private async runOps(action: "securityAudit" | "promptSize" | "dump"): Promise<void> {
    const client = this.client();
    if (!client) return;
    const body = this.root.querySelector("#ops-body") as HTMLPreElement;
    const note = this.root.querySelector("#ops-status") as HTMLElement;
    const label = t.opsPanel[action];
    note.textContent = t.opsPanel.running.replace("{action}", label);
    body.hidden = true;
    try {
      let text: string;
      if (action === "securityAudit") {
        const report = await client.opsSecurityAudit();
        text =
          report.finding_count === 0
            ? report.note ?? t.opsPanel.auditClean.replace("{total}", String(report.total_components_scanned))
            : report.findings
                .map((finding) => {
                  const fix = finding.vuln.fixed_versions.length
                    ? ` (fix: ${finding.vuln.fixed_versions.join(", ")})`
                    : "";
                  return `[${finding.vuln.severity}] ${finding.component.name}@${finding.component.version} \u2014 ${finding.vuln.osv_id}: ${finding.vuln.summary}${fix}`;
                })
                .join("\n");
      } else if (action === "promptSize") {
        const report = await client.opsPromptSize();
        const lines = [
          `model: ${report.model} (${report.provider})`,
          `system prompt: ${report.system_prompt_chars} chars / ${report.system_prompt_bytes} bytes`,
          "",
          ...report.sections.map((row) => `  ${row.label}: ${row.chars} chars / ${row.bytes} bytes`),
          "",
          `tools: ${report.tools_count} tools / ${report.tools_json_bytes} bytes of JSON schema`,
          ...report.toolsets.map((row) => `  ${row.toolset}: ${row.tools} tools / ${row.json_bytes} bytes`),
          "",
          `skills: ${report.skills.length} installed / ${report.skills_total_bytes} bytes on disk`,
        ];
        text = lines.join("\n");
      } else {
        text = await client.opsDump();
      }
      body.textContent = text;
      body.hidden = false;
      note.textContent = "";
    } catch (error) {
      note.textContent = t.opsPanel.failed
        .replace("{action}", label)
        .replace("{error}", String(error));
    }
  }

  /** Raw Prometheus exposition from GET /metrics (collapsible). */
  private async loadMetrics(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-metrics") as HTMLElement;
    const body = this.root.querySelector("#metrics-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      body.textContent = await client.metricsRaw();
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadLogs(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-logs") as HTMLElement;
    const body = this.root.querySelector("#logs-body") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const level = (this.root.querySelector("#logs-level") as HTMLSelectElement).value || undefined;
      const file = (this.root.querySelector("#logs-file") as HTMLSelectElement).value || "gateway";
      const search =
        (this.root.querySelector("#logs-search") as HTMLInputElement).value.trim() || undefined;
      const payload = await client.logsFile(file, { lines: LOGS_LINES, level, search });
      body.textContent = payload.lines.join("\n");
      (this.root.querySelector("#logs-path") as HTMLElement).textContent = payload.path;
      section.hidden = false;
      const refresh = this.root.querySelector("#logs-refresh") as HTMLButtonElement;
      if (!refresh.dataset.wired) {
        refresh.dataset.wired = "1";
        refresh.addEventListener("click", () => this.loadLogs().catch(() => undefined));
        (this.root.querySelector("#logs-level") as HTMLSelectElement).addEventListener(
          "change",
          () => this.loadLogs().catch(() => undefined),
        );
        (this.root.querySelector("#logs-file") as HTMLSelectElement).addEventListener(
          "change",
          () => this.loadLogs().catch(() => undefined),
        );
        (this.root.querySelector("#logs-search") as HTMLInputElement).addEventListener(
          "keydown",
          (event) => {
            if (event.key === "Enter") this.loadLogs().catch(() => undefined);
          },
        );
      }
    } catch {
      section.hidden = true;
    }
  }

  /** P702: bounded readiness report from GET /health/detailed
   * (hermes readiness parity surfaced in the shell; P697 gateway side). */
  private async loadGatewayHealth(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-gateway-health") as HTMLElement;
    const rows = this.root.querySelector("#gateway-health-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const detailed = await client.healthDetailed();
      if (!detailed || !detailed.readiness) {
        section.hidden = true;
        return;
      }
      rows.innerHTML = "";
      const badge = (status: string): string => {
        const cls = status === "ok" ? "ok" : "warn";
        const label = status === "ok" ? t.gatewayHealth.ok : t.gatewayHealth.degraded;
        return `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(label)}</span>`;
      };
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(t.gatewayHealth.overall, badge(detailed.readiness.status));

      const checks = detailed.readiness.checks ?? {};
      const checkNames: [string, string][] = [
        ["state_db", t.gatewayHealth.stateDb],
        ["config", t.gatewayHealth.config],
        ["model", t.gatewayHealth.model],
        ["disk", t.gatewayHealth.disk],
        ["gateway", t.gatewayHealth.gateway],
        ["background_queues", t.gatewayHealth.queues],
      ];
      for (const [key, label] of checkNames) {
        const check = checks[key];
        if (!check) continue;
        let extra = "";
        if (key === "disk" && typeof check.used_percent === "number") {
          extra = ` \u00b7 ${t.gatewayHealth.usedPercent.replace("{pct}", String(check.used_percent))}`;
        }
        if (key === "gateway") {
          const connected = check.connected_platforms;
          const total = check.platforms;
          if (typeof connected === "number" && typeof total === "number") {
            extra = ` \u00b7 ${t.gatewayHealth.connectedPlatforms.replace("{connected}", String(connected)).replace("{total}", String(total))}`;
          }
        }
        if (key === "background_queues") {
          const runs = check.active_api_runs;
          const queued = check.queued_prompts;
          if (typeof runs === "number" && typeof queued === "number") {
            extra = ` \u00b7 ${t.gatewayHealth.queueCounts.replace("{runs}", String(runs)).replace("{queued}", String(queued))}`;
          }
        }
        const detail = typeof check.detail === "string" && check.detail
          ? ` \u00b7 ${escapeHtmlDoctor(check.detail)}`
          : "";
        addRow(label, `${badge(check.status)}${extra}${detail}`);
      }

      addRow(
        t.gatewayHealth.gatewayState,
        escapeHtmlDoctor(detailed.gateway_state ?? "?") +
          (detailed.gateway_busy ? ` \u00b7 ${t.gatewayHealth.busy}` : ""),
      );
      if (typeof detailed.active_agents === "number") {
        addRow(t.gatewayHealth.activeAgents, String(detailed.active_agents));
      }
      if (typeof detailed.uptime_seconds === "number") {
        addRow(t.gatewayHealth.uptime, this.fmtUptime(detailed.uptime_seconds));
      }
      if (typeof detailed.pid === "number") {
        addRow(t.gatewayHealth.pid, String(detailed.pid));
      }
      addRow(
        t.gatewayHealth.restartLoop,
        detailed.restart_loop_tripped
          ? `<span class="models-view-badge warn">${escapeHtmlDoctor(t.gatewayHealth.tripped)}</span>`
          : escapeHtmlDoctor(t.gatewayHealth.calm),
      );
      if (detailed.code_skew) {
        addRow(
          t.gatewayHealth.codeSkew,
          `<span class="models-view-badge warn">${escapeHtmlDoctor(t.gatewayHealth.codeSkewStale)}</span>` +
            ` \u00b7 ${escapeHtmlDoctor(detailed.code_skew.boot)} \u2192 ${escapeHtmlDoctor(detailed.code_skew.disk)}`,
        );
      }
      if (detailed.previous_exit_label) {
        const label =
          detailed.previous_exit_label === "clean"
            ? t.gatewayHealth.exitClean
            : detailed.previous_exit_label === "unclean"
              ? t.gatewayHealth.exitUnclean
              : t.gatewayHealth.exitUnknown;
        const cls = detailed.previous_exit_label === "unclean" ? "warn" : "ok";
        addRow(
          t.gatewayHealth.previousExit,
          `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(label)}</span>`,
        );
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** P706: crash-safe delivery ledger rows (P700-P705 parity). */
  private async loadDeliveryLedger(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-delivery-ledger") as HTMLElement;
    const rows = this.root.querySelector("#delivery-ledger-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const ledger = await client.deliveryLedger();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const stateBadge = (state: string): string => {
        const cls = state === "delivered" ? "ok" : state === "pending" ? "" : "warn";
        return `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(state)}</span>`;
      };
      addRow(t.deliveryLedger.outstanding, String(ledger.outstanding));
      const counts = Object.entries(ledger.counts)
        .map(([state, count]) => `${escapeHtmlDoctor(state)}: ${count}`)
        .join(" \u00b7 ");
      addRow(t.deliveryLedger.byState, counts || escapeHtmlDoctor(t.deliveryLedger.empty));
      const recent = ledger.obligations
        .filter((obligation) => obligation.state !== "delivered")
        .slice(0, 5);
      for (const obligation of recent) {
        const when = new Date(obligation.updated_at * 1000).toLocaleString();
        addRow(
          `${obligation.platform} \u2192 ${obligation.chat_id}`,
          `${stateBadge(obligation.state)} \u00b7 ${t.deliveryLedger.attempts.replace("{n}", String(obligation.attempts))} \u00b7 ${escapeHtmlDoctor(when)}`,
        );
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** P713: confirmed-dead delivery targets (P707 registry surface). */
  private async loadDeadTargets(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-dead-targets") as HTMLElement;
    const rows = this.root.querySelector("#dead-target-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.deadTargets();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(t.deadTargets.count, String(payload.count));
      if (payload.count === 0) {
        addRow(t.deadTargets.empty, "");
      } else {
        for (const target of payload.targets.slice(0, 8)) {
          const when = new Date(target.marked_at * 1000).toLocaleString();
          addRow(
            `${target.platform} \u2192 ${target.chat_id}`,
            `<span class="models-view-badge warn">${escapeHtmlDoctor(target.reason)}</span> \u00b7 ${escapeHtmlDoctor(when)}`,
          );
        }
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadStallWatch(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-stall-watch") as HTMLElement;
    const rows = this.root.querySelector("#stall-watch-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.stallWatch();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(
        t.stallWatch.timeout,
        payload.watcher_enabled
          ? `${Math.round(payload.timeout_seconds)}s`
          : escapeHtmlDoctor(t.stallWatch.disabled),
      );
      addRow(t.stallWatch.pending, String(payload.pending_count));
      if (payload.pending_count === 0) {
        addRow(t.stallWatch.empty, "");
      } else {
        for (const parked of payload.rows.slice(0, 8)) {
          const idle =
            parked.idle_seconds === null ? "?" : `${Math.round(parked.idle_seconds)}s`;
          const badge = parked.stalled
            ? `<span class="models-view-badge warn">${escapeHtmlDoctor(t.stallWatch.stalled)}</span> \u00b7 `
            : "";
          const detail = parked.description
            ? ` \u00b7 ${escapeHtmlDoctor(parked.description)}`
            : "";
          addRow(
            `${parked.platform} \u2192 ${parked.chat_id}`,
            `${badge}${escapeHtmlDoctor(idle)}${detail}`,
          );
        }
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadMonitoringSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-monitoring-settings") as HTMLElement;
    const rows = this.root.querySelector("#monitoring-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.monitoringSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const tri = (value: boolean | null): string =>
        value === null ? escapeHtmlDoctor(auto) : value ? on : off;
      addRow(
        t.monitoringSettingsPanel.installId,
        payload.install_id !== null ? escapeHtmlDoctor(payload.install_id) : escapeHtmlDoctor(auto),
      );
      addRow(t.monitoringSettingsPanel.healthExport, tri(payload.gateway_health_export_enabled));
      addRow(t.monitoringSettingsPanel.metrics, tri(payload.metrics_enabled));
      addRow(t.monitoringSettingsPanel.diagnosticEvents, tri(payload.diagnostic_events_enabled));
      addRow(t.monitoringSettingsPanel.warningEvents, tri(payload.warning_error_events_enabled));
      addRow(
        t.monitoringSettingsPanel.exportInterval,
        payload.export_interval_seconds !== null
          ? escapeHtmlDoctor(String(payload.export_interval_seconds))
          : escapeHtmlDoctor(auto),
      );
      addRow(
        t.monitoringSettingsPanel.logsInterval,
        payload.logs_export_interval_seconds !== null
          ? escapeHtmlDoctor(String(payload.logs_export_interval_seconds))
          : escapeHtmlDoctor(auto),
      );
      addRow(t.monitoringSettingsPanel.otlpEnabled, tri(payload.otlp_enabled));
      addRow(
        t.monitoringSettingsPanel.otlpEndpoint,
        payload.otlp_endpoint !== null
          ? escapeHtmlDoctor(payload.otlp_endpoint)
          : escapeHtmlDoctor(auto),
      );
      // P768: monitoring editor — booleans take true/false, cadences
      // take integers (floors enforced server-side), strings persist as
      // typed; Apply goes through PUT /api/monitoring-settings, Clear
      // removes the override. Applies on gateway restart.
      const editor = this.root.querySelector("#monitoring-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#monitoring-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#monitoring-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#monitoring-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#monitoring-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#monitoring-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "install_id",
          "gateway_health_export_enabled",
          "metrics_enabled",
          "diagnostic_events_enabled",
          "warning_error_events_enabled",
          "export_interval_seconds",
          "logs_export_interval_seconds",
          "otlp_enabled",
          "otlp_endpoint",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (
          value: string | number | boolean | null,
        ): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateMonitoringSetting(keySel.value, value);
            await this.loadMonitoringSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true"
              ? true
              : raw === "false"
                ? false
                : /^\d+$/.test(raw)
                  ? Number(raw)
                  : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadTimezoneSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-timezone-settings") as HTMLElement;
    const rows = this.root.querySelector("#timezone-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.timezoneSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.timezonePanel.configured,
        payload.timezone !== null ? escapeHtmlDoctor(payload.timezone) : escapeHtmlDoctor(auto),
      );
      addRow(t.timezonePanel.envOverride, payload.env_override ? on : off);
      addRow(
        t.timezonePanel.effective,
        payload.effective !== null ? escapeHtmlDoctor(payload.effective) : escapeHtmlDoctor(auto),
      );
      if (payload.effective !== null) {
        const cls = payload.valid ? "ok" : "warn";
        addRow(
          t.timezonePanel.valid,
          `<span class="models-view-badge ${cls}">${escapeHtmlDoctor(payload.valid ? on : off)}</span>`,
        );
      }
      // P767: timezone editor — type an IANA zone name, Apply persists
      // through PUT /api/timezone-settings (validated server-side),
      // Clear returns to server-local time.
      const editor = this.root.querySelector("#timezone-editor") as HTMLElement;
      const keySel = this.root.querySelector("#timezone-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#timezone-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#timezone-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#timezone-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#timezone-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        keySel.innerHTML = "";
        const option = document.createElement("option");
        option.value = "timezone";
        option.textContent = "timezone";
        keySel.appendChild(option);
        const applyEdit = async (value: string | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateTimezoneSetting(keySel.value, value);
            await this.loadTimezoneSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueInput.value.trim());
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadDashboardTheme(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-dashboard-theme") as HTMLElement;
    const rows = this.root.querySelector("#dashboard-theme-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const [themes, fontInfo] = await Promise.all([
        client.dashboardThemes(),
        client.dashboardFontInfo(),
      ]);
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const activeTheme = themes.themes.find((theme) => theme.name === themes.active);
      addRow(
        t.dashboardThemePanel.activeTheme,
        escapeHtmlDoctor(
          activeTheme ? `${activeTheme.label} (${activeTheme.name})` : themes.active,
        ),
      );
      addRow(t.dashboardThemePanel.activeFont, escapeHtmlDoctor(fontInfo.font));
      addRow(
        t.dashboardThemePanel.themes,
        themes.themes.map((theme) => escapeHtmlDoctor(theme.name)).join(", "),
      );
      addRow(
        t.dashboardThemePanel.fontChoices,
        fontInfo.font_choices.map(escapeHtmlDoctor).join(", "),
      );
      // P764: dashboard theme/font editor — pick the knob, type a
      // catalog name, Apply persists through PUT /api/dashboard/theme
      // or /api/dashboard/font, Reset returns to the default theme /
      // the theme's own font.
      const editor = this.root.querySelector("#dashboard-theme-editor") as HTMLElement;
      const keySel = this.root.querySelector("#dashboard-theme-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#dashboard-theme-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#dashboard-theme-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#dashboard-theme-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#dashboard-theme-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        keySel.innerHTML = "";
        for (const key of ["theme", "font"]) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string): Promise<void> => {
          statusEl.textContent = "";
          try {
            if (keySel.value === "theme") {
              await client.dashboardSetTheme(value);
            } else {
              await client.dashboardSetFont(value);
            }
            await this.loadDashboardTheme();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueInput.value.trim());
        clearBtn.onclick = () =>
          void applyEdit(keySel.value === "theme" ? "default" : "theme");
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadBrowserSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-browser-settings") as HTMLElement;
    const rows = this.root.querySelector("#browser-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.browserSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.browserSettingsPanel.cdpUrl,
        payload.cdp_url !== null ? escapeHtmlDoctor(payload.cdp_url) : escapeHtmlDoctor(auto),
      );
      addRow(t.browserSettingsPanel.cdpEnvOverride, payload.cdp_env_override ? on : off);
      addRow(
        t.browserSettingsPanel.cloudProvider,
        payload.cloud_provider !== null
          ? escapeHtmlDoctor(payload.cloud_provider)
          : escapeHtmlDoctor(auto),
      );
      addRow(
        t.browserSettingsPanel.useGateway,
        payload.use_gateway !== null ? (payload.use_gateway ? on : off) : off,
      );
      addRow(
        t.browserSettingsPanel.validProviders,
        payload.valid_cloud_providers.map(escapeHtmlDoctor).join(", "),
      );
      // P763: browser settings editor — cdp_url/cloud_provider take
      // strings (auto/URL, provider name), use_gateway takes
      // true/false; Apply persists through PUT /api/browser-settings,
      // Clear restores auto/legacy behavior.
      const editor = this.root.querySelector("#browser-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#browser-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#browser-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#browser-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#browser-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#browser-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["cdp_url", "cloud_provider", "use_gateway"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateBrowserSetting(keySel.value, value);
            await this.loadBrowserSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | boolean =
            raw === "true" ? true : raw === "false" ? false : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadPetsSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-pets-settings") as HTMLElement;
    const rows = this.root.querySelector("#pets-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.petsSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.petsPanel.baseUrl,
        payload.image_base_url !== null
          ? escapeHtmlDoctor(payload.image_base_url)
          : escapeHtmlDoctor(auto),
      );
      addRow(
        t.petsPanel.model,
        payload.image_model !== null ? escapeHtmlDoctor(payload.image_model) : escapeHtmlDoctor(auto),
      );
      addRow(t.petsPanel.apiKey, payload.image_api_key_configured ? on : off);
      const envParts: string[] = [];
      if (payload.openai_key_env) envParts.push("OPENAI_API_KEY");
      if (payload.ulnclaw_key_env) envParts.push("ULNCLAW_API_KEY");
      addRow(
        t.petsPanel.envFallback,
        envParts.length > 0
          ? envParts.map(escapeHtmlDoctor).join(", ")
          : escapeHtmlDoctor(off),
      );
      // P762: pet image editor — base URL/model overrides persist
      // through PUT /api/pets-settings, Clear restores the defaults;
      // the API key is secret material and stays out of the shell.
      const editor = this.root.querySelector("#pets-editor") as HTMLElement;
      const keySel = this.root.querySelector("#pets-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#pets-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#pets-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#pets-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#pets-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["image_base_url", "image_model"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updatePetsSetting(keySel.value, value);
            await this.loadPetsSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueInput.value.trim());
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadDiscordSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-discord-settings") as HTMLElement;
    const rows = this.root.querySelector("#discord-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.discordSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      addRow(
        t.discordPanel.serverActions,
        payload.server_actions !== null
          ? payload.server_actions.map(escapeHtmlDoctor).join(", ")
          : escapeHtmlDoctor(`${auto} (${payload.known_actions.length})`),
      );
      addRow(
        t.discordPanel.knownActions,
        payload.known_actions.map(escapeHtmlDoctor).join(", "),
      );
      // P761: Discord allowlist editor — type comma-separated action
      // names, Apply persists through PUT /api/discord-settings,
      // Clear removes the allowlist; the panel re-renders from disk
      // state afterwards.
      const editor = this.root.querySelector("#discord-editor") as HTMLElement;
      const keySel = this.root.querySelector("#discord-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#discord-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#discord-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#discord-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#discord-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        keySel.innerHTML = "";
        const option = document.createElement("option");
        option.value = "server_actions";
        option.textContent = "server_actions";
        keySel.appendChild(option);
        const applyEdit = async (value: string[] | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateDiscordSetting(keySel.value, value);
            await this.loadDiscordSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const names = valueInput.value
            .split(",")
            .map((part) => part.trim())
            .filter((part) => part.length > 0);
          void applyEdit(names);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadMoaSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-moa-settings") as HTMLElement;
    const rows = this.root.querySelector("#moa-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.moaSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.moaPanel.defaultPreset,
        payload.default_preset !== null ? escapeHtmlDoctor(payload.default_preset) : escapeHtmlDoctor(auto),
      );
      addRow(t.moaPanel.saveTraces, payload.save_traces ? on : off);
      addRow(
        t.moaPanel.traceDir,
        payload.trace_dir !== null ? escapeHtmlDoctor(payload.trace_dir) : escapeHtmlDoctor(auto),
      );
      addRow(
        t.moaPanel.privacyFilter,
        payload.privacy_filter !== null ? escapeHtmlDoctor(payload.privacy_filter) : off,
      );
      addRow(
        t.moaPanel.presets,
        payload.preset_names.length > 0
          ? payload.preset_names.map(escapeHtmlDoctor).join(", ")
          : escapeHtmlDoctor(auto),
      );
      // P760: MoA editor — pick a key, type the value, Apply persists
      // through PUT /api/moa-settings, Clear removes the override; the
      // panel re-renders from disk state afterwards. save_traces takes
      // true/false; privacy_filter accepts off|display|full.
      const editor = this.root.querySelector("#moa-editor") as HTMLElement;
      const keySel = this.root.querySelector("#moa-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#moa-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#moa-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#moa-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#moa-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["default_preset", "save_traces", "trace_dir", "privacy_filter"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateMoaSetting(keySel.value, value);
            await this.loadMoaSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | boolean =
            raw === "true" ? true : raw === "false" ? false : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadVideoGenSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-video-gen-settings") as HTMLElement;
    const rows = this.root.querySelector("#video-gen-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.videoGenSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const auto = t.webSettingsPanel.autoWord;
      addRow(
        t.videoGenPanel.provider,
        payload.provider !== null ? escapeHtmlDoctor(payload.provider) : escapeHtmlDoctor(auto),
      );
      addRow(
        t.videoGenPanel.model,
        payload.model !== null ? escapeHtmlDoctor(payload.model) : escapeHtmlDoctor(auto),
      );
      addRow(
        t.videoGenPanel.falModel,
        payload.fal_model !== null ? escapeHtmlDoctor(payload.fal_model) : escapeHtmlDoctor(auto),
      );
      // P759: video-gen editor — pick a key, type the value, Apply
      // persists through PUT /api/video-gen-settings, Clear restores
      // auto-select; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#video-gen-editor") as HTMLElement;
      const keySel = this.root.querySelector("#video-gen-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#video-gen-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#video-gen-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#video-gen-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#video-gen-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["provider", "model", "fal_model"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateVideoGenSetting(keySel.value, value);
            await this.loadVideoGenSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueInput.value.trim());
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadXSearchSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-x-search-settings") as HTMLElement;
    const rows = this.root.querySelector("#x-search-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.xSearchSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const dflt = t.agentSettingsPanel.defaultWord;
      addRow(t.xSearchPanel.model, escapeHtmlDoctor(payload.model));
      addRow(
        t.xSearchPanel.reasoning,
        payload.reasoning_effort.trim() !== ""
          ? escapeHtmlDoctor(payload.reasoning_effort)
          : escapeHtmlDoctor(dflt),
      );
      addRow(t.xSearchPanel.timeout, `${payload.timeout_seconds}s`);
      addRow(t.xSearchPanel.retries, String(payload.retries));
      // P758: x_search editor — pick a key, type the value, Apply
      // persists through PUT /api/x-search-settings, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#x-search-editor") as HTMLElement;
      const keySel = this.root.querySelector("#x-search-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#x-search-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#x-search-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#x-search-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#x-search-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["model", "reasoning_effort", "timeout_seconds", "retries"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateXSearchSetting(keySel.value, value);
            await this.loadXSearchSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number = /^\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadKanbanSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-kanban-settings") as HTMLElement;
    const rows = this.root.querySelector("#kanban-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.kanbanSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.kanbanSettingsPanel.dispatch, payload.dispatch_in_gateway ? on : off);
      addRow(t.kanbanSettingsPanel.interval, `${payload.dispatch_interval_secs}s`);
      addRow(t.kanbanSettingsPanel.maxSpawn, String(payload.max_spawn));
      addRow(t.kanbanSettingsPanel.worktrees, payload.worktrees ? on : off);
      addRow(t.kanbanSettingsPanel.autoPromote, payload.auto_promote_children ? on : off);
      addRow(
        t.kanbanSettingsPanel.autoDecompose,
        payload.auto_decompose
          ? `${on} \u00b7 ${payload.auto_decompose_per_tick}/tick`
          : off,
      );
      addRow(t.kanbanSettingsPanel.staleTimeout, `${payload.stale_timeout_seconds}s`);
      // P757: kanban dispatcher editor — pick a key, type the value,
      // Apply persists through PUT /api/kanban-settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#kanban-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#kanban-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#kanban-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#kanban-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#kanban-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#kanban-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "dispatch_in_gateway",
          "dispatch_interval_secs",
          "max_spawn",
          "worktrees",
          "auto_promote_children",
          "auto_decompose",
          "auto_decompose_per_tick",
          "stale_timeout_seconds",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateKanbanSetting(keySel.value, value);
            await this.loadKanbanSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadVoiceSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-voice-settings") as HTMLElement;
    const rows = this.root.querySelector("#voice-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.voiceSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.voicePanel.sttEnabled, payload.stt_enabled ? on : off);
      addRow(t.voicePanel.sttEcho, payload.stt_echo_transcripts ? on : off);
      addRow(t.voicePanel.sttProvider, escapeHtmlDoctor(payload.stt_provider));
      addRow(t.voicePanel.sttLanguage, escapeHtmlDoctor(payload.stt_language));
      addRow(t.voicePanel.ttsProvider, escapeHtmlDoctor(payload.tts_provider));
      addRow(t.voicePanel.ttsEdgeVoice, escapeHtmlDoctor(payload.tts_edge_voice));
      // P756: voice pipeline editor — pick a key, type the value, Apply
      // persists through PUT /api/voice-settings, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#voice-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#voice-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#voice-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#voice-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#voice-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#voice-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "stt_enabled",
          "stt_echo_transcripts",
          "stt_provider",
          "stt_language",
          "tts_provider",
          "tts_edge_voice",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateVoiceSetting(keySel.value, value);
            await this.loadVoiceSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | boolean =
            raw === "true" ? true : raw === "false" ? false : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadCronSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-cron-settings") as HTMLElement;
    const rows = this.root.querySelector("#cron-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.cronSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.cronSettingsPanel.wrapResponse, payload.wrap_response ? on : off);
      addRow(t.cronSettingsPanel.mirrorDelivery, payload.mirror_delivery ? on : off);
      // P755: cron delivery editor — pick a key, choose true/false,
      // Apply persists through PUT /api/cron-settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#cron-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#cron-settings-key") as HTMLSelectElement;
      const valueSel = this.root.querySelector("#cron-settings-value") as HTMLSelectElement;
      const applyBtn = this.root.querySelector("#cron-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#cron-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#cron-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["wrap_response", "mirror_delivery"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateCronSetting(keySel.value, value);
            await this.loadCronSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueSel.value === "true");
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadLoggingSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-logging-settings") as HTMLElement;
    const rows = this.root.querySelector("#logging-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.loggingSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.loggingPanel.memoryMonitor, payload.memory_monitor ? on : off);
      addRow(t.loggingPanel.interval, `${payload.memory_monitor_interval_secs}s`);
      // P754: logging settings editor — pick a key, type the value,
      // Apply persists through PUT /api/logging-settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#logging-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#logging-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#logging-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#logging-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#logging-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#logging-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["memory_monitor", "memory_monitor_interval_secs"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateLoggingSetting(keySel.value, value);
            await this.loadLoggingSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadToolOutputSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-tool-output-settings") as HTMLElement;
    const rows = this.root.querySelector("#tool-output-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.toolOutputSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(t.toolOutputPanel.maxBytes, String(payload.max_bytes));
      addRow(t.toolOutputPanel.maxLines, String(payload.max_lines));
      addRow(t.toolOutputPanel.maxLineLength, String(payload.max_line_length));
      // P753: tool-output limits editor — pick a key, type the value,
      // Apply persists through PUT /api/tool-output-settings, Clear
      // removes the override; the panel re-renders from disk state.
      const editor = this.root.querySelector("#tool-output-editor") as HTMLElement;
      const keySel = this.root.querySelector("#tool-output-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#tool-output-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#tool-output-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#tool-output-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#tool-output-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["max_bytes", "max_lines", "max_line_length"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: number | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateToolOutputSetting(keySel.value, value);
            await this.loadToolOutputSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          if (!/^\d+$/.test(raw) || Number(raw) < 1) {
            statusEl.textContent = t.delegationPanel.invalidNumber;
            return;
          }
          void applyEdit(Number(raw));
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadSecuritySettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-security-settings") as HTMLElement;
    const rows = this.root.querySelector("#security-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.securitySettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const envBadge = `<span class="models-view-badge warn">${escapeHtmlDoctor(t.gatewaySettingsPanel.envOverride)}</span>`;
      addRow(t.securitySettingsPanel.privateUrls, payload.allow_private_urls ? on : off);
      addRow(
        t.securitySettingsPanel.tirithEnabled,
        payload.tirith_enabled_env_override
          ? `${payload.tirith_enabled ? on : off} \u00b7 ${envBadge}`
          : payload.tirith_enabled ? on : off,
      );
      addRow(
        t.securitySettingsPanel.tirithPath,
        payload.tirith_path_env_override
          ? `${escapeHtmlDoctor(payload.tirith_path)} \u00b7 ${envBadge}`
          : escapeHtmlDoctor(payload.tirith_path),
      );
      addRow(
        t.securitySettingsPanel.tirithTimeout,
        payload.tirith_timeout_env_override
          ? `${payload.tirith_timeout}s \u00b7 ${envBadge}`
          : `${payload.tirith_timeout}s`,
      );
      addRow(t.securitySettingsPanel.tirithFailOpen, payload.tirith_fail_open ? on : off);
      // P752: security settings editor — pick a key, type the value,
      // Apply persists through PUT /api/security-settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#security-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#security-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#security-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#security-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#security-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#security-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "allow_private_urls",
          "tirith_enabled",
          "tirith_path",
          "tirith_timeout",
          "tirith_fail_open",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateSecuritySetting(keySel.value, value);
            await this.loadSecuritySettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadCheckpointSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-checkpoint-settings") as HTMLElement;
    const rows = this.root.querySelector("#checkpoint-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.checkpointSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.checkpointSettingsPanel.enabled, payload.enabled ? on : off);
      addRow(t.checkpointSettingsPanel.maxSnapshots, String(payload.max_snapshots));
      addRow(t.checkpointSettingsPanel.totalSize, `${payload.max_total_size_mb}MB`);
      addRow(t.checkpointSettingsPanel.fileSize, `${payload.max_file_size_mb}MB`);
      addRow(t.checkpointSettingsPanel.retention, `${payload.retention_days}d`);
      addRow(t.checkpointSettingsPanel.pruneCadence, `${payload.auto_prune_hours}h`);
      // P751: checkpoint settings editor — pick a key, type the value,
      // Apply persists through PUT /api/checkpoints/settings, Clear
      // removes the override; the panel re-renders from disk state.
      const editor = this.root.querySelector("#checkpoint-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#checkpoint-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#checkpoint-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#checkpoint-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#checkpoint-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#checkpoint-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "enabled",
          "max_snapshots",
          "max_total_size_mb",
          "max_file_size_mb",
          "retention_days",
          "auto_prune_hours",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateCheckpointSetting(keySel.value, value);
            await this.loadCheckpointSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadModelCatalog(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-model-catalog") as HTMLElement;
    const rows = this.root.querySelector("#model-catalog-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.modelCatalog();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const off = t.monitoring.off;
      addRow(
        t.modelCatalogPanel.excluded,
        payload.excluded_providers.length > 0
          ? escapeHtmlDoctor(payload.excluded_providers.join(", "))
          : escapeHtmlDoctor(off),
      );
      addRow(t.modelCatalogPanel.canonical, String(payload.canonical_providers.length));
      addRow(
        t.modelCatalogPanel.custom,
        payload.custom_providers.length > 0
          ? escapeHtmlDoctor(payload.custom_providers.join(", "))
          : escapeHtmlDoctor(off),
      );
      // P750: excluded-providers editor — comma-separated slugs, Apply
      // persists through PUT /api/model-catalog, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#model-catalog-editor") as HTMLElement;
      const valueInput = this.root.querySelector("#model-catalog-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#model-catalog-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#model-catalog-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#model-catalog-status") as HTMLElement;
      if (!applyBtn.dataset.wired) {
        const applyEdit = async (value: string[] | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateModelCatalog("excluded_providers", value);
            await this.loadModelCatalog();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const slugs = valueInput.value
            .split(",")
            .map((item) => item.trim())
            .filter((item) => item.length > 0);
          void applyEdit(slugs);
        };
        clearBtn.onclick = () => void applyEdit(null);
        applyBtn.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadMemorySettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-memory-settings") as HTMLElement;
    const rows = this.root.querySelector("#memory-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.memoryStatus();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(t.memoryPanel.memoryLimit, String(payload.char_limits.memory));
      addRow(t.memoryPanel.userLimit, String(payload.char_limits.user));
      for (const file of payload.files) {
        if (!file.exists) continue;
        addRow(escapeHtmlDoctor(file.file), `${file.bytes}B \u00b7 ${file.entries}`);
      }
      // P749: memory limits editor — pick a key, type the value, Apply
      // persists through PUT /api/memory, Clear removes the override;
      // the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#memory-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#memory-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#memory-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#memory-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#memory-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#memory-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["memory_char_limit", "user_char_limit"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: number | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateMemorySetting(keySel.value, value);
            await this.loadMemorySettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          if (!/^\d+$/.test(raw) || Number(raw) < 1) {
            statusEl.textContent = t.memoryPanel.invalidNumber;
            return;
          }
          void applyEdit(Number(raw));
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadDelegationSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-delegation-settings") as HTMLElement;
    const rows = this.root.querySelector("#delegation-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.delegationSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(t.delegationPanel.children, String(payload.max_concurrent_children));
      addRow(t.delegationPanel.childIterations, String(payload.child_max_iterations));
      addRow(t.delegationPanel.maxDepth, String(payload.max_depth));
      // P748: delegation limits editor — pick a key, type the value,
      // Apply persists through PUT /api/delegation-settings, Clear
      // removes the override; the panel re-renders from disk state.
      const editor = this.root.querySelector("#delegation-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#delegation-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#delegation-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#delegation-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#delegation-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#delegation-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["max_concurrent_children", "child_max_iterations", "max_depth"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: number | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateDelegationSetting(keySel.value, value);
            await this.loadDelegationSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          if (!/^\d+$/.test(raw) || Number(raw) < 1) {
            statusEl.textContent = t.delegationPanel.invalidNumber;
            return;
          }
          void applyEdit(Number(raw));
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadWebSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-web-settings") as HTMLElement;
    const rows = this.root.querySelector("#web-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.webSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.webSettingsPanel.searchBackend,
        payload.search_backend !== null
          ? escapeHtmlDoctor(payload.search_backend)
          : escapeHtmlDoctor(t.webSettingsPanel.autoWord),
      );
      addRow(
        t.webSettingsPanel.extractBackend,
        payload.extract_backend !== null
          ? escapeHtmlDoctor(payload.extract_backend)
          : escapeHtmlDoctor(t.webSettingsPanel.autoWord),
      );
      addRow(t.webSettingsPanel.tavilyKey, payload.tavily_key_configured ? on : off);
      addRow(t.webSettingsPanel.braveKey, payload.brave_key_configured ? on : off);
      addRow(t.webSettingsPanel.searxngUrl, payload.searxng_url_configured ? on : off);
      // P747: web-tool backend editor — pick a key, type the value,
      // Apply persists through PUT /api/web-settings, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#web-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#web-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#web-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#web-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#web-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#web-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = ["search_backend", "extract_backend"];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateWebSetting(keySel.value, value);
            await this.loadWebSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => void applyEdit(valueInput.value.trim());
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadAgentSettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-agent-settings") as HTMLElement;
    const rows = this.root.querySelector("#agent-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.agentSettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const dflt = t.agentSettingsPanel.defaultWord;
      addRow(t.agentSettingsPanel.iterations, String(payload.max_iterations));
      addRow(t.agentSettingsPanel.approvalGate, payload.approval ? on : off);
      addRow(
        t.agentSettingsPanel.concurrentTools,
        payload.concurrent_tool_execution
          ? `${on} \u00b7 ${payload.max_concurrent_tools}`
          : off,
      );
      addRow(t.agentSettingsPanel.contextBudget, `${payload.context_budget_tokens}`);
      addRow(t.agentSettingsPanel.verbose, payload.verbose ? on : off);
      addRow(t.agentSettingsPanel.envProbe, payload.environment_probe ? on : off);
      addRow(
        t.agentSettingsPanel.reasoning,
        payload.reasoning_effort.trim() !== "" ? escapeHtmlDoctor(payload.reasoning_effort) : escapeHtmlDoctor(dflt),
      );
      addRow(
        t.agentSettingsPanel.serviceTier,
        payload.service_tier.trim() !== "" ? escapeHtmlDoctor(payload.service_tier) : escapeHtmlDoctor(dflt),
      );
      addRow(
        t.agentSettingsPanel.personality,
        payload.personality.trim() !== "" ? escapeHtmlDoctor(payload.personality) : escapeHtmlDoctor(dflt),
      );
      // P746: agent behavior editor — pick a key, type the value, Apply
      // persists through PUT /api/agent-settings, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#agent-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#agent-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#agent-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#agent-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#agent-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#agent-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "max_iterations",
          "approval",
          "concurrent_tool_execution",
          "max_concurrent_tools",
          "context_budget_tokens",
          "verbose",
          "environment_probe",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateAgentSetting(keySel.value, value);
            await this.loadAgentSettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^-?\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadGatewaySettings(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-gateway-settings") as HTMLElement;
    const rows = this.root.querySelector("#gateway-settings-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.gatewaySettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const envBadge = `<span class="models-view-badge warn">${escapeHtmlDoctor(t.gatewaySettingsPanel.envOverride)}</span>`;
      const listener = `${escapeHtmlDoctor(payload.host)}:${payload.port}`;
      addRow(
        t.gatewaySettingsPanel.address,
        payload.host_env_override || payload.port_env_override
          ? `${listener} \u00b7 ${envBadge}`
          : listener,
      );
      addRow(
        t.gatewaySettingsPanel.apiKey,
        payload.key_configured
          ? payload.key_env_override
            ? `${on} \u00b7 ${envBadge}`
            : on
          : off,
      );
      addRow(t.gatewaySettingsPanel.multiplex, payload.multiplex_profiles ? on : off);
      addRow(t.gatewaySettingsPanel.profileRoutes, String(payload.profile_routes));
      addRow(t.gatewaySettingsPanel.messageTimestamps, payload.message_timestamps ? on : off);
      addRow(t.gatewaySettingsPanel.loopWatchdog, payload.loop_watchdog ? on : off);
      addRow(
        t.gatewaySettingsPanel.systemdWatchdog,
        payload.systemd_watchdog_seconds > 0 ? `${payload.systemd_watchdog_seconds}s` : off,
      );
      addRow(
        t.gatewaySettingsPanel.sessionCap,
        payload.max_concurrent_sessions !== null && payload.max_concurrent_sessions > 0
          ? String(payload.max_concurrent_sessions)
          : off,
      );
      addRow(
        t.gatewaySettingsPanel.stallTimeout,
        payload.session_stall_env_override
          ? `${payload.session_stall_timeout_secs}s \u00b7 ${envBadge}`
          : `${payload.session_stall_timeout_secs}s`,
      );
      // P745: gateway behavior editor — pick a key, type the value,
      // Apply persists through PUT /api/gateway-settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#gateway-settings-editor") as HTMLElement;
      const keySel = this.root.querySelector("#gateway-settings-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#gateway-settings-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#gateway-settings-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#gateway-settings-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#gateway-settings-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "multiplex_profiles",
          "message_timestamps",
          "loop_watchdog",
          "systemd_watchdog_seconds",
          "max_concurrent_sessions",
          "session_stall_timeout_secs",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateGatewaySetting(keySel.value, value);
            await this.loadGatewaySettings();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^-?\d+(\.\d+)?$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadApprovals(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-approvals") as HTMLElement;
    const rows = this.root.querySelector("#approvals-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.approvalsGet();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.approvalsPanel.mode,
        `<span class="models-view-badge ok">${escapeHtmlDoctor(payload.mode)}</span>`,
      );
      addRow(t.approvalsPanel.timeout, `${payload.timeout}s`);
      addRow(t.approvalsPanel.cronMode, escapeHtmlDoctor(payload.cron_mode));
      addRow(
        t.approvalsPanel.smartPolicy,
        payload.smart_policy.trim().length > 0
          ? escapeHtmlDoctor(payload.smart_policy)
          : escapeHtmlDoctor(off),
      );
      addRow(t.approvalsPanel.breaker, String(payload.denial_breaker_threshold));
      addRow(
        t.approvalsPanel.denyRules,
        payload.deny.length > 0
          ? escapeHtmlDoctor(payload.deny.join(", "))
          : escapeHtmlDoctor(off),
      );
      addRow(
        t.approvalsPanel.mcpReloadConfirm,
        payload.mcp_reload_confirm ? on : off,
      );
      // P744: approvals settings editor — pick a key, type the value,
      // Apply persists through PUT /api/approvals/settings, Clear removes
      // the override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#approvals-editor") as HTMLElement;
      const keySel = this.root.querySelector("#approvals-edit-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#approvals-edit-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#approvals-edit-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#approvals-edit-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#approvals-edit-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "timeout",
          "cron_mode",
          "smart_policy",
          "denial_breaker_threshold",
          "deny",
          "mcp_reload_confirm",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (
          value: string | number | boolean | string[] | null,
        ): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.approvalsSettingsSet(keySel.value, value);
            await this.loadApprovals();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          let parsed: string | number | boolean | string[];
          if (keySel.value === "deny") {
            parsed = raw
              .split(",")
              .map((item) => item.trim())
              .filter((item) => item.length > 0);
          } else if (raw === "true") {
            parsed = true;
          } else if (raw === "false") {
            parsed = false;
          } else if (/^-?\d+$/.test(raw)) {
            parsed = Number(raw);
          } else {
            parsed = raw;
          }
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadPhrases(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-phrases") as HTMLElement;
    const rows = this.root.querySelector("#phrases-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.statusPhrases();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.phrasesPanel.catalog,
        `${payload.status_count} ${t.phrasesPanel.statusWord} \u00b7 ${payload.generic_count} ${t.phrasesPanel.genericWord}`,
      );
      addRow(
        t.phrasesPanel.sample,
        escapeHtmlDoctor(payload.status_sample.slice(0, 3).join(", ")),
      );
      addRow(
        t.phrasesPanel.conventional,
        payload.conventional_files.length > 0
          ? escapeHtmlDoctor(payload.conventional_files.map((f) => f.path).join(", "))
          : escapeHtmlDoctor(off),
      );
      addRow(t.phrasesPanel.configSection, payload.has_config_section ? on : off);
      for (const [platform, catalog] of Object.entries(payload.platforms)) {
        addRow(
          platform,
          `${catalog.status_count} ${t.phrasesPanel.statusWord} \u00b7 ${catalog.generic_count} ${t.phrasesPanel.genericWord}`,
        );
      }
      // P743: live preview — sample the rotation pool the status line
      // would actually emit, per kind.
      const statusBtn = this.root.querySelector("#phrases-preview-status") as HTMLButtonElement;
      const genericBtn = this.root.querySelector("#phrases-preview-generic") as HTMLButtonElement;
      const previewOut = this.root.querySelector("#phrases-preview-out") as HTMLElement;
      if (!statusBtn.dataset.wired) {
        const runPreview = async (kind: "status" | "generic"): Promise<void> => {
          previewOut.textContent = "";
          try {
            const preview = await client.statusPhrasesPreview({ kind, count: 8 });
            previewOut.innerHTML =
              preview.phrases.length > 0
                ? `${escapeHtmlDoctor(preview.phrases.join(" \u00b7 "))} <span class="models-view-badge ok">${preview.total}</span>`
                : escapeHtmlDoctor(t.phrasesPanel.previewEmpty);
          } catch (err) {
            previewOut.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        statusBtn.onclick = () => void runPreview("status");
        genericBtn.onclick = () => void runPreview("generic");
        statusBtn.dataset.wired = "1";
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadPortal(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-portal") as HTMLElement;
    const rows = this.root.querySelector("#portal-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.portalStatus();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const status = payload.logged_in
        ? payload.expired
          ? `<span class="models-view-badge warn">${escapeHtmlDoctor(t.portalPanel.expired)}</span>`
          : `<span class="models-view-badge ok">${escapeHtmlDoctor(t.portalPanel.loggedIn)}</span>`
        : escapeHtmlDoctor(off);
      addRow(t.portalPanel.status, status);
      if (payload.logged_in) {
        addRow(
          t.portalPanel.expires,
          payload.expires_at > 0
            ? escapeHtmlDoctor(new Date(payload.expires_at * 1000).toLocaleString())
            : escapeHtmlDoctor(t.portalPanel.unknown),
        );
        addRow(t.portalPanel.scope, payload.scope ? escapeHtmlDoctor(payload.scope) : off);
        addRow(t.portalPanel.refreshToken, payload.refresh_token_stored ? on : off);
      }
      if (payload.portal_url) {
        addRow(t.portalPanel.portalUrl, escapeHtmlDoctor(payload.portal_url));
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadHooks(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-hooks") as HTMLElement;
    const rows = this.root.querySelector("#hooks-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.hooksInfo();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      addRow(
        t.hooksPanel.count,
        payload.count > 0
          ? String(payload.count)
          : t.monitoring.off,
      );
      for (const hook of payload.hooks) {
        const desc = hook.description
          ? ` \u00b7 ${escapeHtmlDoctor(hook.description)}`
          : "";
        addRow(
          hook.name,
          `${escapeHtmlDoctor(hook.events.join(", "))}${desc}`,
        );
      }
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadCgroup(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-cgroup") as HTMLElement;
    const rows = this.root.querySelector("#cgroup-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.cgroupInfo();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(t.cgroupPanel.supported, payload.supported ? on : off);
      addRow(t.cgroupPanel.path, payload.cgroup_path ? escapeHtmlDoctor(payload.cgroup_path) : off);
      const own = payload.contains_own_pid
        ? ` (${escapeHtmlDoctor(t.cgroupPanel.ownPid)})`
        : "";
      addRow(t.cgroupPanel.pidCount, `${payload.pid_count}${own}`);
      addRow(t.cgroupPanel.reapOnExit, payload.reap_on_exit ? on : off);
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadTerminal(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-terminal") as HTMLElement;
    const rows = this.root.querySelector("#terminal-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.terminalInfo();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const backend = payload.backend_env_override
        ? `${escapeHtmlDoctor(payload.backend)} <span class="models-view-badge warn">${escapeHtmlDoctor(t.terminalPanel.envOverride)}</span>`
        : escapeHtmlDoctor(payload.backend);
      addRow(t.terminalPanel.backend, backend);
      const cwdBadge = payload.configured_is_placeholder
        ? ` <span class="models-view-badge warn">${escapeHtmlDoctor(t.terminalPanel.placeholder)}</span>`
        : "";
      addRow(
        t.terminalPanel.configuredCwd,
        escapeHtmlDoctor(payload.configured_cwd ?? t.terminalPanel.notSet) + cwdBadge,
      );
      addRow(
        t.terminalPanel.resolvedCwd,
        escapeHtmlDoctor(payload.resolved_messaging_cwd ?? off),
      );
      if (payload.backend === "docker") {
        addRow(
          t.terminalPanel.mountWorkspace,
          payload.docker_mount_cwd_to_workspace ? on : off,
        );
        if (payload.container) addRow(t.terminalPanel.container, escapeHtmlDoctor(payload.container));
        if (payload.image) addRow(t.terminalPanel.image, escapeHtmlDoctor(payload.image));
      }
      if (payload.backend === "ssh" && payload.ssh_host) {
        const port = payload.ssh_port ? `:${payload.ssh_port}` : "";
        addRow(
          t.terminalPanel.sshHost,
          escapeHtmlDoctor(`${payload.ssh_user ?? ""}@${payload.ssh_host}${port}`),
        );
      }
      addRow(
        t.terminalPanel.timeouts,
        `${payload.timeout_secs}s \u00b7 ${t.terminalPanel.foregroundMax} ${payload.foreground_max_timeout_secs}s`,
      );
      addRow(
        t.terminalPanel.envPassthrough,
        payload.env_passthrough_count > 0 ? String(payload.env_passthrough_count) : off,
      );
      addRow(t.terminalPanel.sessionCwd, escapeHtmlDoctor(payload.session_cwd));
      // P739: terminal settings editor — pick a key, type the value,
      // Apply persists through PUT /api/terminal, Clear removes the
      // override; the panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#terminal-editor") as HTMLElement;
      const keySel = this.root.querySelector("#terminal-edit-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#terminal-edit-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#terminal-edit-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#terminal-edit-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#terminal-edit-status") as HTMLElement;
      if (!keySel.dataset.wired) {
        const keys = [
          "backend",
          "cwd",
          "container",
          "image",
          "ssh_host",
          "ssh_user",
          "ssh_port",
          "ssh_identity",
          "timeout",
          "foreground_max_timeout",
          "docker_mount_cwd_to_workspace",
        ];
        keySel.innerHTML = "";
        for (const key of keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          statusEl.textContent = "";
          try {
            await client.updateTerminal(keySel.value, value);
            await this.loadTerminal();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^-?\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        keySel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadDisplay(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-display") as HTMLElement;
    const rows = this.root.querySelector("#display-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.displaySettings();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const enabled = payload.platforms.filter((p) => p.enabled);
      const shown = enabled.length > 0 ? enabled : payload.platforms.slice(0, 6);
      if (enabled.length === 0) {
        addRow(t.displayPanel.noneEnabled, "");
      }
      const flag = (value: string | number | boolean | null): string =>
        value === true ? t.monitoring.on : value === false ? t.monitoring.off : "?";
      for (const platform of shown) {
        const s = platform.settings;
        const summary = [
          `${t.displayPanel.progress}=${s.tool_progress ?? "?"}`,
          `${t.displayPanel.preview}=${s.tool_preview_length ?? 0}`,
          `${t.displayPanel.heartbeats}=${flag(s.long_running_notifications)}`,
          `${t.displayPanel.busyDetail}=${flag(s.busy_ack_detail)}`,
          `${t.displayPanel.live}=${s.live_status ?? "?"}`,
        ].join(" \u00b7 ");
        const custom = platform.has_overrides
          ? `<span class="models-view-badge ok">${escapeHtmlDoctor(t.displayPanel.custom)}</span> \u00b7 `
          : "";
        addRow(platform.platform, `${custom}${escapeHtmlDoctor(summary)}`);
      }
      // P738: override editor — pick platform + key, type a value,
      // Apply persists through PUT /api/display, Clear removes the
      // override. The panel re-renders from disk state afterwards.
      const editor = this.root.querySelector("#display-editor") as HTMLElement;
      const platformSel = this.root.querySelector("#display-edit-platform") as HTMLSelectElement;
      const keySel = this.root.querySelector("#display-edit-key") as HTMLSelectElement;
      const valueInput = this.root.querySelector("#display-edit-value") as HTMLInputElement;
      const applyBtn = this.root.querySelector("#display-edit-apply") as HTMLButtonElement;
      const clearBtn = this.root.querySelector("#display-edit-clear") as HTMLButtonElement;
      const statusEl = this.root.querySelector("#display-edit-status") as HTMLElement;
      if (!platformSel.dataset.wired) {
        const platforms = [t.displayPanel.global, ...payload.platforms.map((p) => p.platform)];
        platformSel.innerHTML = "";
        for (const name of platforms) {
          const option = document.createElement("option");
          option.value = name;
          option.textContent = name;
          platformSel.appendChild(option);
        }
        keySel.innerHTML = "";
        for (const key of payload.overrideable_keys) {
          const option = document.createElement("option");
          option.value = key;
          option.textContent = key;
          keySel.appendChild(option);
        }
        const applyEdit = async (value: string | number | boolean | null): Promise<void> => {
          const platform = platformSel.value === t.displayPanel.global ? null : platformSel.value;
          statusEl.textContent = "";
          try {
            await client.updateDisplay(platform, keySel.value, value);
            await this.loadDisplay();
            this.flashSaved(statusEl);
          } catch (err) {
            statusEl.textContent = err instanceof Error ? err.message : String(err);
          }
        };
        applyBtn.onclick = () => {
          const raw = valueInput.value.trim();
          const parsed: string | number | boolean =
            raw === "true" ? true : raw === "false" ? false : /^-?\d+$/.test(raw) ? Number(raw) : raw;
          void applyEdit(parsed);
        };
        clearBtn.onclick = () => void applyEdit(null);
        platformSel.dataset.wired = "1";
      }
      editor.hidden = false;
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadLifecycle(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-lifecycle") as HTMLElement;
    const rows = this.root.querySelector("#lifecycle-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const payload = await client.lifecycle();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const label = payload.previous_exit_label;
      const exitBadge =
        label === "clean"
          ? `<span class="models-view-badge ok">${escapeHtmlDoctor(t.lifecyclePanel.clean)}</span>`
          : label === "unclean"
            ? `<span class="models-view-badge warn">${escapeHtmlDoctor(t.lifecyclePanel.unclean)}</span>`
            : escapeHtmlDoctor(t.lifecyclePanel.unknown);
      addRow(t.lifecyclePanel.previousExit, exitBadge);
      if (payload.sentinel) {
        const reason = payload.sentinel.exit_reason ?? "";
        const code =
          payload.sentinel.exit_code === null || payload.sentinel.exit_code === undefined
            ? ""
            : ` \u00b7 ${payload.sentinel.exit_code}`;
        if (reason || code) {
          addRow(
            t.lifecyclePanel.exitReason,
            `${escapeHtmlDoctor(reason)}${escapeHtmlDoctor(code)}`,
          );
        }
      }
      if (payload.heartbeat.present) {
        const age =
          payload.heartbeat.age_seconds === null || payload.heartbeat.age_seconds === undefined
            ? "?"
            : `${Math.round(payload.heartbeat.age_seconds)}s`;
        const pid = payload.heartbeat.payload?.pid;
        addRow(
          t.lifecyclePanel.heartbeat,
          `${escapeHtmlDoctor(age)}${pid !== undefined ? ` \u00b7 pid ${pid}` : ""}`,
        );
      } else {
        addRow(t.lifecyclePanel.heartbeat, escapeHtmlDoctor(t.lifecyclePanel.heartbeatMissing));
      }
      const artifact = (meta: { present: boolean; bytes?: number }): string =>
        meta.present
          ? `${escapeHtmlDoctor(t.lifecyclePanel.present)} \u00b7 ${meta.bytes ?? 0} B`
          : escapeHtmlDoctor(t.lifecyclePanel.absent);
      addRow(t.lifecyclePanel.watchdogDump, artifact(payload.shutdown_watchdog_dump));
      addRow(t.lifecyclePanel.diagnosticLog, artifact(payload.shutdown_diagnostic_log));
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  private async loadDrain(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-drain") as HTMLElement;
    const rows = this.root.querySelector("#drain-rows") as HTMLElement;
    const beginBtn = this.root.querySelector("#drain-begin") as HTMLButtonElement;
    const cancelBtn = this.root.querySelector("#drain-cancel") as HTMLButtonElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    const render = async (): Promise<void> => {
      const payload = await client.drainStatus();
      rows.innerHTML = "";
      const addRow = (label: string, valueHtml: string): void => {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.innerHTML = valueHtml;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      };
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      addRow(
        t.drainPanel.drainRequested,
        payload.requested
          ? `<span class="models-view-badge warn">${escapeHtmlDoctor(on)}</span>`
          : escapeHtmlDoctor(off),
      );
      addRow(
        t.drainPanel.draining,
        payload.draining
          ? `<span class="models-view-badge warn">${escapeHtmlDoctor(on)}</span>`
          : escapeHtmlDoctor(off),
      );
      if (payload.requested) {
        addRow(t.drainPanel.principal, escapeHtmlDoctor(payload.marker?.principal ?? "?"));
        addRow(
          t.drainPanel.requestedAt,
          escapeHtmlDoctor(payload.marker?.requested_at ?? "?"),
        );
        addRow(t.drainPanel.suppress, payload.suppress_notification ? on : off);
      }
      if (payload.stale) {
        addRow(
          t.drainPanel.epoch,
          `<span class="models-view-badge warn">${escapeHtmlDoctor(t.drainPanel.stale)}</span>`,
        );
      }
      beginBtn.disabled = payload.requested;
      cancelBtn.disabled = !payload.requested;
      section.hidden = false;
    };
    beginBtn.onclick = () => {
      client
        .beginDrain("desktop-doctor", false)
        .then(() => render())
        .catch(() => undefined);
    };
    cancelBtn.onclick = () => {
      client
        .cancelDrain()
        .then(() => render())
        .catch(() => undefined);
    };
    try {
      await render();
    } catch {
      section.hidden = true;
    }
  }

  private async loadBrowser(): Promise<void> {
    const client = this.client();
    const section = this.root.querySelector("#doctor-browser") as HTMLElement;
    const rows = this.root.querySelector("#browser-rows") as HTMLElement;
    if (!client) {
      section.hidden = true;
      return;
    }
    try {
      const status = await client.browserStatus();
      rows.innerHTML = "";
      const on = t.monitoring.on;
      const off = t.monitoring.off;
      const entries: [string, string][] = [
        [t.browserPanel.configured, status.configured ? on : off],
      ];
      if (status.configured) {
        if (status.backend) entries.push([t.browserPanel.backend, status.backend]);
        if (status.mode) entries.push([t.browserPanel.mode, status.mode]);
        if (status.source) entries.push([t.browserPanel.source, status.source]);
        if (status.endpoint) entries.push([t.browserPanel.endpoint, status.endpoint]);
        if (status.backend === "camofox") {
          entries.push([t.browserPanel.available, status.available ? on : off]);
          if (status.vnc_url) entries.push([t.browserPanel.vnc, status.vnc_url]);
        } else if (status.mode === "managed") {
          entries.push([t.browserPanel.managedRunning, status.managed_running ? on : off]);
        }
      }
      for (const [label, value] of entries) {
        const row = document.createElement("div");
        row.className = "monitoring-row";
        const labelEl = document.createElement("span");
        labelEl.className = "monitoring-label";
        labelEl.textContent = label;
        const valueEl = document.createElement("span");
        valueEl.className = "monitoring-value";
        valueEl.textContent = value;
        row.append(labelEl, valueEl);
        rows.appendChild(row);
      }
      rows.appendChild(this.browserControls(status));
      section.hidden = false;
    } catch {
      section.hidden = true;
    }
  }

  /** P352: live CDP override controls (hermes `/browser connect|disconnect`). */
  private browserControls(status: BrowserStatus): HTMLElement {
    const controls = document.createElement("div");
    controls.className = "monitoring-row";
    const input = document.createElement("input");
    input.type = "text";
    input.className = "config-input";
    input.placeholder = t.browserPanel.urlPlaceholder;
    const connect = document.createElement("button");
    connect.className = "ghost";
    connect.textContent = t.browserPanel.connect;
    const disconnect = document.createElement("button");
    disconnect.className = "ghost danger";
    disconnect.textContent = t.browserPanel.disconnect;
    disconnect.disabled = status.source !== "override";
    const note = document.createElement("span");
    note.className = "config-status";
    note.hidden = true;
    connect.addEventListener("click", () => {
      const client = this.client();
      const url = input.value.trim();
      if (!client || !url) return;
      connect.disabled = true;
      connect.textContent = t.browserPanel.connecting;
      client
        .browserConnect(url)
        .then((endpoint) => {
          note.textContent = t.browserPanel.connected.replace("{endpoint}", endpoint);
          note.classList.remove("error");
          note.hidden = false;
          return this.loadBrowser();
        })
        .catch((error) => {
          note.textContent = t.browserPanel.connectFailed.replace(
            "{error}",
            error instanceof Error ? error.message : String(error),
          );
          note.classList.add("error");
          note.hidden = false;
          connect.disabled = false;
          connect.textContent = t.browserPanel.connect;
        });
    });
    disconnect.addEventListener("click", () => {
      const client = this.client();
      if (!client) return;
      client
        .browserDisconnect()
        .then(() => {
          note.textContent = t.browserPanel.disconnected;
          note.classList.remove("error");
          note.hidden = false;
          return this.loadBrowser();
        })
        .catch((error) => {
          note.textContent = error instanceof Error ? error.message : String(error);
          note.classList.add("error");
          note.hidden = false;
        });
    });
    controls.append(input, connect, disconnect, note);
    return controls;
  }

  private render(sections: { title: string; checks: DoctorCheck[] }[], issues: string[]): void {
    const body = this.root.querySelector("#doctor-body") as HTMLElement;
    body.innerHTML = "";

    const issuesBox = document.createElement("div");
    issuesBox.className = issues.length > 0 ? "doctor-issues" : "doctor-clean";
    if (issues.length > 0) {
      const title = document.createElement("h3");
      title.textContent = `${t.doctor.issues} (${issues.length})`;
      issuesBox.appendChild(title);
      const list = document.createElement("ul");
      for (const issue of issues) {
        const item = document.createElement("li");
        item.textContent = issue;
        list.appendChild(item);
      }
      issuesBox.appendChild(list);
    } else {
      issuesBox.textContent = t.doctor.noIssues;
    }
    body.appendChild(issuesBox);

    if (sections.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.doctor.empty;
      body.appendChild(empty);
      return;
    }
    // P527: severity filter — "warn" keeps warnings + failures, "fail"
    // keeps failures only; sections left empty are hidden entirely.
    const keep = (level: DoctorCheck["level"]): boolean =>
      this.levelFilter === "all" ||
      (this.levelFilter === "warn"
        ? level === "warn" || level === "fail"
        : level === "fail");
    const filtered = sections
      .map((section) => ({
        title: section.title,
        checks: section.checks.filter((check) => keep(check.level)),
      }))
      .filter((section) => section.checks.length > 0);
    if (filtered.length === 0) {
      const empty = document.createElement("p");
      empty.className = "config-note";
      empty.textContent = t.doctor.filterNoMatch;
      body.appendChild(empty);
      return;
    }
    for (const section of filtered) {
      const header = document.createElement("h3");
      header.className = "config-section";
      header.textContent = section.title;
      body.appendChild(header);
      for (const check of section.checks) {
        const row = document.createElement("div");
        row.className = `doctor-row doctor-${check.level}`;
        const icon = document.createElement("span");
        icon.className = "doctor-icon";
        icon.textContent = LEVEL_ICON[check.level] ?? "?";
        const text = document.createElement("span");
        text.className = "doctor-text";
        text.textContent = check.text;
        row.appendChild(icon);
        row.appendChild(text);
        if (check.detail) {
          const detail = document.createElement("span");
          detail.className = "doctor-detail";
          detail.textContent = check.detail;
          row.appendChild(detail);
        }
        body.appendChild(row);
      }
    }
  }
}
