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

export async function startCapture(
  ctx: AudioContext,
  opts: CaptureOptions,
): Promise<Capture> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      deviceId: opts.deviceId ? { exact: opts.deviceId } : undefined,
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  });
  const url = URL.createObjectURL(
    new Blob([WORKLET], { type: "application/javascript" }),
  );
  try {
    await ctx.audioWorklet.addModule(url);
  } catch (e) {
    stream.getTracks().forEach((t) => t.stop());
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
      stream.getTracks().forEach((t) => t.stop());
    },
  };
}

export function rms(frame: Float32Array): number {
  let sum = 0;
  for (let i = 0; i < frame.length; i++) sum += frame[i] * frame[i];
  return Math.sqrt(sum / Math.max(1, frame.length));
}

export function floatToPcm16Base64(samples: Float32Array): string {
  const bytes = new Uint8Array(samples.length * 2);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
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

/** Ein eingereihtes Häppchen: `offset` ist, wie viel davon schon gespielt wurde (nach einer Pause). */
interface Piece {
  buf: AudioBuffer;
  rate: number;
  offset: number;
  src: AudioBufferSourceNode | null;
  startAt: number;
  onEnd?: () => void;
  /** Wird einmal gerufen, sobald das Häppchen tatsächlich zu hören ist (nicht schon beim Einplanen). */
  onStart?: () => void;
  started: boolean;
}

/**
 * Spielt Häppchen lückenlos hintereinander; `stop()` verwirft alles sofort. `pause()` hält die
 * Wiedergabe an und merkt sich, wie weit jedes Häppchen war; `resume()` setzt dort fort. Der
 * Audio-Kontext selbst läuft weiter (das Mikrofon hängt daran).
 */
export class SpeechPlayer {
  private ctx: AudioContext;
  private nextAt = 0;
  private pieces: Piece[] = [];
  private gen = 0;
  private held = false;
  /** Wird mit dem ersten hörbaren Häppchen eines Laufs aufgerufen. */
  onFirstSound?: (at: number) => void;
  onIdle?: () => void;
  private firstFired = false;
  private gain: GainNode;
  private rate = 1;

  constructor(ctx: AudioContext) {
    this.ctx = ctx;
    this.gain = ctx.createGain();
    this.gain.connect(ctx.destination);
  }

  /** Lautstärke der Wiedergabe (Verstärkung, 1 = unverändert); gilt auch für Laufendes. */
  setVolume(gain: number): void {
    this.gain.gain.value = gain;
  }

  /** Abspielgeschwindigkeit für alles, was danach eingereiht wird (die Tonhöhe folgt ihr). */
  setRate(rate: number): void {
    this.rate = rate;
  }

  /** Ist noch etwas zu spielen (auch angehalten)? */
  get active(): boolean {
    return this.pieces.length > 0;
  }

  /** Angehalten (`pause()`), bis `resume()` oder `stop()`. */
  get paused(): boolean {
    return this.held;
  }

  /**
   * `onEnd`: wird gerufen, wenn dieses Häppchen zu Ende gespielt ist (nicht bei `stop()`).
   * `onStart`: wird gerufen, sobald es tatsächlich zu hören ist (Startzeit der Quelle erreicht,
   * siehe [`syncStarts`]); ein bloß eingeplantes Häppchen zählt nicht (lva#509).
   */
  async enqueue(
    wavB64: string,
    onEnd?: () => void,
    onStart?: () => void,
  ): Promise<void> {
    const gen = this.gen;
    const buf = await this.ctx.decodeAudioData(base64ToArrayBuffer(wavB64));
    if (gen !== this.gen) return;
    const piece: Piece = {
      buf,
      rate: this.rate,
      offset: 0,
      src: null,
      startAt: 0,
      onEnd,
      onStart,
      started: false,
    };
    this.pieces.push(piece);
    if (!this.held) this.schedule(piece, gen);
  }

  private schedule(piece: Piece, gen: number): void {
    const src = this.ctx.createBufferSource();
    src.buffer = piece.buf;
    src.playbackRate.value = piece.rate;
    src.connect(this.gain);
    const start = Math.max(this.ctx.currentTime + 0.02, this.nextAt);
    this.nextAt = start + (piece.buf.duration - piece.offset) / piece.rate;
    piece.src = src;
    piece.startAt = start;
    if (!this.firstFired) {
      this.firstFired = true;
      // Zeitpunkt, an dem der Ton tatsächlich beginnt (Wanduhr).
      const delayMs = Math.max(0, (start - this.ctx.currentTime) * 1000);
      const at = performance.now() + delayMs;
      this.onFirstSound?.(at);
    }
    src.onended = () => {
      this.pieces = this.pieces.filter((p) => p !== piece);
      if (gen !== this.gen) return;
      this.markStarted(piece);
      piece.onEnd?.();
      if (this.pieces.length === 0) this.onIdle?.();
    };
    src.start(start, piece.offset);
  }

  private markStarted(piece: Piece): void {
    if (piece.started) return;
    piece.started = true;
    piece.onStart?.();
  }

  /**
   * Meldet alle Häppchen als begonnen, deren Quelle läuft und deren Startzeit die Uhr des
   * Audio-Kontexts erreicht hat. Angehalten läuft nichts an. Der Aufrufer fragt unmittelbar vor der
   * Entscheidung nach, was zu hören war: Eingeplantes ohne Ton zählt nicht.
   */
  syncStarts(): void {
    if (this.held) return;
    const now = this.ctx.currentTime;
    for (const p of [...this.pieces])
      if (p.src && now >= p.startAt) this.markStarted(p);
  }

  /**
   * Hält die Wiedergabe sofort an (hörbar still); Eingereihtes und später Eintreffendes wartet.
   * Was schon zu Ende gespielt war, gilt als beendet. `false`, wenn sie schon angehalten war.
   */
  pause(): boolean {
    if (this.held) return false;
    // Zuerst festhalten, was schon zu hören war; danach läuft nichts mehr an.
    this.syncStarts();
    this.held = true;
    const now = this.ctx.currentTime;
    const done: Piece[] = [];
    for (const p of this.pieces) {
      if (!p.src) continue;
      p.src.onended = null;
      try {
        p.src.stop();
      } catch {
        /* schon beendet */
      }
      p.src = null;
      const played = Math.max(0, now - p.startAt) * p.rate;
      p.offset = Math.min(p.buf.duration, p.offset + played);
      if (p.offset >= p.buf.duration - 0.01) done.push(p);
    }
    this.pieces = this.pieces.filter((p) => !done.includes(p));
    this.nextAt = 0;
    for (const p of done) p.onEnd?.();
    return true;
  }

  /** Setzt eine angehaltene Wiedergabe dort fort, wo sie stand; ist nichts mehr da, meldet sie `onIdle`. */
  resume(): void {
    if (!this.held) return;
    this.held = false;
    this.nextAt = 0;
    if (this.pieces.length === 0) {
      this.onIdle?.();
      return;
    }
    const gen = this.gen;
    for (const p of [...this.pieces]) this.schedule(p, gen);
  }

  stop(): void {
    this.gen++;
    this.firstFired = false;
    this.held = false;
    for (const p of this.pieces) {
      if (!p.src) continue;
      p.src.onended = null;
      try {
        p.src.stop();
      } catch {
        /* schon beendet */
      }
    }
    this.pieces = [];
    this.nextAt = 0;
  }

  /** Für einen neuen Lauf: der nächste erste Ton wird wieder gemeldet. */
  rearm(): void {
    this.firstFired = false;
  }
}
