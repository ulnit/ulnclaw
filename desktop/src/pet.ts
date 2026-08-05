// Petdex mascot overlay — the desktop twin of the terminal pet renderer:
// a small animated spritesheet canvas pinned to the bottom-right corner.
// Config (display.pet.*) and the sheet come from the gateway's
// /api/pets/* endpoints; the pet "works" (running row) while /v1/runs
// has live runs, idles otherwise, and waves when clicked.

import type { GatewayClient, PetConfig } from "./gateway";

const FRAME_MS = 120; // animation tick (terminal renderer runs ~8 fps too)
const DISPLAY_SCALE = 0.5; // atlas cells are 192x208; show at half size

export class PetOverlay {
  private canvas: HTMLCanvasElement;
  private sheet: HTMLImageElement | null = null;
  private sheetSlug = "";
  private config: PetConfig | null = null;
  private state = "idle";
  private oneShot: string | null = null; // play once, then back (wave on click)
  private frame = 0;
  private lastTick = 0;
  private raf = 0;
  private pollTimer: number | null = null;

  constructor(private client: () => GatewayClient | null) {
    this.canvas = document.createElement("canvas");
    this.canvas.id = "pet-overlay";
    this.canvas.title = "petdex mascot";
    this.canvas.onclick = () => {
      if (this.sheet && this.state !== "waving") {
        this.oneShot = "waving";
        this.frame = 0;
      }
    };
    document.body.appendChild(this.canvas);
  }

  start(): void {
    void this.poll();
    if (this.pollTimer === null) {
      this.pollTimer = window.setInterval(() => void this.poll(), 5000);
    }
    if (!this.raf) {
      const loop = (now: number) => {
        this.tick(now);
        this.raf = requestAnimationFrame(loop);
      };
      this.raf = requestAnimationFrame(loop);
    }
  }

  stop(): void {
    if (this.pollTimer !== null) {
      window.clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
    if (this.raf) {
      cancelAnimationFrame(this.raf);
      this.raf = 0;
    }
    this.canvas.style.display = "none";
  }

  private async poll(): Promise<void> {
    const client = this.client();
    if (!client) return;
    const config = await client.petConfig();
    if (!config || !config.enabled || !config.slug) {
      this.config = null;
      this.canvas.style.display = "none";
      return;
    }
    this.config = config;
    if (config.slug !== this.sheetSlug) {
      await this.loadSheet(config.slug);
    }
    const activeRuns = await client.activeRunCount();
    if (!this.oneShot) {
      this.state = activeRuns > 0 ? "running" : "idle";
    }
    this.canvas.style.display = "";
  }

  private loadSheet(slug: string): Promise<void> {
    const client = this.client();
    if (!client) return Promise.resolve();
    return new Promise((resolve) => {
      const image = new Image();
      image.onload = () => {
        this.sheet = image;
        this.sheetSlug = slug;
        this.frame = 0;
        resolve();
      };
      image.onerror = () => resolve();
      image.src = client.spritesheetUrl(slug);
    });
  }

  private tick(now: number): void {
    if (!this.sheet || !this.config || this.canvas.style.display === "none") {
      return;
    }
    if (now - this.lastTick < FRAME_MS) return;
    this.lastTick = now;

    const atlas = this.config.atlas;
    const stateName = this.oneShot || this.state;
    const row = atlas.rows.find((r) => r.state === stateName) || atlas.rows[0];
    this.frame += 1;
    if (this.frame >= row.frames) {
      this.frame = 0;
      if (this.oneShot) this.oneShot = null;
    }

    const scale = (this.config.scale ?? 1) * DISPLAY_SCALE;
    const width = Math.max(1, Math.round(atlas.frame_w * scale));
    const height = Math.max(1, Math.round(atlas.frame_h * scale));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    const ctx = this.canvas.getContext("2d");
    if (!ctx) return;
    ctx.imageSmoothingEnabled = false; // keep the pixel-art crisp
    ctx.clearRect(0, 0, width, height);
    ctx.drawImage(
      this.sheet,
      this.frame * atlas.frame_w,
      row.row * atlas.frame_h,
      atlas.frame_w,
      atlas.frame_h,
      0,
      0,
      width,
      height,
    );
  }
}
