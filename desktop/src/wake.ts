// Wake indicator (P537) — hermes apps/desktop wake-indicator parity:
// when the tab returns from a long hidden stretch (system sleep, lid
// close) or the network comes back, probe the gateway and reconcile
// state. Outages surface through the notification stack and retry in
// the background until the gateway answers again.

type WakeProbe = () => Promise<boolean>;

export interface WakeHooks {
  /** Gate: only reconcile once the initial boot has succeeded. */
  armed(): boolean;
  /** Gateway answered; `afterOutage` marks recovery from a downtime. */
  onHealthy(afterOutage: boolean): void;
  /** Gateway stopped answering (fires once per outage). */
  onUnreachable(): void;
}

/** Hidden stretches shorter than this are ordinary tab switches. */
const HIDDEN_WAKE_THRESHOLD_MS = 30_000;
/** Background retry cadence while the gateway is unreachable. */
const RETRY_INTERVAL_MS = 5_000;

export function initWakeIndicator(probe: WakeProbe, hooks: WakeHooks): void {
  let hiddenAt: number | null = null;
  let outage = false;
  let retryTimer: number | null = null;

  const reconcile = async (): Promise<void> => {
    if (!hooks.armed()) return;
    const healthy = await probe();
    if (healthy) {
      const wasOutage = outage;
      outage = false;
      if (retryTimer !== null) {
        window.clearInterval(retryTimer);
        retryTimer = null;
      }
      hooks.onHealthy(wasOutage);
    } else if (!outage) {
      outage = true;
      hooks.onUnreachable();
      if (retryTimer === null) {
        retryTimer = window.setInterval(() => {
          void reconcile();
        }, RETRY_INTERVAL_MS);
      }
    }
  };

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      hiddenAt = Date.now();
      return;
    }
    const away = hiddenAt === null ? 0 : Date.now() - hiddenAt;
    hiddenAt = null;
    if (away >= HIDDEN_WAKE_THRESHOLD_MS || outage) {
      void reconcile();
    }
  });

  window.addEventListener("online", () => {
    void reconcile();
  });
}
