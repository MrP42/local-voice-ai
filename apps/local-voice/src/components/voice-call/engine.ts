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
import {
  GlobalContext,
  speakable,
  speakerContext,
  modeSwitch,
  modeSwitchReply,
  type TalkMode,
  type ContextHit,
  type PreparedContext,
} from "./globalContext";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  hinweisAusEreignis,
  hinweisAusFehler,
  type ProjektBindung,
  type ProjektBudget,
  type ProjektHinweis,
} from "./projekt";
import type { StufeDto } from "@/bindings";
import type { ChatSide } from "@/vendor/wai-voice/backend.ts";
import {
  aktuelleSprache,
  einstufen,
  letzteAusgabe,
  meldeMessung,
  vergissAusgabe,
} from "./assistent";
import {
  baueMessung,
  istVorlage,
  planAssistent,
  stufeVon,
  vorlage,
  type AssistentPlan,
  type Marken,
  type MessArt,
  type Sprache,
  type VorlagePlan,
} from "./assistentPlan";
import {
  LAUTSTAERKE,
  START_STUFEN,
  TEMPO,
  bedienen,
  type Stufen,
} from "./bedienung";
import i18n from "@/i18n";
import { istUeberbrueckung } from "./ueberbrueckung";
import {
  darfZusammenfuehren,
  entscheide,
  fuegeZusammen,
  schliesstSatz,
  systemMitHinweis,
  unterbrechungsHinweis,
  type TurnStand,
} from "./ergaenzen";

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
  /** Schritt, zu dem die Messung gehört (damit eine Zusammenführung sie zurücknehmen kann). */
  turnId?: string;
  /** Spracherkennung (ms) */
  sttMs: number;
  /** Ende der Spracherkennung bis zum ersten Antwort-Häppchen (ms) */
  llmFirstMs: number;
  /** erstes Häppchen bis fertiges Audio (ms) */
  ttsFirstMs: number;
  /** letzter Sprachlaut des Nutzers bis erster Ton der KI (ms) */
  firstSoundMs: number;
  /** Globaler Kontext: Abruf und Prompt fertig (ms ab Start des Abrufs), fehlt ohne Kontext. */
  contextMs?: number;
  /** Zahl der Auszüge im Prompt (0 = gesucht, nichts gefunden). */
  contextHits?: number;
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
  /** `projekt`: Hinweis aus dem Projektweg (Code, Zähler, Budgetanteile; nie Inhalte). */
  code?: string;
  zurueckgesetzt?: boolean;
  entzogen?: number;
  budget?: ProjektBudget | null;
}

export interface CallCallbacks {
  onState(state: CallState): void;
  onLevel(level: number): void;
  /** `turnId`: Kennung der Äußerung, damit eine spätere Korrektur genau sie trifft. */
  onUser(text: string, turnId: string): void;
  onAssistant(text: string, final: boolean, interrupted: boolean): void;
  onMetrics(m: TurnMetrics): void;
  /** Der Schritt wurde durch eine Ergänzung ersetzt: seine Messung gilt nicht (lva#485). */
  onMetricsVerwerfen?(turnId: string): void;
  onError(message: string): void;
  onUsage?(usage: CallUsageEvent): void;
  /** Werkzeug läuft / Hinweis des Orchestrators; `null` löscht die Werkzeugzeile. */
  onStatus?(status: CallStatus | null): void;
  /** Das laufende gespeicherte Gespräch (für die Wissenswerkzeuge); `null` bis zum ersten Beitrag; wartet auf ein laufendes Speichern. */
  conversationId?(): string | null | Promise<string | null>;
  /** Globaler Sprachassistent (lva#325): geprüfte Belege einer Antwort. */
  onCitations?(turnId: string, hits: ContextHit[]): void;
  /** Zeitpunkte des Kontexts (Messung t2/t3, Plan M7). */
  onContextTiming?(
    turnId: string,
    retrieveMs: number,
    promptMs: number,
    hits: number,
  ): void;
  /** Buch-Gespräch: per Sprache auf „allgemein“ oder „im Buch“ gewechselt. */
  onTalkMode?(mode: TalkMode): void;
  /**
   * Das Projekt des offenen Gespräches (aus dem gespeicherten Gespräch) und der Stand der Auswahl, den
   * die Person gesehen hat; `null` in einem gewöhnlichen Gespräch. Im Projekt gibt es keinen
   * App-Kontext, keine Werkzeuge und keinen Orchestrator; die Antwort kommt nur über das Versandtor.
   */
  projekt?(): ProjektBindung | null;
  /** Hinweis aus dem Projektweg (Budgetstopp, Stand veraltet, Verlauf zurückgesetzt, …). */
  onProjekt?(hinweis: ProjektHinweis): void;
  /**
   * Die laufende, noch nicht endgültige Antwortzeile entfernen (Widerruf im Projekt): Was das Tor nicht
   * freigegeben hat, steht danach nirgends mehr, auch nicht als angefangene Antwort.
   */
  onVerwerfen?(): void;
}

interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
  /** Äußerung des Nutzers, zu der die Nachricht gehört (für `correctUserText`). */
  turnId?: string;
  /** Im Buch-Gespräch „allgemein“ entstanden: geht nicht an die Buch-Seite. */
  general?: boolean;
  /** Antwort aus App-Kontext: nur an lokale Empfänger, fällt bei Widerruf weg. */
  ctx?: boolean;
  /** Antwort, die der Nutzer mitten im Sprechen unterbrochen hat (der nächste Aufruf bekommt einen Hinweis). */
  interrupted?: boolean;
}

interface Utterance {
  startFrame: number;
  aiWasSpeaking: boolean;
  barged: boolean;
}

/** Ein eingereihtes Häppchen: `ended` = zu Ende gespielt, `queued` = beim Player eingereiht. */
interface PieceRec {
  text: string;
  kind: PieceKind;
  ended: boolean;
  queued: boolean;
  /** Der Ton dieses Häppchens hat tatsächlich begonnen (nicht bloß eingeplant, lva#509). */
  started: boolean;
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
  /** Der Schritt antwortet mit App-Kontext (Widerruf beendet ihn). */
  withContext?: boolean;
  /**
   * Alle Häppchen in Einreihreihenfolge. Was der Nutzer gehört hat, wird daraus gelesen (`gehoert`):
   * nur Inhaltshäppchen (ohne Ansage und Vorlage) gehören in den Verlauf.
   */
  pieces: PieceRec[];
  /** Alle Häppchen, die hörbar eingereiht wurden (auch Ansage und Vorlage). */
  heard: number;
  /** Vorlagenantwort („Ich wiederhole.“, feste Fehlermeldung): nie in den Verlauf. */
  noHistory?: boolean;
  /** Im Projekt: der Text, den das Tor freigegeben hat (Rückgabe von `voice_call_chat`). */
  projektText?: string;
  /** Wird einmal gerufen, wenn der Schritt zu Ende gesprochen ist (Messung E9: `fertig_ms`). */
  onEnde?: () => void;
  lastSpeechWall: number;
  startFrame: number;
  /** Der Schritt entstand aus gesprochener Eingabe (nur dann gilt Ergänzen, lva#485). */
  fromSpeech?: boolean;
  /** Vorlage, S0-Antwort, Bedienung oder Wechsel: wirkt sofort, wird nie mit einer Ergänzung neu gestellt. */
  noMerge?: boolean;
  /** Der erste Inhaltssatz ist zu Ende gesprochen: ab jetzt ist ein Unterbrechen ein neuer Beitrag. */
  firstContentEnded?: boolean;
  /** Die fertige Antwortzeile; sie wird erst gemeldet (und damit gespeichert), wenn sie nicht mehr ersetzt werden kann. */
  finalText?: string;
  finalSent?: boolean;
  /** Zeit, die die Wiedergabe dieses Schritts angehalten war (ms): gehört nicht in die Messung. */
  heldMs?: number;
  /** Beginn des laufenden Anhaltens (Wanduhr). */
  heldSince?: number;
  /** Text vor der Äußerung, die diesen Schritt auslöste (Ergänzung): geht beim Weitersprechen nicht verloren. */
  prefix?: string;
  /** Die Messung dieses Schritts wurde schon gemeldet. */
  metricsSent?: boolean;
}

const PREROLL_FRAMES = 15;
const TRAILING_FRAMES = 8;
const MAX_HISTORY = 24;
/** Stellschrauben, die Tests verkürzen dürfen. */
export const TUNING = {
  /** So lange darf das Modell ohne ersten Inhalt bleiben, bevor die feste Vorlage kommt (ms). */
  modelFristMs: 30_000,
  /**
   * Im Projekt puffert das Tor die ganze Antwort: Das erste Stück kommt erst nach Erzeugung und Urteil.
   * Jedes Lebenszeichen („laeuft“) verlängert die Frist; diese Obergrenze gilt trotzdem (ms, lva#459).
   */
  projektObergrenzeMs: 300_000,
  /** So lange darf die Erkennung einer Äußerung dauern, während die Wiedergabe angehalten ist (ms). */
  holdSttMaxMs: 8_000,
};
/** Was ein Häppchen ist: Inhalt (Modell, S0-Antwort), Ansage (Ab S1, Überbrückung) oder feste Vorlage. */
type PieceKind = "inhalt" | "ansage" | "vorlage";

const FRIST = Symbol("frist");

/**
 * Frist bis zum ersten Modellinhalt: `abgelaufen` löst nur aus, wenn `stop` vorher nicht kam.
 * Mit `maxMs` (Projekt) verlängert jedes `lebenszeichen` die Frist um `ms`, aber nie über `maxMs`
 * seit dem Start hinaus; ohne `maxMs` bleibt die Frist fest.
 */
function ersteFrist(
  ms: number,
  maxMs?: number,
): {
  abgelaufen: Promise<typeof FRIST>;
  stop: () => void;
  lebenszeichen: () => void;
} {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let deckel: ReturnType<typeof setTimeout> | undefined;
  let gestoppt = false;
  let ablaufen: (f: typeof FRIST) => void = () => undefined;
  const abgelaufen = new Promise<typeof FRIST>((r) => {
    ablaufen = r;
    timer = setTimeout(() => r(FRIST), ms);
    if (maxMs !== undefined) deckel = setTimeout(() => r(FRIST), maxMs);
  });
  return {
    abgelaufen,
    stop: () => {
      gestoppt = true;
      clearTimeout(timer);
      clearTimeout(deckel);
    },
    lebenszeichen: () => {
      if (maxMs === undefined || gestoppt) return;
      clearTimeout(timer);
      timer = setTimeout(() => ablaufen(FRIST), ms);
    },
  };
}

/** Wartet auf `p`, höchstens `ms`; der Zeitgeber wird immer abgeräumt. */
async function warteMax(p: Promise<unknown>, ms: number): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    p,
    new Promise<void>((r) => {
      timer = setTimeout(r, ms);
    }),
  ]);
  clearTimeout(timer);
}
const MAX_UTTERANCE_FRAMES = 50 * 30;
/** Stumm geschaltet: das Mikrofon liefert der Erkennung nur Stille. */
const SILENT_FRAME = new Float32Array(FRAME_SAMPLES);
/** Typische Erkennungsfehler bei Stille/Rauschen; zählen nur bei kurzen Äußerungen. */
const PHANTOMS =
  /^(untertitel|vielen dank|danke(schön)?\.?|tschüss|bis zum nächsten mal|\[.*\]|\(.*\))/i;

/** Fehlertext eines abgebrochenen Starts (gestoppt, bevor er fertig war); kein echter Fehler. */
export const START_ABORTED = "abgebrochen";

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
  /** Höchstzahl Nachrichten, die an die KI gehen (Hauptgespräch: kleiner). */
  private historyLimit = MAX_HISTORY;
  /** Nummer des jüngsten `start()`. */
  private startGen = 0;
  private sentenceHandlers = new Map<string, (e: VoiceCallEvent) => void>();
  private muted = false;
  /** Laufende Text-Anfrage (Textmodus ohne Anruf). */
  private textRun: {
    id: string;
    finish: () => void;
    partial: string;
    side?: ChatSide;
    withContext?: boolean;
  } | null = null;
  /** Zählt Neustarts des Gesprächs: späte Antworten alter Anfragen werden verworfen. */
  private generation = 0;

  /** Orchestrator des Gesprächs (Werkzeuge, Modelle, Souveränität); je Engine einer. */
  private session: OrchestrationSession;
  /**
   * Eigene Denk-Seite statt Orchestrator (Buch-Gespräch, lva#309): Hören,
   * Sprechen, Turn-Taking, Barge-in und Verlauf bleiben die der Engine.
   */
  private sideOverride: ChatSide | null = null;
  /** Im Buch-Gespräch auf „allgemein“ gewechselt: die Buch-Seite ruht. */
  private generalMode = false;
  /**
   * Globaler Kontext aus allen freigegebenen Inhalten (lva#325). Nur aktiv mit
   * Schalter `voice_global_context_enabled`, ohne eigene Denk-Seite (im Buch
   * gilt das Buch-Gespräch) und nur mit LOKALEM Sprecher ohne Orchestrator:
   * Abrufkontext geht so nie an ein Cloud-Modell (Plan M6; die Freigabe je
   * Anbieter folgt mit P5).
   */
  private globalContext = new GlobalContext(
    <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
  );
  private speakerLocal = false;
  /**
   * Der Nutzer hat über die laufende Antwort hinweg zu sprechen begonnen (Pegel-Barge-in): die
   * Wiedergabe steht, die Erzeugung läuft weiter. Erst der erkannte Text entscheidet (lva#485).
   */
  private held: Turn | null = null;
  /** Erster Frame der Äußerung, die das Anhalten ausgelöst hat (Weitersprechen setzt dort fort). */
  private holdFrom = 0;
  /** Zählt Erkennungen im Anhalten: nur die jüngste darf entscheiden. */
  private holdRun = 0;
  /**
   * Zählt jede neue Lage: eine neue Äußerung (`onSpeechStart`), einen neuen Schritt (`newTurn`) und
   * jede Erkennung bei angehaltener Antwort (`decideHeld`). Ein spätes Erkennungsergebnis nach der
   * Frist gilt nur, wenn der Zähler seit dem Start seiner Erkennung gleich geblieben ist (lva#509).
   */
  private lage = 0;
  /** Eine Erkennung im Anhalten läuft. */
  private holdBusy = false;
  /**
   * Vortext einer Ergänzung: der Schritt dazu fiel, weil der Nutzer weitersprach; die nächste Erkennung
   * deckt nur das Audio ab dem Anhalten ab und bekommt ihn vorangestellt.
   */
  private carry: string | undefined;
  /** Hat dieser Anruf schon eine Modellantwort geliefert (E9: `warm`)? */
  private warm = false;
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
      ausgabeKey: () => this.konversation(),
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

  /** Denken aus einer eigenen Seite (z. B. „Buch“); `null` = wieder der Orchestrator. */
  setSideOverride(side: ChatSide | null): void {
    this.sideOverride = side;
    this.generalMode = false;
    // Seitenwechsel: vorbereiteter Kontext gilt nicht mehr (Plan M5).
    this.globalContext.clear();
  }

  /** Die Buch-Seite antwortet (gesetzt und nicht auf „allgemein“ gewechselt). */
  private get bookSide(): ChatSide | null {
    return this.generalMode ? null : this.sideOverride;
  }

  /** Markierung für Verlaufseinträge, die „allgemein“ im Buch entstehen. */
  private get historyTag(): { general?: boolean } {
    return this.sideOverride && this.generalMode ? { general: true } : {};
  }

  /** Gesprochener Wechsel: vorbereiteter Kontext gilt nicht mehr (Plan M5). */
  private setTalkMode(mode: TalkMode): void {
    this.generalMode = mode === "general";
    this.globalContext.clear();
    this.cb.onTalkMode?.(mode);
  }

  /** Schalter an und Bereiche frei (sonst kein Abruf, kein Vorwärmen; Plan M8). */
  private globalContextOn(): boolean {
    const s = useSettingsStore.getState().settings as
      | {
          voice_global_context_enabled?: boolean;
          voice_call_knowledge?: string[];
        }
      | null
      | undefined;
    return (
      !!s?.voice_global_context_enabled &&
      (s.voice_call_knowledge?.length ?? 0) > 0
    );
  }

  /** Letzte Nutzeräußerung (Rückbezug für den Abruf). */
  private lastUserText(): string | null {
    for (let i = this.history.length - 1; i >= 0; i--)
      if (this.history[i].role === "user") return this.history[i].content;
    return null;
  }

  /** Das Projekt des offenen Gespräches (`null`: gewöhnliches Gespräch). */
  private projekt(): ProjektBindung | null {
    return this.cb.projekt?.() ?? null;
  }

  /** Meldet einen Hinweis des Projektwegs (Ereignis `projekt` oder Code einer Ablehnung). */
  private meldeProjekt(h: ProjektHinweis | null): void {
    if (h) this.cb.onProjekt?.(h);
  }

  /** Kontext für einen Schritt; `null`, wenn er nicht gilt (siehe `globalContext`). */
  private async contextFor(
    turnId: string,
    text: string,
    side: ChatSide | null | undefined,
  ): Promise<PreparedContext | null> {
    // Im Projekt gilt nur der Kontext des Projekts, den das Tor in Rust selbst liest.
    if (this.projekt()) return null;
    if (side || this.bookSide || !this.speakerLocal) return null;
    if (!this.globalContextOn()) return null;
    const conversationId = (await this.cb.conversationId?.()) ?? null;
    const ctx =
      (await this.globalContext.take(turnId, text)) ??
      (await this.globalContext.prepare({
        turnId,
        transcript: text,
        previous: this.lastUserText(),
        conversationId,
      }));
    if (ctx)
      this.cb.onContextTiming?.(
        turnId,
        ctx.retrieveMs,
        ctx.promptMs,
        ctx.hits.hits.length,
      );
    return ctx;
  }

  private async reportCitations(
    turnId: string,
    answer: string,
    ctx: PreparedContext | null,
  ): Promise<void> {
    if (!ctx || ctx.hits.hits.length === 0) return;
    const ok = await this.globalContext.verify(answer, ctx);
    this.globalContext.discard(turnId);
    this.cb.onCitations?.(turnId, ok);
  }

  /**
   * Laufende Antwort sofort verwerfen, ohne den Anruf zu beenden: die Seite
   * wird abgebrochen, die Sprechwarteschlange geleert, vorsynthetisierte
   * Sätze fallen weg (Plan §3.7 Bereichswechsel, Esc).
   */
  interrupt(): void {
    this.cancelText(false);
    this.cancelTurn(false);
    if (!this.utt && this.inCall) this.setState("listening");
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
    this.carry = undefined;
    this.history = [];
    this.ausgabeVergessen();
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
    this.history = messages.slice(-this.historyLimit);
  }

  /**
   * Wie viele Nachrichten die KI vom Verlauf sieht; `undefined` = Vorgabe. Der Textverlauf
   * bleibt davon unberührt, nur der Kontext (und damit die Token je Antwort) wird kleiner.
   */
  setHistoryLimit(limit?: number): void {
    this.historyLimit = Math.max(
      2,
      Math.min(MAX_HISTORY, limit ?? MAX_HISTORY),
    );
    if (this.history.length > this.historyLimit * 2)
      this.history = this.history.slice(-this.historyLimit * 2);
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
      if (e.payload.kind === "revoked" && e.payload.request_id === "*") {
        this.onContextRevoked();
        return;
      }
      this.sentenceHandlers.get(e.payload.request_id)?.(e.payload);
    });
    await this.listening;
  }

  /**
   * Freigabe entzogen oder Schalter aus: laufende Antworten mit Kontext samt
   * Sprachausgabe beenden und kontextabgeleitete Antworten aus dem Verlauf
   * nehmen (Codex Auflagen 9/10).
   */
  private onContextRevoked(): void {
    if (this.turn?.withContext) this.cancelTurn(false);
    if (this.textRun?.withContext) this.cancelText(false);
    this.history = this.history.filter((m) => !m.ctx);
    this.globalContext.clear();
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
    // Jeder Start hat eine Nummer: Wird er gestoppt und gleich neu gestartet, solange
    // `voice_call_start` noch lädt, darf der alte nach dem Warten nichts mehr anlegen.
    const gen = ++this.startGen;
    const aborted = () => this.stopped || gen !== this.startGen;
    this.stopped = false;
    this.setState("starting");
    let ctx: AudioContext | null = null;
    try {
      const ready = await invoke<CallReady>("voice_call_start");
      if (aborted()) throw new Error(START_ABORTED);
      this.speakerLocal = ready.llm_local;
      this.warm = false;
      if (this.globalContextOn())
        void invoke("app_context_warm").catch(() => undefined);
      ctx = new AudioContext({ latencyHint: "interactive" });
      this.ctx = ctx;
      await ctx.resume();
      if (aborted()) throw new Error(START_ABORTED);
      this.player = new SpeechPlayer(ctx);
      this.stufenAnwenden();
      this.player.onIdle = () => {
        if (this.turn?.audible && this.turn.done) this.finishTurn();
      };
      await this.ensureListener();
      const capture = await startCapture(ctx, {
        onFrame: (f) => this.onFrame(f),
      });
      if (aborted()) {
        capture.stop();
        throw new Error(START_ABORTED);
      }
      this.capture = capture;
      this.setState("listening");
      return ready;
    } catch (e) {
      if (gen === this.startGen) await this.stop();
      else if (ctx) {
        // Ein neuerer Start läuft: nur das Eigene aufräumen, nicht seinen Zustand.
        if (this.ctx === ctx) this.ctx = null;
        await ctx.close().catch(() => undefined);
      }
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
    this.held = null;
    this.holdRun++;
    this.holdBusy = false;
    this.carry = undefined;
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
    // Angehaltene Wiedergabe (Ergänzen) zählt nicht als sprechende KI: der Nutzer hat das Wort.
    const aiSpeaking =
      !this.held && ((this.player?.active ?? false) || !!this.turn?.audible);
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
    this.lage++;
    let startFrame = Math.max(
      this.firstFrame,
      Math.round(at / 20) - PREROLL_FRAMES,
    );
    if (this.held) {
      // Weitersprechen bei angehaltener Antwort: eine frühere Erkennung ist überholt, die
      // Äußerung beginnt wieder am Anfang des Anhaltens (nichts geht verloren).
      this.holdRun++;
      this.holdBusy = false;
      startFrame = Math.min(startFrame, this.holdFrom);
    } else if (this.turn && !this.turn.audible) {
      startFrame = Math.min(startFrame, this.turn.startFrame);
      this.verwerfeBeimWeitersprechen();
    }
    this.utt = { startFrame, aiWasSpeaking: aiSpeaking, barged: false };
    if (!aiSpeaking) this.setState("hearing");
  }

  private onBargeIn(): void {
    if (this.utt) this.utt.barged = true;
    if (this.held) return;
    // Erst anhalten, dann entscheiden (lva#485): Rückmeldelaut („mhm“) oder Ergänzung brauchen den
    // Text, und der liegt erst nach der Erkennung vor. Die Erzeugung läuft weiter.
    const turn = this.turn;
    if (turn?.audible && this.player?.pause()) {
      this.held = turn;
      turn.heldSince = performance.now();
      this.holdFrom = this.utt?.startFrame ?? this.frameIndex;
      this.holdBusy = false;
      this.setState("hearing");
      return;
    }
    this.cancelTurn(true);
    this.setState("hearing");
  }

  private onResume(): void {
    if (this.held) {
      // Der Nutzer spricht noch: die laufende Erkennung ist überholt, die Antwort bleibt angehalten.
      this.holdRun++;
      this.holdBusy = false;
      this.setState("hearing");
      return;
    }
    this.verwerfeBeimWeitersprechen();
    this.setState("hearing");
  }

  /**
   * Der Nutzer spricht weiter, bevor die KI zu hören war: der Schritt fällt. Trug er schon eine
   * Ergänzung (`prefix`), bleibt deren Vortext für die nächste Erkennung erhalten.
   */
  private verwerfeBeimWeitersprechen(): void {
    const turn = this.turn;
    if (turn) this.carry = turn.prefix;
    this.cancelTurn(false);
  }

  /** Die Äußerung als Samples (mit Vorlauf und Nachlauf); `null`, wenn nichts da ist. */
  private sliceUtterance(
    utt: Utterance,
    lastSpeechAt: number,
  ): { samples: Float32Array; startFrame: number } | null {
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
    if (slice.length === 0) return null;
    const samples = new Float32Array(slice.length * FRAME_SAMPLES);
    slice.forEach((f, i) => samples.set(f, i * FRAME_SAMPLES));
    return { samples, startFrame };
  }

  private onEarlyEnd(lastSpeechAt: number, speechMs: number): void {
    const utt = this.utt;
    if (!utt) return;
    if (this.held) {
      // Zu kurz zum Erkennen: das Anhalten endet mit dem sicheren Satzende (`onEnd`).
      if (speechMs < this.cfg.turn.minUtteranceMs) return;
      const cut = this.sliceUtterance(utt, lastSpeechAt);
      if (cut)
        void this.decideHeld(
          cut.samples,
          cut.startFrame,
          this.wall(lastSpeechAt),
          speechMs,
        );
      return;
    }
    // Husten oder Nachhall, während die KI spricht: kein Unterbrechen, kein Turn.
    const aiStillSpeaking =
      (this.player?.active ?? false) || !!this.turn?.audible;
    if (utt.aiWasSpeaking && !utt.barged && aiStillSpeaking) return;
    if (speechMs < this.cfg.turn.minUtteranceMs) return;
    const cut = this.sliceUtterance(utt, lastSpeechAt);
    if (!cut) return;
    this.setState("thinking");
    void this.runTurn(
      cut.samples,
      cut.startFrame,
      this.wall(lastSpeechAt),
      speechMs,
    );
  }

  private onEnd(): void {
    const turn = this.turn;
    this.utt = null;
    // Angehalten, aber keine Erkennung gestartet (zu kurz, überholt): die Antwort läuft weiter.
    if (this.held && !this.holdBusy) {
      this.releaseHold();
      return;
    }
    if (!turn) {
      this.setState(this.player?.active ? "speaking" : "listening");
      return;
    }
    turn.committed = true;
    turn.commit();
  }

  // -- Ergänzen und Korrigieren während der Antwort (lva#485) ----------------------

  /** Was die reine Entscheidungslogik vom Schritt wissen muss. */
  private stand(turn: Turn): TurnStand {
    return {
      ausSprache: !!turn.fromSpeech,
      vorlage: !!turn.noMerge,
      ersterSatzFertig: !!turn.firstContentEnded,
      schonGespeichert: !!turn.finalSent,
    };
  }

  /** Die angehaltene Antwort läuft weiter (Rückmeldelaut, Rauschen, überholt). */
  private releaseHold(): void {
    const turn = this.held;
    if (!turn) return;
    this.held = null;
    this.holdRun++;
    this.holdBusy = false;
    this.zaehleHaltezeit(turn);
    this.player?.resume();
    this.setState(this.player?.active ? "speaking" : "listening");
  }

  /** Bisherige Haltezeit des Schritts (ms), samt einem noch laufenden Anhalten. */
  private haltezeit(turn: Turn): number {
    const laufend =
      turn.heldSince === undefined ? 0 : performance.now() - turn.heldSince;
    return (turn.heldMs ?? 0) + laufend;
  }

  /** Das Anhalten ist vorbei: seine Dauer zur Haltezeit des Schritts addieren. */
  private zaehleHaltezeit(turn: Turn): void {
    if (turn.heldSince === undefined) return;
    turn.heldMs = (turn.heldMs ?? 0) + (performance.now() - turn.heldSince);
    turn.heldSince = undefined;
  }

  /**
   * Was der Nutzer von diesem Schritt gehört hat: Fertiggespieltes plus das angespielte Häppchen.
   * Maßgeblich ist der tatsächliche Fortschritt der Wiedergabe (Startzeit der Quelle gegen die Uhr
   * des Audio-Kontexts), nicht das Einplanen: ein eingeplantes, noch stummes Häppchen zählt nicht.
   */
  private gehoert(turn: Turn): PieceRec[] {
    this.player?.syncStarts();
    const out: PieceRec[] = [];
    for (const p of turn.pieces) {
      if (!p.started) break;
      out.push(p);
      if (!p.ended) break;
    }
    return out;
  }

  /** Erkennt die Äußerung bei angehaltener Antwort und entscheidet: weiter, stopp, ergänzen, neu. */
  private async decideHeld(
    samples: Float32Array,
    startFrame: number,
    lastSpeechWall: number,
    speechMs: number,
  ): Promise<void> {
    const turn = this.held;
    if (!turn) return;
    const run = ++this.holdRun;
    const meineLage = ++this.lage;
    this.holdBusy = true;
    const t0 = performance.now();
    type Erg = { text: string } | { fehler: unknown };
    const ergebnis: Promise<Erg> = invoke<string>("voice_call_transcribe", {
      pcm16B64: floatToPcm16Base64(samples),
    }).then(
      (r): Erg => ({ text: typeof r === "string" ? r.trim() : "" }),
      (fehler): Erg => ({ fehler }),
    );
    let timer: ReturnType<typeof setTimeout> | undefined;
    const frist = new Promise<"frist">((r) => {
      timer = setTimeout(() => r("frist"), TUNING.holdSttMaxMs);
    });
    const erstes = await Promise.race([ergebnis, frist]);
    clearTimeout(timer);
    // Frist: die Wiedergabe läuft weiter, der Text wird trotzdem abgewartet und danach wie ein
    // spätes Unterbrechen behandelt (er geht nicht verloren).
    let spaet = false;
    let erg: Erg;
    if (erstes === "frist") {
      if (run === this.holdRun && this.held === turn) {
        this.holdBusy = false;
        this.releaseHold();
        spaet = true;
      }
      erg = await ergebnis;
      // Das späte Ergebnis gehört zur Lage von damals: hat sich seither etwas getan (neue Äußerung,
      // neuer Schritt, neue Erkennung), wird es verworfen und bricht nichts ab.
      if (this.lage !== meineLage) return;
    } else erg = erstes;

    if (!spaet) {
      // Überholt (der Nutzer sprach weiter) oder abgebrochen: nichts davon gilt mehr. Nur ein vom
      // Fehler beendeter Schritt (`dropTurn`) lässt die Äußerung als normalen Beitrag gelten.
      if (run !== this.holdRun) return;
      this.holdBusy = false;
    }
    if ("fehler" in erg) {
      if (!spaet && this.held === turn) this.releaseHold();
      this.cb.onError(i18n.t("voiceCall.error.notUnderstood"));
      return;
    }
    const text = erg.text;
    if (isPhantom(text, speechMs)) {
      if (!spaet && this.held === turn) this.releaseHold();
      return;
    }
    // Läuft der Schritt (noch), entscheidet die Logik. Ist er weg, ohne abgebrochen zu sein (Fehler,
    // zu Ende gelaufen), ist die Äußerung ein gewöhnlicher neuer Beitrag; sonst gilt sie nicht mehr.
    const laeuft = this.turn === turn && !turn.cancelled;
    if (!laeuft && (turn.cancelled || this.turn !== null)) return;
    const inhalt = (laeuft ? this.gehoert(turn) : []).filter(
      (p) => p.kind === "inhalt",
    );
    const satz = inhalt.length > 0 ? inhalt[inhalt.length - 1].text : "";
    // Nach der Frist ist die Antwort schon wieder zu hören: wie ein spätes Unterbrechen behandeln.
    const stand: TurnStand | null = laeuft
      ? {
          ...this.stand(turn),
          ersterSatzFertig: this.stand(turn).ersterSatzFertig || spaet,
        }
      : null;
    const d = entscheide(text, stand, turn.userText, satz);
    if (d.art === "weiter") {
      if (!spaet) this.releaseHold();
      return;
    }
    if (this.held === turn) {
      this.held = null;
      this.zaehleHaltezeit(turn);
      // Auch ohne laufenden Schritt: angehaltene Häppchen dürfen nie mehr gespielt werden.
      this.player?.stop();
    }
    if (d.art === "stopp") {
      this.cancelTurn(true);
      this.setState(this.utt ? "hearing" : "listening");
      return;
    }
    // Ergänzung: der alte Schritt verschwindet still; sonst: unterbrochen wie bisher.
    this.cancelTurn(
      d.art === "neu" && d.abbrechen,
      d.art === "zusammenfuehren",
    );
    const neuer = this.newTurn(startFrame, lastSpeechWall);
    neuer.fromSpeech = true;
    if (d.art === "zusammenfuehren") neuer.prefix = turn.userText;
    this.turn = neuer;
    // Das sichere Satzende kann schon vorbei sein: dann gibt es niemanden mehr, der freigibt.
    if (!this.utt) {
      neuer.committed = true;
      neuer.commit();
    }
    this.setState("thinking");
    await this.antworteSicher(neuer, d.text, performance.now() - t0);
  }

  private async antworteSicher(
    turn: Turn,
    text: string,
    sttMs: number,
  ): Promise<void> {
    try {
      await this.respond(turn, text, sttMs);
    } catch (e) {
      this.sentenceHandlers.delete(turn.id);
      if (!turn.cancelled && this.turn === turn) {
        this.cb.onError(String(e));
        this.dropTurn(turn);
      }
    }
  }

  /**
   * Meldet die fertige Antwortzeile. Solange der erste Satz noch läuft und der Schritt durch eine
   * Ergänzung ersetzt werden kann, wartet sie: eine gemeldete Zeile wird gespeichert, und ein
   * gespeicherter Beitrag lässt sich ohne Rust nicht mehr ersetzen.
   */
  private schliesseAntwort(turn: Turn, text: string): void {
    turn.finalText = text;
    if (darfZusammenfuehren(this.stand(turn))) return;
    this.sendeAntwort(turn);
  }

  private sendeAntwort(turn: Turn): void {
    if (turn.finalSent || turn.finalText === undefined || turn.cancelled)
      return;
    turn.finalSent = true;
    this.cb.onAssistant(turn.finalText, true, false);
  }

  /** Der erste Inhaltssatz ist zu Ende gesprochen: eine wartende Antwortzeile geht jetzt raus. */
  private ersterSatzFertig(turn: Turn): void {
    turn.firstContentEnded = true;
    this.sendeAntwort(turn);
  }

  /**
   * Ohne Satzzeichen schließt das letzte Stück der Antwort den Satz: ist die Antwort fertig erzeugt
   * und jedes Inhaltsstück gesprochen, ist der erste Satz zu Ende.
   */
  private pruefeSatzende(turn: Turn): void {
    if (turn.firstContentEnded || !turn.done) return;
    const inhalt = turn.pieces.filter((p) => p.kind === "inhalt");
    if (inhalt.length > 0 && inhalt.every((p) => p.ended))
      this.ersterSatzFertig(turn);
  }

  /** Hat die letzte Antwort im Verlauf der Nutzer unterbrochen? */
  private unterbrochen(): boolean {
    return !!this.history[this.history.length - 1]?.interrupted;
  }

  // -- Ein Gesprächsschritt ---------------------------------------------------

  private newTurn(startFrame: number, lastSpeechWall: number): Turn {
    let commit!: () => void;
    const committedP = new Promise<void>((r) => (commit = r));
    this.lage++;
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
      pieces: [],
      heard: 0,
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
    // Ein Schritt mit Vortext, der hier ersetzt wird, gibt ihn weiter.
    const alt = this.turn;
    if (alt && alt.prefix !== undefined) this.carry = alt.prefix;
    this.cancelTurn(false);
    const prefix = this.carry;
    this.carry = undefined;
    const turn = this.newTurn(startFrame, lastSpeechWall);
    turn.fromSpeech = true;
    turn.prefix = prefix;
    this.turn = turn;
    const dead = () => turn.cancelled || this.turn !== turn;

    try {
      const t0 = performance.now();
      const erkannt = (
        await invoke<string>("voice_call_transcribe", {
          pcm16B64: floatToPcm16Base64(samples),
        })
      ).trim();
      let text = prefix ? fuegeZusammen(prefix, erkannt) : erkannt;
      const sttMs = performance.now() - t0;
      if (dead()) return;
      if (isPhantom(erkannt, speechMs)) {
        // Nur Rauschen nach der Ergänzung: die Äußerung davor gilt allein.
        if (!prefix) {
          this.dropTurn(turn);
          return;
        }
        text = prefix;
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
    this.carry = undefined;
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
    // Im Projekt gibt es weder Denker noch Werkzeuge: der Sprecher antwortet allein, über das Tor.
    if (this.projekt())
      return {
        side: null,
        blocked: false,
        systemPrompt: this.cfg.systemPrompt,
      };
    if (this.bookSide)
      return {
        side: this.bookSide,
        blocked: false,
        systemPrompt: this.cfg.systemPrompt,
      };
    return this.session.prepare({
      ...this.cfg.orchestration,
      systemPrompt: this.cfg.systemPrompt,
    });
  }

  /** Verlauf plus neue Äußerung, ohne Systemanweisung (die hält der Orchestrator). */
  private historyFor(text: string): ChatMessage[] {
    // Seiten (Denker, Werkzeuge, Buch) können entfernt laufen: ohne
    // kontextabgeleitete Antworten (Codex M6).
    const hinweis = unterbrechungsHinweis(
      this.unterbrochen(),
      aktuelleSprache(),
    );
    return [
      ...(hinweis ? [{ role: "system" as const, content: hinweis }] : []),
      ...this.plainHistory(false),
      { role: "user", content: text },
    ];
  }

  /** Der Verlauf ohne die Kennungen: sie gehen nicht an das Modell. */
  private plainHistory(withDerived: boolean): ChatMessage[] {
    // Die Buch-Seite sieht nichts, was „allgemein“ entstand (Plan §3.7).
    const book = !!this.bookSide;
    return this.history
      .filter((m) => !(book && m.general) && (withDerived || !m.ctx))
      .slice(-this.historyLimit)
      .map(({ role, content }) => ({ role, content }));
  }

  /**
   * Nachrichten für `voice_call_chat`. Abgeleitete Antworten tragen
   * `derived`: das Backend gibt sie nur an einen lokalen Empfänger weiter.
   */
  private messagesFor(
    text: string,
    systemPrompt: string,
  ): (ChatMessage & { derived: boolean })[] {
    const book = !!this.bookSide;
    return [
      {
        role: "system",
        content: systemMitHinweis(
          systemPrompt,
          this.unterbrochen(),
          aktuelleSprache(),
        ),
        derived: false,
      },
      ...this.history
        .filter((m) => !(book && m.general))
        .slice(-this.historyLimit)
        .map(({ role, content, ctx }) => ({
          role,
          content,
          derived: !!ctx,
        })),
      { role: "user", content: text, derived: false },
    ];
  }

  /** Gespräch, dessen letzte Ausgabe das Tor in Rust gemerkt hat (`null`: noch keine). */
  private ausgabeKey: { id: string | null } | null = null;

  /**
   * Schlüssel des Gesprächs für den Speicher in Rust: einmal gewählt, bleibt er, solange das Gespräch
   * läuft (auch wenn die Kennung erst nach dem ersten Beitrag da ist), sonst fände „ja“ nach einer
   * Rückfrage und „wiederholen“ die letzte Ausgabe nicht.
   */
  private async konversation(): Promise<string | null> {
    if (this.ausgabeKey) return this.ausgabeKey.id;
    return (await this.cb.conversationId?.()) ?? null;
  }

  /** Stufen von Lautstärke und Tempo der Wiedergabe (lauter, leiser, schneller, langsamer). */
  private stufen: Stufen = START_STUFEN;

  private stufenAnwenden(): void {
    this.player?.setVolume(LAUTSTAERKE[this.stufen.lautstaerke]);
    this.player?.setRate(TEMPO[this.stufen.tempo]);
  }

  /**
   * Ohne Modell enden: S0-Antwort, Wiederholen, feste Meldung, Bedienung. `bereit`: die Bedienung
   * wirkt erst, wenn der Schritt hörbar wird (ein verworfener spekulativer Schritt ändert nichts).
   * `null`: nicht mehr nötig (abgebrochen).
   */
  private async vorlagenstuecke(
    plan: VorlagePlan,
    sprache: Sprache,
    bereit: () => Promise<boolean>,
  ): Promise<{ pieces: string[]; inhalt: boolean } | null> {
    switch (plan.kind) {
      case "antwort":
        return { pieces: [plan.text], inhalt: true };
      case "fehler":
        return { pieces: [plan.text], inhalt: false };
      case "wiederholen":
        return { pieces: await this.wiederholung(sprache), inhalt: false };
      case "bedienung": {
        if (!(await bereit())) return null;
        const r = bedienen(
          plan.art,
          this.stufen,
          this.player !== null && this.inCall,
          sprache,
        );
        this.stufen = r.stufen;
        if (r.geaendert) this.stufenAnwenden();
        return { pieces: [r.text], inhalt: false };
      }
    }
  }

  /**
   * Lokale Einstufung (ADR-0028 A2, E4) vor dem Modellaufruf. Das Tor in Rust gilt unabhängig
   * davon; hier entscheidet sich nur, was die Oberfläche daraus macht (Vorlage, Ansage, Meldung).
   */
  private async plan(text: string): Promise<{
    plan: AssistentPlan;
    stufe: StufeDto | null;
    sprache: Sprache;
  }> {
    const sprache = aktuelleSprache();
    const r = await einstufen(text, await this.konversation());
    return { plan: planAssistent(r, sprache), stufe: stufeVon(r), sprache };
  }

  /** „Wiederholen“: die letzte vom Tor freigegebene Ausgabe, sonst die Vorlage „nichts gesagt“. */
  private async wiederholung(sprache: Sprache): Promise<string[]> {
    const key = this.ausgabeKey;
    const again = key ? await letzteAusgabe(key.id) : null;
    const text = again ? speakable(again) : "";
    return text
      ? [vorlage("wiederhole", sprache), text]
      : [vorlage("nichts-zu-wiederholen", sprache)];
  }

  /** Nach einer Antwort, die nicht durchs Tor in Rust lief: keine ältere Ausgabe mehr wiederholen. */
  private ausgabeVergessen(): void {
    const key = this.ausgabeKey;
    this.ausgabeKey = null;
    if (key) void vergissAusgabe(key.id);
  }

  /** Merkmale der Messung (E9): Vorlagen sind immer warm und lokal, das Modell erst ab dem zweiten Schritt. */
  private messArt(modell: boolean): MessArt {
    if (!modell) return { warm: true, weg: "lokal" };
    return { warm: this.warm, weg: this.speakerLocal ? "lokal" : "relay" };
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
      const marken: Marken = { start: performance.now() };
      // Was das Modell geliefert hat (ohne Ansage und Vorlagen): Verlauf und „nichts geantwortet“.
      let modelPartial = "";
      let ansageText = "";
      let ansageGesprochen = false;
      const onPiece = (piece: string, kind: PieceKind = "inhalt") => {
        run.partial += (run.partial ? " " : "") + piece;
        this.cb.onAssistant(run.partial, false, false);
        const now = performance.now();
        if (kind === "ansage") {
          marken.ansage ??= now;
          ansageGesprochen = true;
          ansageText = piece;
          return;
        }
        marken.inhalt ??= now;
        if (kind === "inhalt")
          modelPartial += (modelPartial ? " " : "") + piece;
      };
      const { plan, stufe, sprache } = await this.plan(text);
      if (stale()) return;
      if (istVorlage(plan)) {
        // Vorlage, kein Modellaufruf. Nur die S0-Antwort gehört in den Verlauf.
        const v = await this.vorlagenstuecke(plan, sprache, async () => true);
        if (!v || stale()) return;
        const kind: PieceKind = v.inhalt ? "inhalt" : "vorlage";
        for (const p of v.pieces) onPiece(p, kind);
        marken.fertig = performance.now();
        // Im Projekt kennt das Antwortregister die Antwort der App nicht: sie bleibt aus dem Verlauf.
        if (kind === "inhalt" && !this.projekt()) {
          const tag = this.historyTag;
          this.history.push({
            role: "user",
            content: shown,
            turnId: id,
            ...tag,
          });
          this.history.push({
            role: "assistant",
            content: speakable(run.partial),
            ...tag,
          });
          if (stufe)
            meldeMessung(baueMessung(id, marken, stufe, this.messArt(false)));
        }
        this.cb.onAssistant(run.partial, true, false);
        return;
      }
      const { side, blocked, systemPrompt } = await this.orchestrate();
      if (stale() || blocked) return;
      // Ab S1: Ansage sofort, danach das Modell.
      // Im Projekt ohne Ansage: Die angezeigte Antwort muss dem freigegebenen Text gleichen.
      if (plan.kind === "ansage" && !this.projekt())
        onPiece(plan.text, "ansage");
      const gctx = await this.contextFor(id, text, side);
      if (stale()) return;
      const frist = ersteFrist(
        TUNING.modelFristMs,
        this.projekt() ? TUNING.projektObergrenzeMs : undefined,
      );
      let timedOut = false;
      // Das Tor im Projekt hat die Anfrage mit einem Code abgelehnt: Die Hinweisleiste sagt den Grund.
      let projektStopp = false;
      const onModel = (s: string) => {
        if (stale() || timedOut) return;
        // Nach einer Ansage keine zweite Überbrückung.
        if (ansageGesprochen && istUeberbrueckung(s)) return;
        frist.stop();
        onPiece(s);
      };
      let full = "";
      // Auch bei einem Rücksprung (veraltet, abgebrochen) die Zeitgeber der Frist abräumen.
      try {
        if (side) {
          run.side = side;
          if (!this.ausgabeKey) {
            const id = await this.konversation();
            if (stale()) return;
            this.ausgabeKey = { id };
          }
          const chat = side.chat(id, this.historyFor(text), onModel).then(
            (r) => (typeof r === "string" ? r : ""),
            () => "",
          );
          const r = await Promise.race([chat, frist.abgelaufen]);
          if (r === FRIST) {
            timedOut = true;
            void side.cancel?.(id);
          } else full = r;
          frist.stop();
          if (stale()) return;
        } else {
          const projekt = this.projekt();
          this.sentenceHandlers.set(id, (e) => {
            if (stale()) return;
            if (e.kind === "projekt") {
              this.meldeProjekt(hinweisAusEreignis(e));
              return;
            }
            if (e.kind === "laeuft") {
              // Lebenszeichen ohne Inhalt: das Tor puffert noch; die Frist läuft weiter.
              frist.lebenszeichen();
              return;
            }
            if (e.kind === "revoked") {
              // Freigabe entzogen: Antwort verwerfen (Codex Auflage 9).
              if (projekt) this.meldeProjekt({ code: "projekt.widerrufen" });
              this.cancelText(false);
              if (projekt) this.cb.onVerwerfen?.();
              return;
            }
            if (e.kind === "done" || e.kind === "failed") {
              if (e.usage) this.cb.onUsage?.(e.usage);
              finish();
            }
            if (e.kind !== "sentence") return;
            onModel(e.text ?? "");
          });
          // Im Projekt geht kein App-Kontext mit; die Kontextnachricht baut das Tor aus dem Projekt.
          const context = projekt ? null : speakerContext(gctx);
          const messages = this.messagesFor(text, systemPrompt);
          // Auch eine Antwort auf abgeleiteten Verlauf gilt als abgeleitet (nicht im Projekt: dort
          // prüft das Antwortregister in Rust, was vom Verlauf gilt).
          run.withContext =
            !projekt && (!!context || messages.some((m) => m.derived));
          const conversationId = await this.konversation();
          if (stale()) return;
          this.ausgabeKey = { id: conversationId };
          let abgelehnt = false;
          const chat = invoke<string>("voice_call_chat", {
            requestId: id,
            messages,
            context,
            conversationId,
            expectedStand: projekt?.stand ?? null,
          }).then(
            (r) => (typeof r === "string" ? r : ""),
            (e) => {
              abgelehnt = true;
              const h = projekt ? hinweisAusFehler(e) : null;
              projektStopp = h !== null;
              this.meldeProjekt(h);
              return "";
            },
          );
          const r = await Promise.race([chat, frist.abgelaufen]);
          if (r === FRIST) {
            timedOut = true;
            // Im Projekt: verwerfen über das Tor, keine Vorlagenzeile, der Hinweis sagt den Grund.
            if (projekt) {
              projektStopp = true;
              this.meldeProjekt({ code: "projekt.zeitueberschreitung" });
            }
            void invoke("voice_call_cancel", { requestId: id }).catch(
              () => undefined,
            );
          } else {
            full = r;
            if (!stale() && !abgelehnt) await warteMax(streamEnded, 3000);
          }
          frist.stop();
          if (stale()) return;
        }
      } finally {
        frist.stop();
      }
      // Mit Orchestrator zählt, was durch den Filter lief (die Rückgabe kann die Überbrückung enthalten).
      const rueckgabe = timedOut || istUeberbrueckung(full) ? "" : full.trim();
      const answer = side
        ? modelPartial || rueckgabe
        : rueckgabe || modelPartial;
      if (!answer) {
        // Im Projekt erklärt die Hinweisleiste, warum nichts gesendet wurde: keine zweite, unklare Zeile
        // („Das kann ich gerade nicht sicher beantworten“) und kein Eintrag dazu im Gespräch.
        if (projektStopp) {
          if (run.partial) this.cb.onAssistant(run.partial, true, false);
          return;
        }
        // Kein Modellinhalt (Fehler, Frist, leer): feste Vorlage, nicht in den Verlauf.
        onPiece(vorlage("fehler", sprache), "vorlage");
        this.cb.onAssistant(run.partial, true, false);
        return;
      }
      marken.fertig = performance.now();
      if (stufe)
        meldeMessung(baueMessung(id, marken, stufe, this.messArt(true)));
      this.warm = true;
      const tag = this.historyTag;
      this.history.push({ role: "user", content: shown, turnId: id, ...tag });
      this.history.push({
        role: "assistant",
        // Im Projekt genau der Text, den das Tor freigegeben hat: Das Antwortregister erkennt ihn
        // am Fingerabdruck wieder, `speakable` würde die Marken entfernen.
        content: this.projekt() ? answer : speakable(answer),
        ...tag,
        ...(run.withContext ? { ctx: true } : {}),
      });
      // Die Ansage gehört zur angezeigten Antwort, nicht zur Prüfung der Belege.
      this.cb.onAssistant(
        ansageText ? `${ansageText} ${answer}` : answer,
        true,
        false,
      );
      void this.reportCitations(id, answer, gctx);
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
    // Was das Modell geliefert hat (ohne Ansage und Vorlagen): Verlauf, Belege, „nichts geantwortet“.
    let modelText = "";
    let ansageGesprochen = false;
    // ADR-0028 E9: Zeitmarken dieses Schritts (ohne Inhalte).
    const marken: Marken = { start: turn.lastSpeechWall };
    let metricsSent = false;
    // Kontext dieses Schritts (für die Messung; gesetzt nach dem Abruf).
    let turnCtx: PreparedContext | null = null;

    let finishStream!: () => void;
    const streamEnded = new Promise<void>((r) => (finishStream = r));
    turn.finishStream = finishStream;
    const emitMetrics = (at: number) => {
      if (metricsSent) return;
      metricsSent = true;
      turn.metricsSent = true;
      // Die Haltezeit (Barge-in vor dem ersten Ton) gehört nicht zur Zeit bis zum ersten Ton.
      this.cb.onMetrics({
        turnId: turn.id,
        sttMs: Math.round(sttMs),
        llmFirstMs: Math.round(firstSentenceAt - tChat),
        ttsFirstMs: Math.round(firstAudioAt - firstSentenceAt),
        firstSoundMs: Math.round(
          at - turn.lastSpeechWall - this.haltezeit(turn),
        ),
        ...(turnCtx
          ? {
              contextMs: turnCtx.promptMs,
              contextHits: turnCtx.hits.hits.length,
            }
          : {}),
      });
    };
    // `kind`: Inhalt (Modell, S0-Antwort), Ansage ab S1 oder feste Vorlage. Nur Inhalt kommt in den Verlauf.
    const onPiece = (piece: string, kind: PieceKind = "inhalt") => {
      const ansage = kind === "ansage";
      if (!ansage && !firstSentenceAt) firstSentenceAt = performance.now();
      if (ansage) ansageGesprochen = true;
      if (kind === "inhalt") modelText += (modelText ? " " : "") + piece;
      assistantText += (assistantText ? " " : "") + piece;
      this.cb.onAssistant(assistantText, false, false);
      chain = chain.then(async () => {
        if (dead()) return;
        let wav: string;
        try {
          const spoken = speakable(piece);
          if (!spoken.trim()) return;
          wav = await invoke<string>("voice_call_synthesize", {
            text: spoken,
            requestId: turn.id,
          });
        } catch (err) {
          if (!dead()) this.cb.onError(String(err));
          return;
        }
        if (dead()) return;
        if (!ansage && !firstAudioAt) firstAudioAt = performance.now();
        await turn.committedP;
        if (dead()) return;
        if (!turn.audible) {
          turn.audible = true;
          this.player?.rearm();
          this.player!.onFirstSound = (at) => {
            this.setState("speaking");
            if (ansage) {
              // Die Metriken folgen mit dem ersten Satz danach.
              marken.ansage = at;
              return;
            }
            marken.inhalt = at;
            emitMetrics(at);
          };
        } else if (!ansage) {
          // Die Ansage läuft schon: Zeit bis der erste Satz danach hörbar bereit ist.
          marken.inhalt ??= performance.now();
          emitMetrics(performance.now());
        }
        turn.heard++;
        const rec: PieceRec = {
          text: piece,
          kind,
          ended: false,
          queued: false,
          started: false,
        };
        turn.pieces.push(rec);
        // Das Ende des Stücks, das den ersten Inhaltssatz schließt, entscheidet, ob eine Ergänzung
        // noch zusammenführt. Ein Stück ist nicht unbedingt ein Satz („Für morgen,“).
        await this.player!.enqueue(
          wav,
          () => {
            rec.ended = true;
            if (kind === "inhalt" && schliesstSatz(piece))
              this.ersterSatzFertig(turn);
            else this.pruefeSatzende(turn);
          },
          () => {
            rec.started = true;
          },
        );
        rec.queued = true;
      });
    };
    // E9: `fertig_ms` erst, wenn der Schritt zu Ende gesprochen ist (`finishTurn`).
    const messenBeiEnde = (stufe: StufeDto | null, modell: boolean) => {
      if (!stufe) return;
      turn.onEnde = () => {
        marken.fertig = performance.now() - this.haltezeit(turn);
        meldeMessung(baueMessung(turn.id, marken, stufe, this.messArt(modell)));
        if (modell) this.warm = true;
      };
    };

    // Buch-Gespräch: „allgemein“ / „im Buch“ wechselt den Bereich. Der
    // Wechsel gilt erst, wenn der Schritt hörbar wird (nicht spekulativ), und
    // landet nicht im Verlauf.
    const sw = this.sideOverride ? modeSwitch(text) : null;
    if (sw) {
      onPiece(modeSwitchReply(sw.mode, sw.german));
      await chain;
      this.sentenceHandlers.delete(turn.id);
      if (dead()) return;
      this.setTalkMode(sw.mode);
      turn.done = true;
      turn.noMerge = true;
      this.schliesseAntwort(turn, assistantText);
      if (!this.player?.active) this.finishTurn();
      return;
    }

    // ADR-0028 A2: lokale Einstufung vor dem Modellaufruf.
    const { plan, stufe, sprache } = await this.plan(text);
    if (dead()) return;
    if (istVorlage(plan)) {
      // Vorlage, kein Modellaufruf. Nur die S0-Antwort gehört in den Verlauf.
      const v = await this.vorlagenstuecke(plan, sprache, async () => {
        await turn.committedP;
        return !dead();
      });
      if (!v || dead()) return;
      const kind: PieceKind = v.inhalt ? "inhalt" : "vorlage";
      turn.noHistory = kind === "vorlage";
      turn.noMerge = true;
      for (const p of v.pieces) onPiece(p, kind);
      await chain;
      this.sentenceHandlers.delete(turn.id);
      if (dead()) return;
      turn.done = true;
      this.pruefeSatzende(turn);
      if (turn.heard === 0) {
        this.dropTurn(turn);
        return;
      }
      // Im Projekt kennt das Antwortregister die Antwort der App nicht: sie bleibt aus dem Verlauf.
      if (kind === "inhalt" && !this.projekt()) {
        const tag = this.historyTag;
        this.history.push({
          role: "user",
          content: turn.userText,
          turnId: turn.id,
          ...tag,
        });
        this.history.push({
          role: "assistant",
          content: speakable(assistantText),
          ...tag,
        });
        turn.historyPushed = true;
        messenBeiEnde(stufe, false);
      }
      this.schliesseAntwort(turn, assistantText);
      if (!this.player?.active) this.finishTurn();
      return;
    }

    const { side, blocked, systemPrompt } = await this.orchestrate();
    if (dead()) return;
    // Ab S1: Ansage sofort (vor dem Abruf des Kontexts), danach das Modell.
    // Im Projekt ohne Ansage: Die angezeigte Antwort muss dem freigegebenen Text gleichen.
    if (plan.kind === "ansage" && !blocked && !this.projekt())
      onPiece(plan.text, "ansage");
    // Globaler Kontext: lokal geholt, schon beim spekulativen Start (Plan L1/M5).
    const gctx = blocked ? null : await this.contextFor(turn.id, text, side);
    turnCtx = gctx;
    if (dead()) {
      this.globalContext.discard(turn.id);
      return;
    }
    if (blocked) {
      // Kein Gespräch mit einem gesperrten Modell: der Hinweis steht im Gespräch.
      this.dropTurn(turn);
      return;
    }
    const frist = ersteFrist(
      TUNING.modelFristMs,
      this.projekt() ? TUNING.projektObergrenzeMs : undefined,
    );
    let timedOut = false;
    // Das Tor im Projekt hat die Anfrage mit einem Code abgelehnt: Die Hinweisleiste sagt den Grund.
    let projektStopp = false;
    const onModel = (s: string) => {
      if (dead() || timedOut) return;
      // Nach einer Ansage keine zweite Überbrückung (Orchestrator, Buch-Seite).
      if (ansageGesprochen && istUeberbrueckung(s)) return;
      frist.stop();
      onPiece(s);
    };
    // Auch bei einem Rücksprung (veraltet, abgebrochen) die Zeitgeber der Frist abräumen.
    try {
      if (side) {
        // Denker oder Werkzeuge aktiv: der Orchestrator liefert die Sätze (auch die Überbrückung).
        turn.side = side;
        if (!this.ausgabeKey) {
          const id = await this.konversation();
          if (dead()) return;
          this.ausgabeKey = { id };
        }
        const chat = side.chat(turn.id, this.historyFor(text), onModel).then(
          () => "",
          () => "",
        );
        if ((await Promise.race([chat, frist.abgelaufen])) === FRIST) {
          timedOut = true;
          void side.cancel?.(turn.id);
        }
        frist.stop();
      } else {
        // Mit App-Kontext startet das Modell erst nach dem Commit, und der
        // Kontext wird davor erneut geprüft (Turn, Transkript, Stand; Codex
        // M5, Auflagen 6–8). Ohne Kontext bleibt der spekulative Start.
        const projekt = this.projekt();
        let context = projekt ? null : speakerContext(gctx);
        if (context) {
          await turn.committedP;
          if (dead()) {
            this.globalContext.discard(turn.id);
            return;
          }
          const fresh = await this.globalContext.take(turn.id, text);
          if (dead()) return;
          turnCtx = fresh;
          context = speakerContext(fresh);
        }
        const messages = this.messagesFor(text, systemPrompt);
        // Auch eine Antwort auf abgeleiteten Verlauf gilt als abgeleitet
        // (Widerruf, Route; Codex M5 Must-fix 1).
        turn.withContext =
          !projekt && (!!context || messages.some((m) => m.derived));
        this.sentenceHandlers.set(turn.id, (e) => {
          if (e.kind === "projekt") {
            this.meldeProjekt(hinweisAusEreignis(e));
            return;
          }
          if (e.kind === "laeuft") {
            // Lebenszeichen ohne Inhalt: das Tor puffert noch; die Frist läuft weiter.
            frist.lebenszeichen();
            return;
          }
          if (e.kind === "revoked") {
            // Freigabe entzogen: Antwort und Sprachausgabe sofort beenden.
            if (projekt) this.meldeProjekt({ code: "projekt.widerrufen" });
            finishStream();
            if (this.turn === turn) this.cancelTurn(false);
            if (projekt) this.cb.onVerwerfen?.();
            return;
          }
          if ((e.kind === "done" || e.kind === "failed") && e.usage)
            this.cb.onUsage?.(e.usage);
          if (e.kind === "done" || e.kind === "failed") finishStream();
          if (e.kind !== "sentence" || dead()) return;
          onModel(e.text ?? "");
        });
        const conversationId = await this.konversation();
        if (dead()) return;
        this.ausgabeKey = { id: conversationId };
        let abgelehnt = false;
        const chat = invoke<string>("voice_call_chat", {
          requestId: turn.id,
          messages,
          context,
          conversationId,
          expectedStand: projekt?.stand ?? null,
        }).then(
          (r) => {
            // Im Projekt ist die Rückgabe der Text, den das Tor freigegeben hat; nur er kommt in den
            // Verlauf (leer: abgebrochen, widerrufen oder verspätet).
            if (projekt && typeof r === "string") turn.projektText = r.trim();
            return "";
          },
          (e) => {
            abgelehnt = true;
            const h = projekt ? hinweisAusFehler(e) : null;
            projektStopp = h !== null;
            this.meldeProjekt(h);
            return "";
          },
        );
        if ((await Promise.race([chat, frist.abgelaufen])) === FRIST) {
          timedOut = true;
          // Im Projekt: verwerfen über das Tor, keine Vorlagenzeile, der Hinweis sagt den Grund.
          if (projekt) {
            projektStopp = true;
            this.meldeProjekt({ code: "projekt.zeitueberschreitung" });
          }
          finishStream();
          void invoke("voice_call_cancel", { requestId: turn.id }).catch(
            () => undefined,
          );
        } else if (!dead() && !abgelehnt) {
          // Das letzte Häppchen kann nach dem Rückgabewert eintreffen: auf `done` warten.
          await warteMax(streamEnded, 3000);
        }
        frist.stop();
      }
    } finally {
      frist.stop();
    }
    await chain;
    this.cb.onStatus?.(null);
    this.sentenceHandlers.delete(turn.id);
    if (dead()) return;
    if (!modelText && projektStopp) {
      // Im Projekt erklärt die Hinweisleiste, warum nichts gesendet wurde: nichts sprechen, keine zweite
      // Zeile („Das kann ich gerade nicht sicher beantworten“), kein Eintrag im Gespräch.
      turn.noHistory = true;
      turn.done = true;
      this.dropTurn(turn);
      return;
    }
    if (!modelText) {
      // Kein Modellinhalt (Fehler, Frist, leer): feste Vorlage, nicht in den Verlauf.
      turn.noHistory = true;
      turn.noMerge = true;
      onPiece(vorlage("fehler", sprache), "vorlage");
      await chain;
      if (dead()) return;
    }
    turn.done = true;
    this.pruefeSatzende(turn);
    if (turn.heard === 0) {
      this.dropTurn(turn);
      return;
    }
    if (turn.noHistory) {
      this.schliesseAntwort(turn, assistantText);
      if (!this.player?.active) this.finishTurn();
      return;
    }
    const tag = this.historyTag;
    // Im Projekt zählt nur der freigegebene Text aus der Rückgabe; fehlt er, bleibt der Schritt aus
    // dem Verlauf (das Register kennt dann keine Antwort, und sie wäre ohnehin ersetzt worden).
    const verlaufText = this.projekt()
      ? (turn.projektText ?? "")
      : speakable(modelText);
    if (verlaufText) {
      this.history.push({
        role: "user",
        content: turn.userText,
        turnId: turn.id,
        ...tag,
      });
      // Der Verlauf behält, was das Modell gesagt hat, nicht die Ansage.
      this.history.push({
        role: "assistant",
        content: verlaufText,
        ...tag,
        ...(turn.withContext ? { ctx: true } : {}),
      });
      turn.historyPushed = true;
    }
    messenBeiEnde(stufe, true);
    this.schliesseAntwort(turn, assistantText);
    // Die Belege prüfen nur, was das Modell gesagt hat, nicht die Ansage.
    void this.reportCitations(turn.id, modelText, turnCtx);
    // Ende der Wiedergabe meldet player.onIdle -> finishTurn.
    if (!this.player?.active) this.finishTurn();
  }

  private dropTurn(turn: Turn): void {
    if (this.turn === turn) this.turn = null;
    if (this.held === turn) {
      // Der Schritt endet ohne Antwort, solange die Wiedergabe angehalten ist: die wartenden Häppchen
      // dürfen nie mehr gespielt werden. Eine laufende Erkennung bleibt gültig (die Äußerung wird ein
      // gewöhnlicher Beitrag, `decideHeld`).
      this.held = null;
      this.player?.stop();
    }
    this.sentenceHandlers.delete(turn.id);
    if (!this.utt) this.setState("listening");
  }

  private finishTurn(): void {
    // Eine angehaltene Antwort endet nicht von selbst: erst die Entscheidung (`decideHeld`).
    if (this.held && this.held === this.turn) return;
    const turn = this.turn;
    const ende = turn?.onEnde;
    this.turn = null;
    if (turn) this.sendeAntwort(turn);
    ende?.();
    if (!this.utt) this.setState("listening");
  }

  /**
   * Laufenden Schritt verwerfen; `interrupted`: der Nutzer hat die KI unterbrochen. `merge`: die
   * Äußerung wird mit einer Ergänzung neu gestellt, der Schritt hinterlässt weder Verlauf noch Zeile.
   */
  private cancelTurn(interrupted: boolean, merge = false): void {
    const turn = this.turn;
    if (!turn) return;
    turn.cancelled = true;
    turn.finishStream?.();
    this.turn = null;
    if (this.held === turn) {
      this.held = null;
      this.holdRun++;
      this.holdBusy = false;
    }
    this.globalContext.discard(turn.id);
    this.sentenceHandlers.delete(turn.id);
    void turn.side?.cancel?.(turn.id);
    void invoke("voice_call_cancel", { requestId: turn.id }).catch(
      () => undefined,
    );
    this.cb.onStatus?.(null);
    turn.commit();
    const said = this.gehoert(turn)
      .filter((p) => p.kind === "inhalt")
      .map((p) => p.text);
    if (merge || (interrupted && said.length === 0 && turn.historyPushed)) {
      // Nur der Verlauf der KI: Anzeige und Speicher ersetzt `onUser` der zusammengeführten Äußerung.
      const i = this.history.findIndex(
        (m) => m.role === "user" && m.turnId === turn.id,
      );
      if (i >= 0) this.history.splice(i, 2);
    }
    if (merge) {
      // Die Messung eines ersetzten Schritts gilt nicht; „wiederholen“ kennt seine Antwort nicht mehr.
      if (turn.metricsSent) this.cb.onMetricsVerwerfen?.(turn.id);
      this.ausgabeVergessen();
    } else if (interrupted && said.length > 0 && !turn.noHistory) {
      // Nur das tatsächlich Gehörte gehört in den Verlauf: Fertiggespieltes plus das angespielte Häppchen.
      if (turn.historyPushed) this.history.splice(-2, 2);
      const heard = said.join(" ");
      const tag = this.historyTag;
      this.history.push({
        role: "user",
        content: turn.userText,
        turnId: turn.id,
        ...tag,
      });
      this.history.push({
        role: "assistant",
        content: `${speakable(heard)} …`,
        ...tag,
        interrupted: true,
        ...(turn.withContext ? { ctx: true } : {}),
      });
      this.cb.onAssistant(heard, true, true);
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
