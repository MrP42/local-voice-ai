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
  private queue: { onEnd?: () => void }[] = [];
  get active(): boolean {
    return this.playing;
  }
  get paused(): boolean {
    return this.held;
  }
  async enqueue(wav: string, onEnd?: () => void): Promise<void> {
    this.enqueued.push(wav);
    this.queue.push({ onEnd });
    this.playing = true;
    if (!this.firstFired && !this.held) {
      this.firstFired = true;
      this.onFirstSound?.(performance.now());
    }
  }
  /** Das älteste Häppchen ist zu Ende gesprochen (Test); das letzte löst `onIdle` aus. */
  finishPiece(): void {
    this.queue.shift()?.onEnd?.();
    if (this.queue.length === 0 && !this.held) {
      this.playing = false;
      this.onIdle?.();
    }
  }
  /** Wiedergabe zu Ende (Test). */
  finish(): void {
    while (this.queue.length > 0) this.queue.shift()?.onEnd?.();
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
