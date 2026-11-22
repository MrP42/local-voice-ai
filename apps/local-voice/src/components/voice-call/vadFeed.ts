/** Silero-Zufuhr ohne Tauri: Zeiten beziehen sich auf den Anfang der Samples. */
type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

interface Chunk {
  samples: Float32Array;
  at: number;
}

interface Session {
  id?: string;
  enabled: boolean;
  queue: Chunk[];
  partial: Float32Array;
  filled: number;
  at: number;
  sent: number;
  processed: number;
  spans: { offset: number; at: number; length: number }[];
}

const CHUNK_SAMPLES = 1600;
const MAX_QUEUE = 10;
const VAD_SAMPLES = 480;
const SAMPLES_PER_MS = 16;

export class VadFeed {
  private session: Session | null = null;
  private busy = false;
  private logged = false;

  private invoke: Invoke;
  private encode: (samples: Float32Array) => string;
  private onProb: (at: number, prob: number) => void;
  private onFallback: () => void;

  constructor(
    invoke: Invoke,
    encode: (samples: Float32Array) => string,
    onProb: (at: number, prob: number) => void,
    onFallback: () => void,
  ) {
    this.invoke = invoke;
    this.encode = encode;
    this.onProb = onProb;
    this.onFallback = onFallback;
  }

  /** Öffnen läuft neben dem Gesprächsstart; bis dahin werden Häppchen gepuffert. */
  start(): void {
    this.logged = false;
    this.open();
  }

  /** Ein Reset entwertet auch laufende Antworten und den Rest im Rust-Puffer. */
  reset(): void {
    if (this.session?.enabled) this.open();
  }

  stop(): void {
    const previous = this.session;
    this.session = null;
    if (previous?.id) this.close(previous.id);
  }

  private open(): void {
    this.stop();
    const session: Session = {
      enabled: true,
      queue: [],
      partial: new Float32Array(CHUNK_SAMPLES),
      filled: 0,
      at: 0,
      sent: 0,
      processed: 0,
      spans: [],
    };
    this.session = session;
    void this.invoke<string>("voice_call_vad_open").then(
      (id) => {
        if (this.session !== session) {
          this.close(id);
          return;
        }
        session.id = id;
        void this.drain();
      },
      () => this.fail(session),
    );
  }

  private close(sessionId: string): void {
    void this.invoke<void>("voice_call_vad_close", { sessionId }).catch(() => undefined);
  }

  private fail(session: Session): void {
    if (this.session !== session || !session.enabled) return;
    session.enabled = false;
    session.queue = [];
    session.filled = 0;
    session.spans = [];
    if (session.id) {
      this.close(session.id);
      session.id = undefined;
    }
    if (!this.logged) {
      this.logged = true;
      this.onFallback();
    }
  }

  /** `at` ist die Streamzeit des ersten Samples, nicht die Empfangszeit. */
  frame(samples: Float32Array, at: number): void {
    const session = this.session;
    if (!session?.enabled) return;
    let offset = 0;
    while (offset < samples.length) {
      if (session.filled === 0) session.at = at + offset / SAMPLES_PER_MS;
      const count = Math.min(CHUNK_SAMPLES - session.filled, samples.length - offset);
      session.partial.set(samples.subarray(offset, offset + count), session.filled);
      session.filled += count;
      offset += count;
      if (session.filled === CHUNK_SAMPLES) {
        session.queue.push({ samples: session.partial, at: session.at });
        if (session.queue.length > MAX_QUEUE) session.queue.shift();
        session.partial = new Float32Array(CHUNK_SAMPLES);
        session.filled = 0;
      }
    }
    void this.drain();
  }

  private async drain(): Promise<void> {
    if (this.busy) return;
    const session = this.session;
    if (!session?.enabled || !session.id) return;
    this.busy = true;
    try {
      while (this.session === session && session.enabled && session.queue.length > 0) {
        const chunk = session.queue.shift()!;
        // Nur gesendete Samples zählen: verworfene Häppchen hinterlassen Zeitlücken.
        session.spans.push({
          offset: session.sent,
          at: chunk.at,
          length: chunk.samples.length,
        });
        session.sent += chunk.samples.length;
        const probs = await this.invoke<number[]>("voice_call_vad_push", {
          sessionId: session.id,
          pcm16B64: this.encode(chunk.samples),
        });
        if (this.session !== session || !session.enabled) return;
        for (const prob of probs) {
          const offset = session.processed;
          const span = session.spans.find((s) => offset < s.offset + s.length)!;
          this.onProb(span.at + (offset - span.offset) / SAMPLES_PER_MS, prob);
          session.processed += VAD_SAMPLES;
        }
        // Ein angebrochenes 30-ms-Fenster behält seinen ursprünglichen Zeitanker.
        session.spans = session.spans.filter(
          (s) => s.offset + s.length > session.processed,
        );
      }
    } catch {
      this.fail(session);
    } finally {
      this.busy = false;
      // Ein Reset kann inzwischen eine neue Sitzung geöffnet haben.
      if (this.session !== session) void this.drain();
    }
  }
}
