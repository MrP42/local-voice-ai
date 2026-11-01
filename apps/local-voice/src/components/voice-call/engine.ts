import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CAPTURE_RATE,
  FRAME_SAMPLES,
  SpeechPlayer,
  floatToPcm16Base64,
  rms,
  startCapture,
  type Capture,
} from "./audio";
import { TurnTaker, type TurnConfig, DEFAULT_TURN_CONFIG } from "./turnTaking";
import {
  DEFAULT_ORCHESTRATION,
  OrchestrationSession,
  type CallStatus,
  type OrchestrationConfig,
  type Prepared,
} from "./orchestration";
import { COMPLIANCE_CHANGED } from "@/stores/complianceStore";
import { useSettingsStore } from "@/stores/settingsStore";
import type { ChatSide } from "@/vendor/wai-voice/backend.ts";

export const DEFAULT_SYSTEM_PROMPT = `Du bist ein Gesprächspartner am Telefon. Antworte so, wie man spricht: kurze, natürliche Sätze auf Deutsch, meist ein bis drei Sätze. Keine Listen, keine Aufzählungszeichen, keine Markdown-Zeichen, keine Emojis, keine Links. Zahlen und Abkürzungen so schreiben, wie man sie ausspricht. Stelle bei Unklarheit eine kurze Rückfrage, statt lange zu raten. Fasse dich kurz, der Gesprächspartner kann jederzeit nachfragen oder unterbrechen.`;

export type CallState =
  "idle" | "starting" | "listening" | "hearing" | "thinking" | "speaking";

export interface CallConfig {
  systemPrompt: string;
  turn: TurnConfig;
  /** Denker, Souveränität und Werkzeuge (Stufe 2). */
  orchestration: OrchestrationConfig;
}

export const DEFAULT_CALL_CONFIG: CallConfig = {
  systemPrompt: DEFAULT_SYSTEM_PROMPT,
  turn: DEFAULT_TURN_CONFIG,
  orchestration: DEFAULT_ORCHESTRATION,
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

export interface CallReady {
  llm: string;
  llm_local: boolean;
  warnings: string[];
  warmup_ms: number;
}

/** Verbrauch einer Antwort, aus dem `done`-Ereignis von `voice_call_chat`. */
export interface CallUsageEvent {
  prompt_tokens: number;
  completion_tokens: number;
  usage_event_id?: number | null;
  usage_event_ids?: number[];
  provider_id?: string | null;
  local?: boolean;
}

interface VoiceCallEvent {
  kind: string;
  request_id: string;
  index?: number;
  text?: string;
  usage?: CallUsageEvent;
}

export interface CallCallbacks {
  onState(state: CallState): void;
  onLevel(level: number): void;
  /** `turnId`: Kennung der Äußerung, damit eine spätere Korrektur genau sie trifft. */
  onUser(text: string, turnId: string): void;
  onAssistant(text: string, final: boolean, interrupted: boolean): void;
  onMetrics(m: TurnMetrics): void;
  onError(message: string): void;
  onUsage?(usage: CallUsageEvent): void;
  /** Werkzeug läuft / Hinweis des Orchestrators; `null` löscht die Werkzeugzeile. */
  onStatus?(status: CallStatus | null): void;
  /** Das laufende gespeicherte Gespräch (für die Wissenswerkzeuge); `null` bis zum ersten Beitrag; wartet auf ein laufendes Speichern. */
  conversationId?(): string | null | Promise<string | null>;
}

interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
  /** Äußerung des Nutzers, zu der die Nachricht gehört (für `correctUserText`). */
  turnId?: string;
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
  /** Was der Verlauf von diesem Schritt behält (ohne Dateitext). */
  userText: string;
  /** Löst das Warten auf das Ende des Stroms sofort auf (Abbruch). */
  finishStream?: () => void;
  /** Orchestrator dieses Schritts (nur wenn Denker oder Werkzeuge aktiv sind). */
  side?: ChatSide;
  spoken: string[];
  lastSpeechWall: number;
  startFrame: number;
}

const PREROLL_FRAMES = 15;
const TRAILING_FRAMES = 8;
const MAX_HISTORY = 24;
const MAX_UTTERANCE_FRAMES = 50 * 30;
/** Stumm geschaltet: das Mikrofon liefert der Erkennung nur Stille. */
const SILENT_FRAME = new Float32Array(FRAME_SAMPLES);
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
  private listening: Promise<UnlistenFn> | null = null;
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
  private sentenceHandlers = new Map<string, (e: VoiceCallEvent) => void>();
  private muted = false;
  /** Laufende Text-Anfrage (Textmodus ohne Anruf). */
  private textRun: {
    id: string;
    finish: () => void;
    partial: string;
    side?: ChatSide;
  } | null = null;
  /** Zählt Neustarts des Gesprächs: späte Antworten alter Anfragen werden verworfen. */
  private generation = 0;

  /** Orchestrator des Gesprächs (Werkzeuge, Modelle, Souveränität); je Engine einer. */
  private session: OrchestrationSession;
  private unwatch: (() => void) | null = null;

  constructor(cb: CallCallbacks, cfg: CallConfig) {
    this.cb = cb;
    this.cfg = cfg;
    this.taker = new TurnTaker(cfg.turn);
    this.session = new OrchestrationSession({
      subscribe: async (id, handler) => {
        await this.ensureListener();
        this.sentenceHandlers.set(id, handler);
        return () => {
          this.sentenceHandlers.delete(id);
        };
      },
      onUsage: (u) => this.cb.onUsage?.(u),
      onStatus: (s) => this.cb.onStatus?.(s),
      conversationId: async () => (await this.cb.conversationId?.()) ?? null,
    });
    // Ändern sich Modelle, Verbindungen, Regelwerk oder Einstellungen, liest der nächste Schritt neu.
    const invalidate = () => this.session.invalidate();
    window.addEventListener(COMPLIANCE_CHANGED, invalidate);
    const off = useSettingsStore.subscribe((now, before) => {
      if (now.settings !== before.settings) invalidate();
    });
    this.unwatch = () => {
      window.removeEventListener(COMPLIANCE_CHANGED, invalidate);
      off();
    };
  }

  /** Läuft ein Anruf (Mikrofon offen)? Sonst gilt der Textmodus ohne Ton. */
  get inCall(): boolean {
    return this.state !== "idle" && this.state !== "starting" && !this.stopped;
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
  }

  /** Neues Gespräch: der Verlauf, den die KI kennt, beginnt von vorn. */
  resetHistory(): void {
    this.generation++;
    this.session.invalidate();
    this.cancelText(false);
    this.cancelTurn(false);
    this.history = [];
  }

  /**
   * Der Nutzer hat eine erkannte Äußerung verbessert: die KI kennt ab jetzt die richtige Fassung.
   * Gesucht wird nach der Kennung der Äußerung, nicht nach dem Text (gleiche Sätze kommen vor).
   */
  correctUserText(turnId: string, to: string): void {
    const i = this.history.findIndex(
      (m) => m.role === "user" && m.turnId === turnId,
    );
    if (i >= 0) this.history[i] = { ...this.history[i], content: to };
  }

  /** Gespeichertes Gespräch öffnen: die KI kennt ab jetzt genau diesen Verlauf. */
  setHistory(
    messages: {
      role: "user" | "assistant";
      content: string;
      turnId?: string;
    }[],
  ): void {
    this.resetHistory();
    this.history = messages.slice(-MAX_HISTORY);
  }

  /**
   * Laufende Text-Anfrage abbrechen. `keepPartial`: das bisher Gestreamte
   * bleibt als unterbrochene Antwort stehen (Stopp-Knopf), sonst wird es verworfen.
   */
  cancelText(keepPartial = true): void {
    const run = this.textRun;
    if (!run) return;
    this.textRun = null;
    this.sentenceHandlers.delete(run.id);
    void run.side?.cancel?.(run.id);
    void invoke("voice_call_cancel", { requestId: run.id }).catch(
      () => undefined,
    );
    run.finish();
    this.cb.onStatus?.(null);
    if (keepPartial && run.partial)
      this.cb.onAssistant(run.partial, true, true);
  }

  /** Hörer für die Antwortstücke entfernen (Seite wird verlassen). */
  async dispose(): Promise<void> {
    this.unwatch?.();
    this.unwatch = null;
    this.session.dispose();
    const pending = this.listening;
    this.listening = null;
    (await pending)?.();
  }

  private async ensureListener(): Promise<void> {
    // Das Versprechen wird gemerkt: gleichzeitige Aufrufer teilen sich einen Hörer.
    this.listening ??= listen<VoiceCallEvent>("voice-call", (e) => {
      this.sentenceHandlers.get(e.payload.request_id)?.(e.payload);
    });
    await this.listening;
  }

  /** Wissensbereiche geändert: der nächste Schritt lädt Werkzeuge und Hinweis neu. */
  invalidateKnowledge(): void {
    this.session.invalidate();
  }

  updateConfig(cfg: CallConfig): void {
    this.cfg = cfg;
    this.session.invalidate();
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
      const ready = await invoke<CallReady>("voice_call_start");
      if (this.stopped) throw new Error("abgebrochen");
      this.ctx = new AudioContext({ latencyHint: "interactive" });
      await this.ctx.resume();
      if (this.stopped) throw new Error("abgebrochen");
      this.player = new SpeechPlayer(this.ctx);
      this.player.onIdle = () => {
        if (this.turn?.audible && this.turn.done) this.finishTurn();
      };
      await this.ensureListener();
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
    this.session.dispose();
    this.cancelText(true);
    this.cancelTurn(true);
    this.capture?.stop();
    this.capture = null;
    this.player?.stop();
    this.player = null;
    await this.ctx?.close().catch(() => undefined);
    this.ctx = null;
    this.frames = [];
    this.utt = null;
    this.taker.reset();
    this.setState("idle");
  }

  // -- Audio-Eingang --------------------------------------------------------

  private onFrame(raw: Float32Array): void {
    if (raw.length !== FRAME_SAMPLES) return;
    const frame = this.muted ? SILENT_FRAME : raw;
    this.frames.push(frame);
    this.frameIndex++;
    this.wallOffset = performance.now() - this.frameIndex * 20;
    // Puffer begrenzen, ohne laufende Äußerung anzutasten.
    const keepFrom = Math.min(
      this.utt?.startFrame ?? this.frameIndex - 100,
      this.turn?.startFrame ?? Infinity,
    );
    while (
      this.firstFrame < keepFrom - PREROLL_FRAMES &&
      this.frames.length > 0
    ) {
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
    let startFrame = Math.max(
      this.firstFrame,
      Math.round(at / 20) - PREROLL_FRAMES,
    );
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
    const aiStillSpeaking =
      (this.player?.active ?? false) || !!this.turn?.audible;
    if (utt.aiWasSpeaking && !utt.barged && aiStillSpeaking) return;
    if (speechMs < this.cfg.turn.minUtteranceMs) return;
    const endFrame = Math.min(
      this.frameIndex,
      Math.round(lastSpeechAt / 20) + TRAILING_FRAMES,
    );
    const startFrame = Math.max(
      this.firstFrame,
      Math.max(utt.startFrame, endFrame - MAX_UTTERANCE_FRAMES),
    );
    const slice = this.frames.slice(
      startFrame - this.firstFrame,
      endFrame - this.firstFrame,
    );
    if (slice.length === 0) return;
    const samples = new Float32Array(slice.length * FRAME_SAMPLES);
    slice.forEach((f, i) => samples.set(f, i * FRAME_SAMPLES));
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

  private newTurn(startFrame: number, lastSpeechWall: number): Turn {
    let commit!: () => void;
    const committedP = new Promise<void>((r) => (commit = r));
    return {
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
  }

  private async runTurn(
    samples: Float32Array,
    startFrame: number,
    lastSpeechWall: number,
    speechMs: number,
  ): Promise<void> {
    this.cancelTurn(false);
    const turn = this.newTurn(startFrame, lastSpeechWall);
    this.turn = turn;
    const dead = () => turn.cancelled || this.turn !== turn;

    try {
      const t0 = performance.now();
      const text = (
        await invoke<string>("voice_call_transcribe", {
          pcm16B64: floatToPcm16Base64(samples),
        })
      ).trim();
      const sttMs = performance.now() - t0;
      if (dead()) return;
      if (isPhantom(text, speechMs)) {
        this.dropTurn(turn);
        return;
      }
      await this.respond(turn, text, sttMs);
    } catch (e) {
      this.sentenceHandlers.delete(turn.id);
      if (!dead()) {
        this.cb.onError(String(e));
        this.dropTurn(turn);
      }
    }
  }

  /**
   * Getippter Text. Im Gespräch läuft er wie eine erkannte Äußerung ab der
   * Stelle nach der Spracherkennung (die KI antwortet hörbar, ein laufendes
   * Sprechen wird unterbrochen); ohne Gespräch gibt es nur Text, kein Ton.
   */
  async sendText(text: string, display?: string): Promise<void> {
    const clean = text.trim();
    if (!clean) return;
    if (!this.inCall) {
      await this.runTextOnly(clean, display);
      return;
    }
    this.cancelTurn(true);
    this.utt = null;
    this.taker.reset();
    this.setState("thinking");
    const turn = this.newTurn(this.frameIndex, performance.now());
    turn.committed = true;
    turn.commit();
    this.turn = turn;
    try {
      await this.respond(turn, clean, 0, display);
    } catch (e) {
      this.sentenceHandlers.delete(turn.id);
      if (!turn.cancelled && this.turn === turn) {
        this.cb.onError(String(e));
        this.dropTurn(turn);
      }
    }
  }

  /**
   * Orchestrator für diesen Schritt: `side` nur, wenn Denker oder Werkzeuge aktiv sind (sonst der
   * alte Weg); `blocked`, wenn der Sprecher bei der Souveränität gesperrt ist (kein Gespräch).
   */
  private async orchestrate(): Promise<Prepared> {
    await this.ensureListener();
    return this.session.prepare({
      ...this.cfg.orchestration,
      systemPrompt: this.cfg.systemPrompt,
    });
  }

  /** Verlauf plus neue Äußerung, ohne Systemanweisung (die hält der Orchestrator). */
  private historyFor(text: string): ChatMessage[] {
    return [...this.plainHistory(), { role: "user", content: text }];
  }

  /** Der Verlauf ohne die Kennungen: sie gehen nicht an das Modell. */
  private plainHistory(): ChatMessage[] {
    return this.history
      .slice(-MAX_HISTORY)
      .map(({ role, content }) => ({ role, content }));
  }

  private messagesFor(text: string, systemPrompt: string): ChatMessage[] {
    return [
      { role: "system", content: systemPrompt },
      ...this.plainHistory(),
      { role: "user", content: text },
    ];
  }

  /** Textmodus ohne Anruf: Sätze ins UI streamen, nichts sprechen. */
  private async runTextOnly(text: string, display?: string): Promise<void> {
    if (this.textRun) return;
    const gen = this.generation;
    const id = newId();
    let finish!: () => void;
    const streamEnded = new Promise<void>((r) => (finish = r));
    const run: NonNullable<VoiceCallEngine["textRun"]> = {
      id,
      finish,
      partial: "",
    };
    this.textRun = run;
    const stale = () => this.textRun !== run || gen !== this.generation;
    const shown = display ?? text;
    try {
      await this.ensureListener();
      if (stale()) return;
      this.cb.onUser(shown, id);
      const onPiece = (piece: string) => {
        run.partial += (run.partial ? " " : "") + piece;
        this.cb.onAssistant(run.partial, false, false);
      };
      const { side, blocked, systemPrompt } = await this.orchestrate();
      if (stale() || blocked) return;
      let full = "";
      if (side) {
        run.side = side;
        full = await side.chat(id, this.historyFor(text), (s) => {
          if (!stale()) onPiece(s);
        });
        if (stale()) return;
      } else {
        this.sentenceHandlers.set(id, (e) => {
          if (stale()) return;
          if (e.kind === "done" || e.kind === "failed") {
            if (e.usage) this.cb.onUsage?.(e.usage);
            finish();
          }
          if (e.kind !== "sentence") return;
          onPiece(e.text ?? "");
        });
        full = await invoke<string>("voice_call_chat", {
          requestId: id,
          messages: this.messagesFor(text, systemPrompt),
        });
        if (stale()) return;
        await Promise.race([
          streamEnded,
          new Promise<void>((r) => setTimeout(r, 3000)),
        ]);
        if (stale()) return;
      }
      const answer = full.trim() || run.partial;
      if (answer) {
        this.history.push({ role: "user", content: shown, turnId: id });
        this.history.push({ role: "assistant", content: answer });
        this.cb.onAssistant(answer, true, false);
      }
    } catch (e) {
      if (!stale()) this.cb.onError(String(e));
    } finally {
      this.sentenceHandlers.delete(id);
      if (this.textRun === run) {
        this.textRun = null;
        this.cb.onStatus?.(null);
      }
    }
  }

  /** Ab der Stelle nach der Spracherkennung: Antwort holen, sprechen, Verlauf führen. */
  private async respond(
    turn: Turn,
    text: string,
    sttMs: number,
    display?: string,
  ): Promise<void> {
    const dead = () => turn.cancelled || this.turn !== turn;
    turn.userText = display ?? text;
    this.cb.onUser(turn.userText, turn.id);

    const tChat = performance.now();
    let firstSentenceAt = 0;
    let firstAudioAt = 0;
    let chain: Promise<void> = Promise.resolve();
    let assistantText = "";

    let finishStream!: () => void;
    const streamEnded = new Promise<void>((r) => (finishStream = r));
    turn.finishStream = finishStream;
    const onPiece = (piece: string) => {
      if (!firstSentenceAt) firstSentenceAt = performance.now();
      assistantText += (assistantText ? " " : "") + piece;
      this.cb.onAssistant(assistantText, false, false);
      chain = chain.then(async () => {
        if (dead()) return;
        let wav: string;
        try {
          wav = await invoke<string>("voice_call_synthesize", {
            text: piece,
          });
        } catch (err) {
          if (!dead()) this.cb.onError(String(err));
          return;
        }
        if (dead()) return;
        if (!firstAudioAt) firstAudioAt = performance.now();
        await turn.committedP;
        if (dead()) return;
        if (!turn.audible) {
          turn.audible = true;
          this.player?.rearm();
          this.player!.onFirstSound = (at) => {
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
        await this.player!.enqueue(wav);
      });
    };

    const { side, blocked, systemPrompt } = await this.orchestrate();
    if (dead()) return;
    if (blocked) {
      // Kein Gespräch mit einem gesperrten Modell: der Hinweis steht im Gespräch.
      this.dropTurn(turn);
      return;
    }
    if (side) {
      // Denker oder Werkzeuge aktiv: der Orchestrator liefert die Sätze (auch die Überbrückung).
      turn.side = side;
      await side.chat(turn.id, this.historyFor(text), (s) => {
        if (!dead()) onPiece(s);
      });
    } else {
      this.sentenceHandlers.set(turn.id, (e) => {
        if ((e.kind === "done" || e.kind === "failed") && e.usage)
          this.cb.onUsage?.(e.usage);
        if (e.kind === "done" || e.kind === "failed") finishStream();
        if (e.kind !== "sentence" || dead()) return;
        onPiece(e.text ?? "");
      });
      await invoke<string>("voice_call_chat", {
        requestId: turn.id,
        messages: this.messagesFor(text, systemPrompt),
      });
      // Das letzte Häppchen kann nach dem Rückgabewert eintreffen: auf `done` warten.
      if (!dead())
        await Promise.race([
          streamEnded,
          new Promise<void>((r) => setTimeout(r, 3000)),
        ]);
    }
    await chain;
    this.cb.onStatus?.(null);
    this.sentenceHandlers.delete(turn.id);
    if (dead()) return;
    turn.done = true;
    if (turn.spoken.length === 0) {
      this.dropTurn(turn);
      return;
    }
    this.history.push({
      role: "user",
      content: turn.userText,
      turnId: turn.id,
    });
    this.history.push({ role: "assistant", content: assistantText });
    turn.historyPushed = true;
    this.cb.onAssistant(assistantText, true, false);
    // Ende der Wiedergabe meldet player.onIdle -> finishTurn.
    if (!this.player?.active) this.finishTurn();
  }

  private dropTurn(turn: Turn): void {
    if (this.turn === turn) this.turn = null;
    this.sentenceHandlers.delete(turn.id);
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
    turn.finishStream?.();
    this.turn = null;
    this.sentenceHandlers.delete(turn.id);
    void turn.side?.cancel?.(turn.id);
    void invoke("voice_call_cancel", { requestId: turn.id }).catch(
      () => undefined,
    );
    this.cb.onStatus?.(null);
    turn.commit();
    if (interrupted && turn.spoken.length > 0) {
      // Nur das tatsächlich Gesprochene gehört in den Verlauf.
      if (turn.historyPushed) this.history.splice(-2, 2);
      const said = turn.spoken.join(" ");
      this.history.push({
        role: "user",
        content: turn.userText,
        turnId: turn.id,
      });
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

export { CAPTURE_RATE };
