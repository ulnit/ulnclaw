// Activity timer (P256) — port of hermes apps/desktop
// `chat/activity-timer.ts`: elapsed-seconds formatting (`Xs` under a
// minute, `M:SS` after) plus a small ticking helper for the streaming
// progress strip and measured tool-card durations. The React hook /
// remount-registry machinery doesn't apply to the dependency-free
// shell; the semantics (one-second ticks, measured durations remembered
// per call) are preserved.

export function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export class ActivityTimer {
  private startedAt = 0;
  private interval: number | null = null;

  start(onTick: (elapsedSeconds: number) => void): void {
    this.stop();
    this.startedAt = Date.now();
    onTick(0);
    this.interval = window.setInterval(() => {
      onTick(Math.max(0, Math.floor((Date.now() - this.startedAt) / 1000)));
    }, 1000);
  }

  stop(): void {
    if (this.interval !== null) {
      window.clearInterval(this.interval);
      this.interval = null;
    }
  }

  elapsedSeconds(): number {
    return this.startedAt === 0 ? 0 : Math.max(0, Math.floor((Date.now() - this.startedAt) / 1000));
  }
}
