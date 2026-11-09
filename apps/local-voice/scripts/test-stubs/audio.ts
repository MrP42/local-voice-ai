// Attrappe des Players: reiht Häppchen ein und meldet Ende nur auf `finish()`.
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
  get active(): boolean {
    return this.playing;
  }
  async enqueue(wav: string): Promise<void> {
    this.enqueued.push(wav);
    this.playing = true;
    if (!this.firstFired) {
      this.firstFired = true;
      this.onFirstSound?.(performance.now());
    }
  }
  /** Wiedergabe zu Ende (Test). */
  finish(): void {
    this.playing = false;
    this.onIdle?.();
  }
  stop(): void {
    this.stopped++;
    this.playing = false;
    this.firstFired = false;
  }
  rearm(): void {
    this.firstFired = false;
  }
}
