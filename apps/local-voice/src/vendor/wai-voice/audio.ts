// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
/**
 * Audio des Sprachgesprächs: Mikrofon (mit Echo-Unterdrückung des Browsers)
 * und gaplose Wiedergabe in EINEM AudioContext. Beides im selben Kontext zu
 * halten ist Absicht: nur so kennt die Echo-Unterdrückung das Signal, das aus
 * den Lautsprechern kommt, und zieht es vom Mikrofon ab.
 */

export const CAPTURE_RATE = 16000;
/** 20 ms bei 16 kHz */
export const FRAME_SAMPLES = 320;

const WORKLET = `
class Capture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.ratio = sampleRate / ${CAPTURE_RATE};
    this.pos = 0;
    this.prev = 0;
    this.out = new Float32Array(${FRAME_SAMPLES});
    this.n = 0;
  }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch) return true;
    // lineare Abtastratenwandlung auf 16 kHz
    for (let i = 0; i < ch.length; i++) {
      const cur = ch[i];
      while (this.pos < 1) {
        this.out[this.n++] = this.prev + (cur - this.prev) * this.pos;
        this.pos += this.ratio;
        if (this.n === ${FRAME_SAMPLES}) {
          this.port.postMessage(this.out.slice());
          this.n = 0;
        }
      }
      this.pos -= 1;
      this.prev = cur;
    }
    return true;
  }
}
registerProcessor("capture", Capture);
`;

export interface Capture {
  stop(): void;
}

export interface CaptureOptions {
  deviceId?: string;
  onFrame(frame: Float32Array): void;
}

export async function startCapture(ctx: AudioContext, opts: CaptureOptions): Promise<Capture> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      deviceId: opts.deviceId ? { exact: opts.deviceId } : undefined,
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  });
  const url = URL.createObjectURL(new Blob([WORKLET], { type: "application/javascript" }));
  try {
    await ctx.audioWorklet.addModule(url);
  } catch (e) {
    for (const t of stream.getTracks()) t.stop();
    throw e;
  } finally {
    URL.revokeObjectURL(url);
  }
  const source = ctx.createMediaStreamSource(stream);
  const node = new AudioWorkletNode(ctx, "capture", {
    numberOfInputs: 1,
    numberOfOutputs: 0,
  });
  node.port.onmessage = (e: MessageEvent<Float32Array>) => opts.onFrame(e.data);
  source.connect(node);
  return {
    stop() {
      node.port.onmessage = null;
      source.disconnect();
      node.disconnect();
      for (const t of stream.getTracks()) t.stop();
    },
  };
}

export function rms(frame: Float32Array): number {
  let sum = 0;
  for (let i = 0; i < frame.length; i++) {
    const v = frame[i] ?? 0;
    sum += v * v;
  }
  return Math.sqrt(sum / Math.max(1, frame.length));
}

export function floatToPcm16Base64(samples: Float32Array): string {
  const bytes = new Uint8Array(samples.length * 2);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i] ?? 0));
    view.setInt16(i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

export function base64ToArrayBuffer(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes.buffer;
}

/** Spielt Häppchen lückenlos hintereinander; `stop()` verwirft alles sofort. */
export class SpeechPlayer {
  private ctx: AudioContext;
  private nextAt = 0;
  private sources = new Set<AudioBufferSourceNode>();
  private gen = 0;
  /** Wird mit dem ersten hörbaren Häppchen eines Laufs aufgerufen. */
  onFirstSound?: (at: number) => void;
  onIdle?: () => void;
  private firstFired = false;

  constructor(ctx: AudioContext) {
    this.ctx = ctx;
  }

  get active(): boolean {
    return this.sources.size > 0;
  }

  async enqueue(wavB64: string): Promise<void> {
    const gen = this.gen;
    const buf = await this.ctx.decodeAudioData(base64ToArrayBuffer(wavB64));
    if (gen !== this.gen) return;
    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    src.connect(this.ctx.destination);
    const start = Math.max(this.ctx.currentTime + 0.02, this.nextAt);
    this.nextAt = start + buf.duration;
    this.sources.add(src);
    if (!this.firstFired) {
      this.firstFired = true;
      // Zeitpunkt, an dem der Ton tatsächlich beginnt (Wanduhr).
      const delayMs = Math.max(0, (start - this.ctx.currentTime) * 1000);
      const at = performance.now() + delayMs;
      this.onFirstSound?.(at);
    }
    src.onended = () => {
      this.sources.delete(src);
      if (gen === this.gen && this.sources.size === 0) this.onIdle?.();
    };
    src.start(start);
  }

  stop(): void {
    this.gen++;
    this.firstFired = false;
    for (const s of this.sources) {
      s.onended = null;
      try {
        s.stop();
      } catch {
        /* schon beendet */
      }
    }
    this.sources.clear();
    this.nextAt = 0;
  }

  /** Für einen neuen Lauf: der nächste erste Ton wird wieder gemeldet. */
  rearm(): void {
    this.firstFired = false;
  }
}
