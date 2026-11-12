// Attrappe des Players: reiht Häppchen ein und meldet Ende nur auf `finish()` bzw. `finishPiece()`.
export const CAPTURE_RATE = 16000;
export const FRAME_SAMPLES = 320;
export const floatToPcm16Base64 = (): string => "";
export const rms = (): number => 0;
export const startCapture = async (): Promise<{ stop: () => void }> => ({
  stop: () => undefined,
});

export class SpeechPlayer {
  onFirstSound?: (at: number) => void;
  onIdle?: () => void;
  enqueued: string[] = [];
  stopped = 0;
  pauses = 0;
  resumes = 0;
  volume = 1;
  rate = 1;
  setVolume(v: number): void {
    this.volume = v;
  }
  setRate(r: number): void {
    this.rate = r;
  }
  private firstFired = false;
  private playing = false;
  private held = false;
  private queue: {
    onEnd?: () => void;
    onStart?: () => void;
    started: boolean;
  }[] = [];
  /** Test: Häppchen sind eingeplant, aber noch nicht zu hören, bis `audioStart()` kommt. */
  deferStart = false;
  get active(): boolean {
    return this.playing;
  }
  get paused(): boolean {
    return this.held;
  }
  async enqueue(
    wav: string,
    onEnd?: () => void,
    onStart?: () => void,
  ): Promise<void> {
    this.enqueued.push(wav);
    this.queue.push({ onEnd, onStart, started: false });
    this.playing = true;
    if (!this.firstFired && !this.held) {
      this.firstFired = true;
      this.onFirstSound?.(performance.now());
    }
    this.startHead();
  }
  /** Das vorderste Häppchen beginnt zu spielen (außer angehalten oder `deferStart`). */
  private startHead(): void {
    const head = this.queue[0];
    if (!head || head.started || this.held || this.deferStart) return;
    head.started = true;
    head.onStart?.();
  }
  /** Test: der Ton des vordersten Häppchens beginnt tatsächlich. */
  audioStart(): void {
    const head = this.queue[0];
    if (!head || head.started || this.held) return;
    head.started = true;
    head.onStart?.();
  }
  syncStarts(): void {
    /* Die Attrappe meldet Beginne sofort (`startHead`/`audioStart`). */
  }
  /** Das älteste Häppchen ist zu Ende gesprochen (Test); das letzte löst `onIdle` aus. */
  finishPiece(): void {
    const head = this.queue.shift();
    if (head && !head.started) head.onStart?.();
    head?.onEnd?.();
    if (this.queue.length === 0 && !this.held) {
      this.playing = false;
      this.onIdle?.();
      return;
    }
    this.startHead();
  }
  /** Wiedergabe zu Ende (Test). */
  finish(): void {
    while (this.queue.length > 0) {
      const head = this.queue.shift();
      if (head && !head.started) head.onStart?.();
      head?.onEnd?.();
    }
    this.playing = false;
    this.onIdle?.();
  }
  pause(): boolean {
    if (this.held) return false;
    this.held = true;
    this.pauses++;
    return true;
  }
  resume(): void {
    if (!this.held) return;
    this.held = false;
    this.resumes++;
    if (this.queue.length === 0) {
      this.playing = false;
      this.onIdle?.();
      return;
    }
    if (!this.firstFired) {
      this.firstFired = true;
      this.onFirstSound?.(performance.now());
    }
    this.startHead();
  }
  stop(): void {
    this.stopped++;
    this.playing = false;
    this.held = false;
    this.queue = [];
    this.firstFired = false;
  }
  rearm(): void {
    this.firstFired = false;
  }
}
