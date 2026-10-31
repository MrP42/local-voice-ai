// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
import {
  type Capture,
  FRAME_SAMPLES,
  floatToPcm16Base64,
  rms,
  SpeechPlayer,
  startCapture,
} from "./audio.ts";
import type { CallReady, ChatMessage, SpeechBackend } from "./backend.ts";
import { DEFAULT_TURN_CONFIG, type TurnConfig, TurnTaker } from "./turnTaking.ts";

export const DEFAULT_SYSTEM_PROMPT = `Du bist ein Gesprächspartner am Telefon. Antworte so, wie man spricht: kurze, natürliche Sätze auf Deutsch, meist ein bis drei Sätze. Keine Listen, keine Aufzählungszeichen, keine Markdown-Zeichen, keine Emojis, keine Links. Zahlen und Abkürzungen so schreiben, wie man sie ausspricht. Stelle bei Unklarheit eine kurze Rückfrage, statt lange zu raten. Fasse dich kurz, der Gesprächspartner kann jederzeit nachfragen oder unterbrechen.`;

export type CallState = "idle" | "starting" | "listening" | "hearing" | "thinking" | "speaking";

export interface CallConfig {
  systemPrompt: string;
  turn: TurnConfig;
}

export const DEFAULT_CALL_CONFIG: CallConfig = {
  systemPrompt: DEFAULT_SYSTEM_PROMPT,
  turn: DEFAULT_TURN_CONFIG,
};

export interface TurnMetrics {
  /** Spracherkennung (ms) */
  sttMs: number;
  /** Ende der Spracherkennung bis zum ersten Antwort-Häppchen (ms) */
  llmFirstMs: number;
  /** erstes Häppchen bis fertiges Audio (ms) */
  ttsFirstMs: number;
  /** letzter Sprachlaut des Nutzers bis erster Ton der KI (ms) */
  firstSoundMs: number;
}

export interface CallCallbacks {
  onState(state: CallState): void;
  onLevel(level: number): void;
  onUser(text: string): void;
  onAssistant(text: string, final: boolean, interrupted: boolean): void;
  onMetrics(m: TurnMetrics): void;
  onError(message: string): void;
}

interface Utterance {
  startFrame: number;
  aiWasSpeaking: boolean;
  barged: boolean;
}

interface Turn {
  id: string;
  cancelled: boolean;
  committed: boolean;
  commit: () => void;
  committedP: Promise<void>;
  audible: boolean;
  /** Antwort ist vollständig erzeugt (das Sprechen kann noch laufen). */
  done: boolean;
  /** Verlauf wurde schon um diesen Schritt ergänzt. */
  historyPushed: boolean;
  userText: string;
  spoken: string[];
  lastSpeechWall: number;
  startFrame: number;
}

const PREROLL_FRAMES = 15;
const TRAILING_FRAMES = 8;
const MAX_HISTORY = 24;
const MAX_UTTERANCE_FRAMES = 50 * 30;
/** Typische Erkennungsfehler bei Stille/Rauschen; zählen nur bei kurzen Äußerungen. */
const PHANTOMS =
  /^(untertitel|vielen dank|danke(schön)?\.?|tschüss|bis zum nächsten mal|\[.*\]|\(.*\))/i;

let counter = 0;
const newId = () => `vc-${Date.now().toString(36)}-${counter++}`;

export class VoiceCallEngine {
  private cb: CallCallbacks;
  private cfg: CallConfig;
  private ctx: AudioContext | null = null;
  private capture: Capture | null = null;
  private player: SpeechPlayer | null = null;
  private taker: TurnTaker;
  private state: CallState = "idle";
  private stopped = false;

  private frames: Float32Array[] = [];
  private firstFrame = 0;
  private frameIndex = 0;
  private wallOffset = 0;

  private utt: Utterance | null = null;
  private turn: Turn | null = null;
  private history: ChatMessage[] = [];

  private backend: SpeechBackend;

  constructor(backend: SpeechBackend, cb: CallCallbacks, cfg: CallConfig) {
    this.backend = backend;
    this.cb = cb;
    this.cfg = cfg;
    this.taker = new TurnTaker(cfg.turn);
  }

  updateConfig(cfg: CallConfig): void {
    this.cfg = cfg;
    this.taker.config = cfg.turn;
  }

  private setState(s: CallState): void {
    if (this.state === s) return;
    this.state = s;
    this.cb.onState(s);
  }

  async start(): Promise<CallReady> {
    this.stopped = false;
    this.setState("starting");
    try {
      const ready = await this.backend.start();
      if (this.stopped) throw new Error("abgebrochen");
      this.ctx = new AudioContext({ latencyHint: "interactive" });
      await this.ctx.resume();
      if (this.stopped) throw new Error("abgebrochen");
      this.player = new SpeechPlayer(this.ctx);
      this.player.onIdle = () => {
        if (this.turn?.audible && this.turn.done) this.finishTurn();
      };
      this.history = [];
      const capture = await startCapture(this.ctx, {
        onFrame: (f) => this.onFrame(f),
      });
      if (this.stopped) {
        capture.stop();
        throw new Error("abgebrochen");
      }
      this.capture = capture;
      this.setState("listening");
      return ready;
    } catch (e) {
      await this.stop();
      throw e;
    }
  }

  async stop(): Promise<void> {
    this.stopped = true;
    this.cancelTurn(true);
    this.capture?.stop();
    this.capture = null;
    this.player?.stop();
    this.player = null;
    this.backend.dispose?.();
    await this.ctx?.close().catch(() => undefined);
    this.ctx = null;
    this.frames = [];
    this.utt = null;
    this.taker.reset();
    this.setState("idle");
  }

  // -- Audio-Eingang --------------------------------------------------------

  private onFrame(frame: Float32Array): void {
    if (frame.length !== FRAME_SAMPLES) return;
    this.frames.push(frame);
    this.frameIndex++;
    this.wallOffset = performance.now() - this.frameIndex * 20;
    // Puffer begrenzen, ohne laufende Äußerung anzutasten.
    const keepFrom = Math.min(
      this.utt?.startFrame ?? this.frameIndex - 100,
      this.turn?.startFrame ?? Infinity,
    );
    while (this.firstFrame < keepFrom - PREROLL_FRAMES && this.frames.length > 0) {
      this.frames.shift();
      this.firstFrame++;
    }

    const level = rms(frame);
    this.cb.onLevel(level);
    const aiSpeaking = (this.player?.active ?? false) || !!this.turn?.audible;
    const at = this.frameIndex * 20;
    for (const ev of this.taker.frame(at, level, aiSpeaking)) {
      switch (ev.kind) {
        case "speech_start":
          this.onSpeechStart(ev.at, aiSpeaking);
          break;
        case "barge_in":
          this.onBargeIn();
          break;
        case "early_end":
          this.onEarlyEnd(ev.lastSpeechAt, ev.speechMs);
          break;
        case "resume":
          this.onResume();
          break;
        case "end":
          this.onEnd();
          break;
      }
    }
  }

  private wall(streamMs: number): number {
    return streamMs + this.wallOffset;
  }

  private onSpeechStart(at: number, aiSpeaking: boolean): void {
    // Spricht der Nutzer weiter, bevor die KI zu hören ist: Vorarbeit verwerfen,
    // die Äußerung beginnt wieder am alten Anfang (nichts geht verloren).
    let startFrame = Math.max(this.firstFrame, Math.round(at / 20) - PREROLL_FRAMES);
    if (this.turn && !this.turn.audible) {
      startFrame = Math.min(startFrame, this.turn.startFrame);
      this.cancelTurn(false);
    }
    this.utt = { startFrame, aiWasSpeaking: aiSpeaking, barged: false };
    if (!aiSpeaking) this.setState("hearing");
  }

  private onBargeIn(): void {
    if (this.utt) this.utt.barged = true;
    this.cancelTurn(true);
    this.setState("hearing");
  }

  private onResume(): void {
    this.cancelTurn(false);
    this.setState("hearing");
  }

  private onEarlyEnd(lastSpeechAt: number, speechMs: number): void {
    const utt = this.utt;
    if (!utt) return;
    // Husten oder Nachhall, während die KI spricht: kein Unterbrechen, kein Turn.
    const aiStillSpeaking = (this.player?.active ?? false) || !!this.turn?.audible;
    if (utt.aiWasSpeaking && !utt.barged && aiStillSpeaking) return;
    if (speechMs < this.cfg.turn.minUtteranceMs) return;
    const endFrame = Math.min(this.frameIndex, Math.round(lastSpeechAt / 20) + TRAILING_FRAMES);
    const startFrame = Math.max(
      this.firstFrame,
      Math.max(utt.startFrame, endFrame - MAX_UTTERANCE_FRAMES),
    );
    const slice = this.frames.slice(startFrame - this.firstFrame, endFrame - this.firstFrame);
    if (slice.length === 0) return;
    const samples = new Float32Array(slice.length * FRAME_SAMPLES);
    for (const [i, f] of slice.entries()) samples.set(f, i * FRAME_SAMPLES);
    this.setState("thinking");
    void this.runTurn(samples, startFrame, this.wall(lastSpeechAt), speechMs);
  }

  private onEnd(): void {
    const turn = this.turn;
    this.utt = null;
    if (!turn) {
      this.setState(this.player?.active ? "speaking" : "listening");
      return;
    }
    turn.committed = true;
    turn.commit();
  }

  // -- Ein Gesprächsschritt ---------------------------------------------------

  private async runTurn(
    samples: Float32Array,
    startFrame: number,
    lastSpeechWall: number,
    speechMs: number,
  ): Promise<void> {
    this.cancelTurn(false);
    let commit!: () => void;
    const committedP = new Promise<void>((r) => {
      commit = r;
    });
    const turn: Turn = {
      id: newId(),
      cancelled: false,
      committed: false,
      commit,
      committedP,
      audible: false,
      done: false,
      historyPushed: false,
      userText: "",
      spoken: [],
      lastSpeechWall,
      startFrame,
    };
    this.turn = turn;
    const dead = () => turn.cancelled || this.turn !== turn;

    try {
      const t0 = performance.now();
      const text = (await this.backend.transcribe(floatToPcm16Base64(samples))).trim();
      const sttMs = performance.now() - t0;
      if (dead()) return;
      if (isPhantom(text, speechMs)) {
        this.dropTurn(turn);
        return;
      }
      turn.userText = text;
      this.cb.onUser(text);

      const messages: ChatMessage[] = [
        { role: "system", content: this.cfg.systemPrompt },
        ...this.history.slice(-MAX_HISTORY),
        { role: "user", content: text },
      ];

      const tChat = performance.now();
      let firstSentenceAt = 0;
      let firstAudioAt = 0;
      let chain: Promise<void> = Promise.resolve();
      let assistantText = "";

      const onSentence = (piece: string): void => {
        if (dead()) return;
        if (!firstSentenceAt) firstSentenceAt = performance.now();
        assistantText += (assistantText ? " " : "") + piece;
        this.cb.onAssistant(assistantText, false, false);
        chain = chain.then(async () => {
          if (dead()) return;
          let wav: string;
          try {
            wav = await this.backend.synthesize(piece);
          } catch (e) {
            if (!dead()) this.cb.onError(String(e));
            return;
          }
          if (dead()) return;
          if (!firstAudioAt) firstAudioAt = performance.now();
          await turn.committedP;
          const player = this.player;
          if (dead() || !player) return;
          if (!turn.audible) {
            turn.audible = true;
            player.rearm();
            player.onFirstSound = (at) => {
              this.setState("speaking");
              this.cb.onMetrics({
                sttMs: Math.round(sttMs),
                llmFirstMs: Math.round(firstSentenceAt - tChat),
                ttsFirstMs: Math.round(firstAudioAt - firstSentenceAt),
                firstSoundMs: Math.round(at - turn.lastSpeechWall),
              });
            };
          }
          turn.spoken.push(piece);
          await player.enqueue(wav);
        });
      };

      // Das Backend ruft onSentence für jedes Häppchen und löst erst auf,
      // wenn das letzte geliefert ist.
      await this.backend.chat(turn.id, messages, onSentence);
      await chain;
      if (dead()) return;
      turn.done = true;
      if (turn.spoken.length === 0) {
        this.dropTurn(turn);
        return;
      }
      this.history.push({ role: "user", content: text });
      this.history.push({ role: "assistant", content: assistantText });
      turn.historyPushed = true;
      this.cb.onAssistant(assistantText, true, false);
      // Ende der Wiedergabe meldet player.onIdle -> finishTurn.
      if (!this.player?.active) this.finishTurn();
    } catch (e) {
      if (!dead()) {
        this.cb.onError(String(e));
        this.dropTurn(turn);
      }
    }
  }

  private dropTurn(turn: Turn): void {
    if (this.turn === turn) this.turn = null;
    if (!this.utt) this.setState("listening");
  }

  private finishTurn(): void {
    this.turn = null;
    if (!this.utt) this.setState("listening");
  }

  /** Laufenden Schritt verwerfen; `interrupted`: der Nutzer hat die KI unterbrochen. */
  private cancelTurn(interrupted: boolean): void {
    const turn = this.turn;
    if (!turn) return;
    turn.cancelled = true;
    this.turn = null;
    void this.backend.cancel(turn.id).catch(() => undefined);
    turn.commit();
    if (interrupted && turn.spoken.length > 0) {
      // Nur das tatsächlich Gesprochene gehört in den Verlauf.
      if (turn.historyPushed) this.history.splice(-2, 2);
      const said = turn.spoken.join(" ");
      this.history.push({ role: "user", content: turn.userText });
      this.history.push({ role: "assistant", content: `${said} …` });
      this.cb.onAssistant(said, true, true);
    }
    this.player?.stop();
  }
}

function isPhantom(text: string, speechMs: number): boolean {
  if (text.length < 2) return true;
  if (/^[\s.,!?…–-]*$/.test(text)) return true;
  return speechMs < 1200 && PHANTOMS.test(text);
}
