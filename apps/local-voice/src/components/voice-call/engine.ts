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
import { istUeberbrueckung, ohneUeberbrueckungen } from "./ueberbrueckung";
import {
  auftragsBlock,
  liesSofort,
  sofortPrompt,
  SOFORT_MAX_MS,
  SOFORT_VERLAUF,
  wendeAn,
  wendeAuftragAn,
  type SofortEntscheid,
  type SofortZustand,
} from "./sofort";
import type { ModelMessage, VoiceModel } from "@/vendor/wai-voice/model.ts";
import {
  darfZusammenfuehren,
  entscheide,
  fuegeZusammen,
  istStoppwort,
  schliesstSatz,
  systemMitHinweis,
  unterbrechungsHinweis,
  type Entscheidung,
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

/** Ereignis der Denk-Anzeige; Ansagen und Überbrückungen landen hier statt in Chat und Stimme. */
export type DenkEreignis =
  | { art: "start"; turnId: string }
  /** `turnId`: Schritt, zu dem der Text gehört (ein offener Auftrag hat eine eigene Anzeige). */
  | { art: "schritt"; text: string; turnId?: string }
  /** Der Sofort-Sprecher (lokales Modell) wurde gefragt. */
  | { art: "sofort"; modell: string }
  /** Der nächste Text ist der Sofort-Sprecher, der weiterspricht: die Anzeige läuft danach weiter. */
  | { art: "zwischen" }
  /** Gestreamter Denktext des Modells (Reasoning): nur Anzeige, nie Stimme oder Verlauf. */
  | { art: "gedanke"; text: string; turnId?: string }
  /** Der Schritt endet ohne Antwort (Abbruch, Unterbrechung, verworfen): die Anzeige verschwindet. */
  | { art: "ende"; turnId: string }
  /**
   * Der Denker-Lauf des Schritts läuft als offener Auftrag im Hintergrund weiter (der Vordergrund endete):
   * seine Anzeige wechselt in den Hintergrund, `frage` ist die ursprüngliche Frage.
   */
  | { art: "auftrag"; turnId: string; frage: string }
  /** Der Auftrag wird nachgereicht: seine Anzeige ist wieder die des Vordergrunds. */
  | { art: "vordergrund"; turnId: string };

/** Laufender Sofort-Sprecher eines Schritts (`sofort.ts`). */
interface SofortLauf {
  zustand(): SofortZustand;
  /** Erster Inhalt des Denkers: wartet der Sprecher noch, verliert er. */
  denkerKommt(): void;
  /** Abbrechen ohne Entscheidung (Schritt verworfen). */
  stop(): void;
  /** Erfüllt sich (mit leerem Text), wenn der Sprecher allein geantwortet hat: niemand wartet auf den Denker. */
  gewonnen: Promise<string>;
  /** Der Sprecher hat etwas gesagt (Antwort oder Weitersprechen). */
  gesprochen(): boolean;
  /** Erfüllt sich, sobald der Sprecher entschieden hat (höchstens `SOFORT_MAX_MS`). */
  abwarten(): Promise<void>;
  /** Der Sprecher hat „WEITER“ gesagt (auch ohne Text) oder bei offenem Auftrag neu suchen lassen. */
  weiter(): boolean;
}

/**
 * Offener Auftrag (ADR-0028 E4.2c 7(e)): der Denker-Lauf eines Schritts, dessen Vordergrund endete, weil der
 * Nutzer weitersprach, bevor der Denker etwas Hörbares lieferte. Seine Sätze werden gepuffert und nie
 * direkt gespielt; bei freiem Vordergrund werden sie als eigene Einheit nachgereicht.
 */
interface Auftrag {
  /** Schritt, dessen Denker-Lauf weiterläuft; seine Kennung ist die der Denker-Anfrage. */
  turn: Turn;
  turnId: string;
  /** Die ursprüngliche Frage (so, wie sie im Gespräch steht). */
  frage: string;
  /** Das Weitersprechen des Sprechers, soweit es zu hören war (für den Verlauf bei „SUCHE“). */
  gesagt: string;
  /** Gepufferte Sätze des Denkers, ohne Überbrückungen. */
  saetze: string[];
  zeichen: number;
  /** Sätze, die der Deckel des Puffers verworfen hat (gezählt, nicht verschluckt). */
  ueberlauf: number;
  /** Der Denker ist zu Ende und hat geliefert. */
  fertig: boolean;
  /** Beginn des Denker-Laufs (`performance.now`). */
  gestartet: number;
  /**
   * Frist `TUNING.auftragMaxMs` ab `gestartet`: gilt bis das Ergebnis zu hören beginnt, auch für ein
   * schon fertiges, aber noch nicht nachgereichtes.
   */
  timer: ReturnType<typeof setTimeout> | undefined;
  /**
   * Das Gedächtnis des Tors in Rust (`assistent_letzte_ausgabe`) gehört dem Auftrag, solange er offen ist:
   * dort steht (nach seinem Ende) sein Ergebnis. Bis es gesprochen ist, gibt „wiederholen“ nichts aus, und eine
   * Antwort des Sprechers vergisst es nicht; danach übernimmt die Engine den Schlüssel zurück.
   */
  ausgabe: { id: string | null } | null;
  /** Verworfen oder verbraucht: nichts davon darf noch wirken. */
  verworfen: boolean;
  /** Ein Schritt spricht das Ergebnis gerade (`nachreichen`). */
  nachreichend: boolean;
}

export interface CallCallbacks {
  onState(state: CallState): void;
  onLevel(level: number): void;
  /** `turnId`: Kennung der Äußerung, damit eine spätere Korrektur genau sie trifft. */
  onUser(text: string, turnId: string): void;
  /**
   * `auftragVon`: die Zeile ist das Ergebnis eines Auftrags (Kennung des Auftrags); alle Zeilen dieses
   * Nachreichens tragen sie, damit spätere Meldungen (Quellen) genau diese Zeile treffen.
   */
  onAssistant(
    text: string,
    final: boolean,
    interrupted: boolean,
    auftragVon?: string,
  ): void;
  onMetrics(m: TurnMetrics): void;
  /** Der Schritt wurde durch eine Ergänzung ersetzt: seine Messung gilt nicht (lva#485). */
  onMetricsVerwerfen?(turnId: string): void;
  onError(message: string): void;
  onUsage?(usage: CallUsageEvent): void;
  /**
   * Werkzeug läuft / Hinweis des Orchestrators; `null` löscht die Werkzeugzeile. `auftrag`: das Werkzeug
   * gehört zum Denker eines offenen Auftrags im Hintergrund (nur ein Schritt seiner Anzeige).
   */
  onStatus?(status: CallStatus | null, auftrag?: boolean): void;
  /**
   * Denk-Anzeige (Patrick 12.11.2026): `start` nach der Äußerung, `schritt` je Zwischenschritt
   * (Ansage, Überbrückung, Werkzeug). Gesprochen und in den Chat geschrieben wird davon nichts.
   */
  onDenken?(e: DenkEreignis): void;
  /** Das laufende gespeicherte Gespräch (für die Wissenswerkzeuge); `null` bis zum ersten Beitrag; wartet auf ein laufendes Speichern. */
  conversationId?(): string | null | Promise<string | null>;
  /** Globaler Sprachassistent (lva#325): geprüfte Belege einer Antwort. */
  /** `nachgereicht`: die Quellen gehören zur Zeile des Auftrags (`onAssistant`: `auftragVon` = `turnId`), nicht zur ersten nach der Frage. */
  onCitations?(
    turnId: string,
    hits: ContextHit[],
    nachgereicht?: boolean,
  ): void;
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
  /** Bricht einen noch wartenden Sofort-Sprecher ab. */
  sofortStop?: () => void;
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
  /** Der Sofort-Sprecher dieses Schritts (nur wenn er läuft). */
  sprecher?: SofortLauf;
  /** Beginn des Denker-Laufs (`performance.now`); fehlt, solange kein Denker gefragt ist. */
  denkerStart?: number;
  /** Alle Sätze des Denkers (ohne Überbrückungen), auch die noch nicht hörbaren. */
  denkerSaetze?: string[];
  /** Der Denker-Teil von `respond` ist zu Ende. */
  denkerFertig?: boolean;
  /** Der Denker ist gescheitert (Fehler, Abweisung): ein Auftrag wird still verworfen. */
  denkerFehler?: boolean;
  /** Hält die Frist des Denkers an (beim Übergang zum Auftrag gilt `TUNING.auftragMaxMs`). */
  denkerFristStop?: () => void;
  /** Das Weitersprechen des Sprechers (gesprochen und angezeigt, keine Antwort). */
  sofortText?: string;
  /** Der Schritt hat schon Text in den Chat gemeldet (`onAssistant`). */
  angezeigt?: boolean;
  /** Der Denker-Lauf dieses Schritts läuft als offener Auftrag weiter; der Vordergrund ist beendet. */
  auftrag?: Auftrag;
  /** Dieser Schritt spricht das Ergebnis eines Auftrags (eigene Einheit, nie mit Nutzersprache zusammengeführt). */
  nachreichen?: Auftrag;
  /** Der Sprecher hat den offenen Auftrag mit „ABBRUCH“ beendet: ohne Bestätigung bleibt der Schritt stumm. */
  auftragAbbruch?: boolean;
  /**
   * Kontext des Schritts (Auszüge für die Belege). Ein Auftrag meldet damit beim Nachreichen die Quellen
   * seines Ergebnisses, wie ein gewöhnlicher Abschluss.
   */
  kontext?: PreparedContext | null;
  /**
   * Meldung der Quellen, die noch wartet: bei einem Schritt, der noch zum Auftrag werden kann (Sprecher sagte
   * „WEITER“, vom Denker ist nichts zu hören), erst wenn sein Inhalt zu hören beginnt. Wird er zum Auftrag,
   * meldet das Nachreichen die Quellen.
   */
  quellenMelden?: () => void;
  /**
   * Stücke, die schon als Satz vorliegen, aber noch nicht im Player sind (Synthese läuft oder steht hinter einer
   * anderen an). Nur solange etwas offen ist, kann der Player auf ein Stück warten, das ausbleibt (`stueckFrist`);
   * wartet der Schritt auf das Modell, ist nichts offen, und die Fristen des Modells gelten.
   */
  synthOffen: number;
  /**
   * Wartezeit auf das nächste Stück (`warteAufStueck`), seit der Player leer lief, obwohl noch ein Stück offen ist.
   * Läuft sie ab, endet der Schritt wie bei einem Abbruch.
   */
  stueckTimer?: ReturnType<typeof setTimeout>;
  /**
   * Die Wartegrenze (`stueckFrist`) hat den Schritt beendet, nicht der Nutzer. Eine Erkennung, die danach noch
   * eintrifft, bleibt gültig (`aeusserungGilt`): ein Timeout überholt keine Äußerung.
   */
  abgelaufen?: boolean;
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
  /**
   * Nach einer Überbrückung (Werkzeug oder Denker arbeitet) gilt diese längere Frist bis zum ersten
   * Inhalt; sie wird nie ausgeschaltet (ms).
   */
  denkerFristMs: 180_000,
  /** So lange darf die Erkennung einer Äußerung dauern, während die Wiedergabe angehalten ist (ms). */
  holdSttMaxMs: 8_000,
  /** Ein offener Auftrag wird verworfen, wenn der Denker nach dieser Zeit (ab seinem Start) nichts Fertiges hat (ms). */
  auftragMaxMs: 60_000,
  /** Mindeststille vor dem Nachreichen: seit dem letzten Sprachende des Nutzers und dem Ende des Vordergrunds (ms). */
  nachreichenStilleMs: 600,
  /** Deckel für den Satzpuffer eines Auftrags (Zeichen); was darüber liegt, wird gezählt und verworfen. */
  auftragMaxZeichen: 20_000,
  /**
   * So lange darf die Synthese eines schon vorliegenden Stücks ausbleiben, nachdem der Player leer lief (ms), beim
   * gewöhnlichen Schritt wie beim Nachreichen: dieselbe Störung (die Synthese antwortet nicht), dieselbe Lücke im Ton,
   * darum ein Wert. Was schon klingt, wird nie abgeschnitten; ein Stück, das danach nicht kommt, beendet den Schritt
   * wie ein Abbruch.
   */
  stueckMaxMs: 10_000,
};
/** Was ein Häppchen ist: Inhalt (Modell, S0-Antwort), Ansage (Ab S1, Überbrückung) oder feste Vorlage. */
/** `sofort`: Weitersprechen des Sofort-Sprechers (keine Antwort, schließt keinen ersten Satz). */
type PieceKind = "inhalt" | "ansage" | "vorlage" | "sofort";

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
  verlaengern: (neuMs: number) => void;
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
    // Einmal eine längere Frist ab jetzt (nie aus); sie gilt auch für spätere Lebenszeichen, die
    // Obergrenze `maxMs` bleibt. Weitere Überbrückungen ziehen sie nicht neu auf.
    verlaengern: (neuMs: number) => {
      if (gestoppt || neuMs <= ms) return;
      ms = neuMs;
      clearTimeout(timer);
      timer = setTimeout(() => ablaufen(FRIST), neuMs);
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

/** Was am Modell des Gesprächs hängt (Sprecher, Sofort-Sprecher, aktives Modell): ändert es sich, fällt ein offener Auftrag weg. */
function modellStand(settings: unknown): string {
  const s = settings as {
    function_models?: unknown;
    llm_active_model_id?: unknown;
  } | null;
  return JSON.stringify([s?.function_models, s?.llm_active_model_id]);
}

/** Was am Zugriff auf eigene Inhalte hängt (Schalter und freigegebene Bereiche): ändert es sich, gilt der Kontext eines Auftrags nicht mehr. */
function kontextStand(settings: unknown): string {
  const s = settings as {
    voice_global_context_enabled?: unknown;
    voice_call_knowledge?: unknown;
  } | null;
  return JSON.stringify([
    s?.voice_global_context_enabled,
    s?.voice_call_knowledge,
  ]);
}

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
    /** Bricht einen noch wartenden Sofort-Sprecher ab. */
    sofortStop?: () => void;
  } | null = null;
  /** Anfrage, deren Denker zugunsten des Sofort-Sprechers abgebrochen wurde: kein Denktext mehr. */
  private denkStumm: string | null = null;
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
  /**
   * Der offene Auftrag des Gesprächs (höchstens einer): der Denker-Lauf eines Schritts, dessen Vordergrund
   * endete. Nur mit Sofort-Sprecher (Gesprächsmodell nicht lokal); sonst bleibt alles wie bisher.
   */
  private auftrag: Auftrag | null = null;
  /** Zeitpunkt (`performance.now`) des letzten Sprachendes des Nutzers oder des letzten Endes eines Vordergrund-Schritts. */
  private stilleSeit = 0;
  private nachreichTimer: ReturnType<typeof setTimeout> | undefined;
  /** Zählt Sprechbeginn und Fortsetzung einer Äußerung: eine Erkennung davor ist überholt. */
  private ausserungRun = 0;

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
      onStatus: (s) => this.statusWeiter(s),
      conversationId: async () => (await this.cb.conversationId?.()) ?? null,
      ausgabeKey: () => this.ausgabeSchluessel(),
      onDenktext: (id, text) => this.denkText(id, text),
    });
    // Ändern sich Modelle, Verbindungen, Regelwerk oder Einstellungen, liest der nächste Schritt neu.
    // Ein offener Auftrag fällt bei Regelwechsel und bei Modellwechsel weg.
    const invalidate = () => this.invalidiere(true);
    window.addEventListener(COMPLIANCE_CHANGED, invalidate);
    const off = useSettingsStore.subscribe((now, before) => {
      if (now.settings === before.settings) return;
      this.invalidiere(
        modellStand(now.settings) !== modellStand(before.settings),
        kontextStand(now.settings) !== kontextStand(before.settings),
      );
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
    nachgereicht = false,
  ): Promise<void> {
    if (!ctx || ctx.hits.hits.length === 0) return;
    const gen = this.generation;
    const ok = await this.globalContext.verify(answer, ctx);
    this.globalContext.discard(turnId);
    // Ein neues Gespräch hat begonnen, während die Belege geprüft wurden: sie gehören nicht mehr dazu.
    if (gen !== this.generation) return;
    this.cb.onCitations?.(turnId, ok, nachgereicht);
  }

  /** Die wartende Quellenmeldung eines Schritts (höchstens einmal). */
  private quellenMeldenJetzt(turn: Turn): void {
    const melden = turn.quellenMelden;
    turn.quellenMelden = undefined;
    melden?.();
  }

  /**
   * Laufende Antwort sofort verwerfen, ohne den Anruf zu beenden: die Seite
   * wird abgebrochen, die Sprechwarteschlange geleert, vorsynthetisierte
   * Sätze fallen weg (Plan §3.7 Bereichswechsel, Esc).
   */
  interrupt(): void {
    // Eine Erkennung, die noch läuft, gilt nach Esc nicht mehr (auch wenn der Schritt von selbst zu Ende war).
    this.lage++;
    this.verwerfeAuftrag();
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
    this.verwerfeAuftrag();
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
    run.sofortStop?.();
    void run.side?.cancel?.(run.id);
    void invoke("voice_call_cancel", { requestId: run.id }).catch(
      () => undefined,
    );
    run.finish();
    this.cb.onStatus?.(null);
    if (keepPartial && run.partial)
      this.cb.onAssistant(run.partial, true, true);
    this.cb.onDenken?.({ art: "ende", turnId: run.id });
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
    // Ein offener Auftrag mit Kontext fällt weg; sein Ergebnis wird nie nachgereicht.
    if (this.auftrag?.turn.withContext) this.verwerfeAuftrag();
    if (this.turn?.withContext) this.cancelTurn(false);
    if (this.textRun?.withContext) this.cancelText(false);
    this.history = this.history.filter((m) => !m.ctx);
    this.globalContext.clear();
  }

  /** Wissensbereiche geändert: der nächste Schritt lädt Werkzeuge und Hinweis neu. */
  invalidateKnowledge(): void {
    this.invalidiere(false, true);
  }

  updateConfig(cfg: CallConfig): void {
    const wechsel =
      JSON.stringify(this.cfg.orchestration) !==
      JSON.stringify(cfg.orchestration);
    this.cfg = cfg;
    this.invalidiere(wechsel);
    this.taker.config = cfg.turn;
  }

  /**
   * Der nächste Schritt liest Einstellungen, Modelle und Werkzeuge neu. Ein Orchestrator, der dabei
   * abgebrochen wird, nimmt einen offenen Auftrag mit (ein halbes Ergebnis wird nie nachgereicht);
   * bei einem Modell- oder Regelwechsel fällt der Auftrag in jedem Fall weg, bei einer Änderung der Wissensbereiche
   * (`kontextwechsel`) nur ein Auftrag mit Kontext (sein Ergebnis beruht auf eigenen Inhalten).
   */
  private invalidiere(modellwechsel: boolean, kontextwechsel = false): void {
    this.session.invalidate();
    if (
      modellwechsel ||
      this.auftrag?.turn.side ||
      (kontextwechsel && this.auftrag?.turn.withContext)
    )
      this.verwerfeAuftrag();
  }

  private setState(s: CallState): void {
    if (this.state === s) return;
    this.state = s;
    this.cb.onState(s);
    if (s === "listening") this.pruefeNachreichen();
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
      this.player.onIdle = () => this.playerLeer();
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
    this.verwerfeAuftrag();
    clearTimeout(this.nachreichTimer);
    this.nachreichTimer = undefined;
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
          this.onEnd(ev.lastSpeechAt);
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
    this.ausserungRun++;
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
    } else if (this.turn?.nachreichen && !this.warZuHoeren(this.turn)) {
      // Das Nachreichen war noch nicht zu hören (auch nicht eingeplant und synthetisiert, aber ohne Ton):
      // zurückstellen, erst die neue Äußerung beantworten.
      this.cancelTurn(false);
    } else if (
      this.turn &&
      !this.turn.audible &&
      !this.darfAuftrag(this.turn)
    ) {
      startFrame = Math.min(startFrame, this.turn.startFrame);
      this.verwerfeBeimWeitersprechen();
    }
    // Sonst, mit einem Auftrags-Kandidaten (Sprecher sagte „WEITER“, der Denker liefert noch nichts): der
    // Schritt läuft weiter; die Äußerung entscheidet, ob er zum Auftrag wird (`runTurn`).
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
    this.ausserungRun++;
    // Ein Auftrags-Kandidat läuft weiter (die überholte Erkennung gilt nicht mehr, die nächste deckt alles ab).
    if (!(this.turn && this.darfAuftrag(this.turn)))
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

  private onEnd(lastSpeechAt?: number): void {
    const turn = this.turn;
    this.utt = null;
    this.merkeStille(
      lastSpeechAt === undefined ? undefined : this.wall(lastSpeechAt),
    );
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
    // Setzt die Wiedergabe leer fort (nichts mehr zu spielen), gilt das wie ein Leerlauf des Players: der fertige
    // Schritt endet, sonst läuft die Wartezeit auf das nächste Stück von neuem. In der Pause war sie ausgesetzt, und
    // das Fortsetzen meldet nicht in jedem Fall einen Leerlauf (`resume` ohne `onIdle`).
    if (this.turn === turn && !this.player?.active) this.playerLeer();
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

  /**
   * War von diesem Schritt schon etwas zu hören, hat also ein Häppchen tatsächlich begonnen? `turn.audible`
   * genügt dafür nicht: es steht schon, wenn das erste Häppchen eingereiht wird, lange vor dem ersten Ton.
   */
  private warZuHoeren(turn: Turn): boolean {
    return this.gehoert(turn).length > 0;
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
    // Anruf und Gespräch, zu denen diese Erkennung gehört: Auflegen, Neustart und Gesprächswechsel entwerten sie.
    const gespraech = this.generation;
    const anruf = this.startGen;
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
      // neuer Schritt, neue Erkennung), wird es verworfen und bricht nichts ab. Nach dem Auflegen, bei einem Neustart
      // des Anrufs oder einem Gesprächswechsel gilt es erst recht nicht: ein Timeout hebt die Erkennung auf, das
      // Ende des Anrufs nicht.
      if (
        this.lage !== meineLage ||
        gespraech !== this.generation ||
        anruf !== this.startGen ||
        !this.inCall
      )
        return;
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
    if (!this.aeusserungGilt(turn, laeuft)) return;
    const stand = this.entscheidungsStand(turn, laeuft, spaet);
    const inhalt = (stand ? this.gehoert(turn) : []).filter(
      (p) => p.kind === "inhalt",
    );
    const satz = inhalt.length > 0 ? inhalt[inhalt.length - 1].text : "";
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
      this.verwerfeAuftrag();
      this.cancelTurn(true);
      this.setState(this.utt ? "hearing" : "listening");
      return;
    }
    await this.folgeSchritt(
      turn,
      laeuft,
      d,
      text,
      startFrame,
      lastSpeechWall,
      performance.now() - t0,
    );
  }

  /**
   * Gilt eine erkannte Äußerung noch, deren Schritt `turn` ist? Ja, solange er läuft; ist er weg, nur wenn nichts
   * Neues begonnen hat und er von selbst zu Ende lief (Fehler, ausgesprochen) oder die Wartegrenze ihn beendet hat
   * (`stueckFrist`): ein Timeout überholt keine Äußerung, die Erkennung durfte nur länger dauern. Nach Abbruch durch
   * den Nutzer, Esc oder einen anderen Schritt gilt sie nicht mehr.
   */
  private aeusserungGilt(turn: Turn, laeuft: boolean): boolean {
    if (laeuft) return true;
    return this.turn === null && (!turn.cancelled || !!turn.abgelaufen);
  }

  /**
   * Stand für die Entscheidung über eine erkannte Äußerung (`entscheide`). Läuft der Schritt, gilt sein Stand
   * (`spaet`: die Antwort ist schon wieder zu hören, nichts mehr zusammenzuführen). Hat ihn die Wartegrenze
   * beendet, gilt er als schon gehört: zusammengeführt wird nie, Rückmeldelaut und Stoppwort wirken wie sonst.
   * Sonst ist die Äußerung ein gewöhnlicher neuer Beitrag (`null`).
   */
  private entscheidungsStand(
    turn: Turn,
    laeuft: boolean,
    spaet: boolean,
  ): TurnStand | null {
    if (!laeuft && !turn.abgelaufen) return null;
    const stand = this.stand(turn);
    return {
      ...stand,
      ersterSatzFertig: stand.ersterSatzFertig || spaet || !laeuft,
    };
  }

  /**
   * Nach der Entscheidung „Ergänzung“ oder „neuer Beitrag“ (`d`): der alte Schritt endet, ein neuer beginnt.
   * Wartet der alte nur auf den Denker, den der Sprecher mit „WEITER“ überbrückt hat (`darfAuftrag`), endet
   * nur sein Vordergrund: der Denker-Lauf bleibt ein offener Auftrag, und die Äußerung (`text`, unverändert)
   * geht zuerst an den Sprecher. Sonst verschwindet die Ergänzung still (Zusammenführen) oder der Schritt
   * wird unterbrochen wie bisher.
   */
  private async folgeSchritt(
    turn: Turn,
    laeuft: boolean,
    d: Extract<Entscheidung, { art: "zusammenfuehren" | "neu" }>,
    text: string,
    startFrame: number,
    lastSpeechWall: number,
    sttMs: number,
  ): Promise<void> {
    const alsAuftrag = laeuft && this.darfAuftrag(turn);
    if (alsAuftrag) this.zumAuftrag(turn);
    else
      this.cancelTurn(
        d.art === "neu" && d.abbrechen,
        d.art === "zusammenfuehren",
      );
    await this.neuerSchritt(
      startFrame,
      lastSpeechWall,
      alsAuftrag ? text : d.text,
      sttMs,
      !alsAuftrag && d.art === "zusammenfuehren" ? turn.userText : undefined,
    );
  }

  /** Neuer Schritt aus gesprochener Eingabe; `prefix`: Vortext einer Ergänzung. */
  private async neuerSchritt(
    startFrame: number,
    lastSpeechWall: number,
    text: string,
    sttMs: number,
    prefix?: string,
  ): Promise<void> {
    const neuer = this.newTurn(startFrame, lastSpeechWall);
    neuer.fromSpeech = true;
    if (prefix !== undefined) neuer.prefix = prefix;
    this.turn = neuer;
    // Das sichere Satzende kann schon vorbei sein: dann gibt es niemanden mehr, der freigibt.
    if (!this.utt) {
      neuer.committed = true;
      neuer.commit();
    }
    this.setState("thinking");
    await this.antworteSicher(neuer, text, sttMs);
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
    // Eine endgültige Zeile wird gespeichert und ist nicht mehr zurückzunehmen: sie geht erst hinaus, wenn von der
    // Antwort wirklich etwas zu hören war. Bis dahin lässt sich der Schritt noch ohne Spur beenden (Nutzer spricht,
    // Stopp, Wechsel), ein Nachreichen sogar zurückstellen. Ganz eingereihte, aber stumme Antworten (zurückgehaltener
    // Start, ausgebliebener Ton) hinterlassen nichts.
    if (!this.warZuHoeren(turn)) return;
    // `warZuHoeren` kann den Beginn melden, und der meldet die Antwort bereits.
    if (turn.finalSent) return;
    turn.finalSent = true;
    this.cb.onAssistant(turn.finalText, true, false, turn.nachreichen?.turnId);
  }

  /**
   * Ein Stück des Schritts klingt wirklich: eine fertige, bisher stumm gewartete Antwortzeile geht jetzt hinaus
   * (Vorlage, Wiederholen, kurze Antwort). Eine Antwort, die noch durch eine Ergänzung ersetzt werden kann, wartet
   * weiter auf das Ende des ersten Satzes (`ersterSatzFertig`).
   */
  private beginnGehoert(turn: Turn): void {
    if (turn.finalText === undefined || darfZusammenfuehren(this.stand(turn)))
      return;
    this.sendeAntwort(turn);
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
      synthOffen: 0,
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
    // Wartet der laufende Schritt nur auf den Denker (Sprecher sagte „WEITER“): er läuft weiter, bis die
    // Äußerung erkannt ist; erst dann entscheidet sich, ob er zum offenen Auftrag wird.
    const kandidat =
      this.turn && this.darfAuftrag(this.turn) ? this.turn : null;
    if (kandidat) {
      await this.runTurnMitAuftrag(
        kandidat,
        samples,
        startFrame,
        lastSpeechWall,
        speechMs,
      );
      return;
    }
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

  /** Nach einer Erkennung, die nichts ändert: der Zustand folgt dem, was gerade läuft. */
  private zurueckInsGespraech(): void {
    this.setState(
      this.utt
        ? "hearing"
        : this.player?.active
          ? "speaking"
          : this.turn
            ? "thinking"
            : "listening",
    );
  }

  /**
   * Äußerung, während der Schritt `kandidat` auf den Denker wartet (der Sprecher hat „WEITER“ gesagt, hörbar
   * ist vom Denker noch nichts). Der Schritt läuft währenddessen unverändert weiter. Mit dem Text gilt dieselbe
   * Entscheidung wie bei angehaltener Wiedergabe: Rückmeldelaut (nichts ändert sich), Stoppwort (alles endet),
   * sonst endet nur der Vordergrund und der Denker bleibt ein offener Auftrag.
   */
  private async runTurnMitAuftrag(
    kandidat: Turn,
    samples: Float32Array,
    startFrame: number,
    lastSpeechWall: number,
    speechMs: number,
  ): Promise<void> {
    const run = this.ausserungRun;
    const gen = this.generation;
    const start = this.startGen;
    const lage = this.lage;
    // Überholt: der Nutzer spricht weiter (die nächste Erkennung deckt die ganze Äußerung ab), das Gespräch hat
    // sich geändert (neues Gespräch, Ende oder Neustart des Anrufs) oder inzwischen hat ein anderer Schritt
    // begonnen, auch einer, der schon wieder zu Ende ist (getippter Text), oder Esc hat alles beendet (`lage`).
    // Diese Erkennung wirkt dann gar nicht.
    const ueberholt = () =>
      run !== this.ausserungRun ||
      gen !== this.generation ||
      start !== this.startGen ||
      lage !== this.lage ||
      !this.inCall;
    const t0 = performance.now();
    let erkannt: string;
    try {
      erkannt = (
        await invoke<string>("voice_call_transcribe", {
          pcm16B64: floatToPcm16Base64(samples),
        })
      ).trim();
    } catch (e) {
      if (ueberholt()) return;
      this.cb.onError(String(e));
      this.zurueckInsGespraech();
      return;
    }
    if (ueberholt()) return;
    if (isPhantom(erkannt, speechMs)) {
      this.zurueckInsGespraech();
      return;
    }
    const laeuft = this.turn === kandidat && !kandidat.cancelled;
    // Der Schritt ist weg, weil ihn etwas anderes beendet hat (getippter Text, Esc, Widerruf): die Äußerung
    // gilt nicht mehr und darf weder den neuen Schritt abbrechen noch einen eigenen beginnen. Nur ein Schritt,
    // der von selbst zu Ende lief (Fehler, ausgesprochen) oder dem die Wartegrenze ein Ende setzte, lässt sie
    // gelten (`aeusserungGilt`).
    if (!this.aeusserungGilt(kandidat, laeuft)) return;
    const d = entscheide(
      erkannt,
      this.entscheidungsStand(kandidat, laeuft, false),
      kandidat.userText,
    );
    if (d.art === "weiter") {
      this.zurueckInsGespraech();
      return;
    }
    if (d.art === "stopp") {
      this.verwerfeAuftrag();
      this.cancelTurn(true);
      this.setState(this.utt ? "hearing" : "listening");
      return;
    }
    await this.folgeSchritt(
      kandidat,
      laeuft,
      d,
      erkannt,
      startFrame,
      lastSpeechWall,
      performance.now() - t0,
    );
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
    // Getippter Text bleibt wie bisher: ein offener Auftrag fällt weg, der neue Schritt beginnt frei.
    this.verwerfeAuftrag();
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

  /**
   * Schlüssel für die Modellrunden des Tors: wie `konversation`, aber auch der einmal gewählte, solange das
   * Gedächtnis dem offenen Auftrag gehört (seine Runden laufen im Hintergrund weiter und müssen dort ablegen,
   * wo „wiederholen“ später sucht).
   */
  private async ausgabeSchluessel(): Promise<string | null> {
    const a = this.auftrag;
    if (a && !a.verworfen && a.ausgabe) return a.ausgabe.id;
    return this.konversation();
  }

  /**
   * Der Schlüssel, den der Denker-Lauf von `turn` gewählt hat, gehört diesem Lauf: ist er inzwischen ein offener
   * Auftrag (die Kennungsabfrage dauerte), kommt er in den Auftrag, nie ins Feld des Gesprächs, wo ihn die kurze
   * Antwort des Sprechers samt Gedächtnis des Tors löschen würde.
   */
  private schluesselDesLaufs(turn: Turn, key: { id: string | null }): void {
    const a = turn.auftrag;
    if (a && !a.verworfen) a.ausgabe = key;
    else this.ausgabeKey = key;
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

  /**
   * Startet den Sofort-Sprecher (lokal, nur Verlauf und Frage). Er entscheidet höchstens einmal:
   * vollständige Antwort (der Denker wird abgebrochen) oder kurz weitersprechen (der Denker folgt).
   * Ohne Modell, im Projekt oder bei gesperrtem Gespräch läuft nichts, und der Denker gilt allein.
   */
  private sofortStarten(o: {
    model: VoiceModel | null | undefined;
    text: string;
    id: string;
    datenAbsicht: boolean;
    tot: () => boolean;
    sagen: (text: string, weiter: boolean) => void;
    denkerAbbrechen: () => void;
    /**
     * Offener Auftrag: der Sprecher bekommt zusätzlich den Block „offener Suchauftrag“ und entscheidet mit
     * „ANTWORT“ (der Auftrag bleibt), „SUCHE“ oder „ABBRUCH“ (`verwerfen` räumt den alten Auftrag ab).
     */
    auftrag?: {
      frage: string;
      ergebnisDa: boolean;
      verwerfen: (art: "suche" | "abbruch") => void;
    };
  }): SofortLauf {
    let siegen: (t: string) => void = () => undefined;
    const gewonnen = new Promise<string>((r) => (siegen = r));
    let entschieden: () => void = () => undefined;
    const entscheidung = new Promise<void>((r) => (entschieden = r));
    let gesprochen = false;
    let weiterGesagt = false;
    let zustand: SofortZustand =
      o.model && !this.projekt() ? "wartet" : "denker";
    const nichts: SofortLauf = {
      zustand: () => zustand,
      denkerKommt: () => undefined,
      stop: () => undefined,
      gewonnen,
      gesprochen: () => false,
      abwarten: () => Promise.resolve(),
      weiter: () => false,
    };
    if (!o.model || zustand !== "wartet") return nichts;
    const model = o.model;
    const ctl = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const beenden = () => {
      clearTimeout(timer);
      if (!ctl.signal.aborted) ctl.abort();
      entschieden();
    };
    const entscheide = (e: SofortEntscheid) => {
      if (zustand !== "wartet") return;
      beenden();
      if (o.tot()) {
        zustand = "denker";
        return;
      }
      if (o.auftrag) {
        // Keine Heuristik: die Marke entscheidet. Alles Unbrauchbare gilt wie „SUCHE“.
        const a = wendeAuftragAn(e, o.datenAbsicht);
        if (a.art !== "antwort") o.auftrag.verwerfen(a.art);
        weiterGesagt = a.art === "suche";
        zustand = weiterGesagt ? "denker" : "sprecher";
        if (a.sagen) {
          gesprochen = true;
          o.sagen(a.sagen, weiterGesagt);
        }
        if (zustand === "sprecher") siegen("");
        return;
      }
      const r = wendeAn(zustand, e, o.datenAbsicht);
      zustand = r.zustand;
      weiterGesagt = e.art === "weiter";
      if (r.sagen) {
        gesprochen = true;
        o.sagen(r.sagen, r.zustand === "denker");
      }
      if (r.denkerAbbrechen) {
        // Späte Denkschritte des abgebrochenen Denkers gehören nicht zur Antwort des Sprechers.
        this.denkStumm = o.id;
        o.denkerAbbrechen();
        siegen("");
      }
    };
    timer = setTimeout(() => entscheide({ art: "nichts" }), SOFORT_MAX_MS);
    this.cb.onDenken?.({ art: "sofort", modell: model.label });
    const sprache = aktuelleSprache();
    const messages: ModelMessage[] = [
      {
        role: "system",
        // Der Block „offener Suchauftrag“ geht nur an dieses lokale Modell, nie an den Denker.
        content: o.auftrag
          ? `${sofortPrompt(sprache)}\n\n${auftragsBlock(o.auftrag.frage, o.auftrag.ergebnisDa, sprache)}`
          : sofortPrompt(sprache),
      },
      ...this.plainHistory(false)
        .filter((m) => m.role === "user" || m.role === "assistant")
        .slice(-SOFORT_VERLAUF)
        .map((m) => ({
          role: m.role as "user" | "assistant",
          content: m.content,
        })),
      { role: "user", content: o.text },
    ];
    void model
      .turn({
        requestId: `${o.id}:sofort`,
        messages,
        signal: ctl.signal,
        onText: () => undefined,
      })
      .then(
        (r) =>
          entscheide(
            ctl.signal.aborted
              ? { art: "nichts" }
              : liesSofort(r.text, !!o.auftrag),
          ),
        () => entscheide({ art: "nichts" }),
      );
    return {
      zustand: () => zustand,
      denkerKommt: () => {
        if (zustand !== "wartet") return;
        zustand = "denker";
        beenden();
      },
      stop: () => {
        if (zustand === "wartet") zustand = "denker";
        beenden();
      },
      gewonnen,
      gesprochen: () => gesprochen,
      abwarten: () => entscheidung,
      weiter: () => weiterGesagt,
    };
  }

  /**
   * Ansage als Schritt der Denk-Anzeige. Die allgemeine Ansage („Ich denke kurz nach.“) sagt nichts,
   * was die Anzeige nicht schon zeigt: sie entfällt ganz.
   */
  private denkSchritt(absicht: string, text: string): void {
    if (absicht === "allgemein" || !text.trim()) return;
    this.cb.onDenken?.({ art: "schritt", text: text.trim() });
  }

  /** „Wiederholen“: die letzte vom Tor freigegebene Ausgabe, sonst die Vorlage „nichts gesagt“. */
  private async wiederholung(sprache: Sprache): Promise<string[]> {
    const key = this.ausgabeKey;
    const again = key ? await letzteAusgabe(key.id) : null;
    // Gemerkt ist die ganze freigegebene Ausgabe: Überbrückungen darin werden nie wiederholt.
    const text = again ? speakable(ohneUeberbrueckungen(again)) : "";
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
      this.cb.onDenken?.({ art: "start", turnId: id });
      const marken: Marken = { start: performance.now() };
      // Was das Modell geliefert hat (ohne Vorlagen): Verlauf und „nichts geantwortet“.
      let modelPartial = "";
      // Weitersprechen des Sofort-Sprechers: angezeigt, aber keine Antwort.
      let sofortText = "";
      const onPiece = (piece: string, kind: PieceKind = "inhalt") => {
        run.partial += (run.partial ? " " : "") + piece;
        this.cb.onAssistant(run.partial, false, false);
        const now = performance.now();
        marken.inhalt ??= now;
        if (kind === "inhalt")
          modelPartial += (modelPartial ? " " : "") + piece;
        if (kind === "sofort") sofortText += (sofortText ? " " : "") + piece;
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
      const { side, blocked, systemPrompt, sofort } = await this.orchestrate();
      if (stale() || blocked) return;
      // Ab S1: die Ansage ist nur ein Schritt der Denk-Anzeige, nie Chat oder Stimme.
      if (plan.kind === "ansage") this.denkSchritt(plan.absicht, plan.text);
      let fristStop = () => undefined as void;
      let denkerLaeuft = false;
      const sofortLauf = this.sofortStarten({
        model: sofort,
        text,
        id,
        datenAbsicht: plan.kind === "ansage" && plan.absicht !== "allgemein",
        tot: stale,
        sagen: (t, weiter) => {
          if (weiter) this.cb.onDenken?.({ art: "zwischen" });
          onPiece(t, weiter ? "sofort" : "inhalt");
        },
        denkerAbbrechen: () => {
          fristStop();
          if (!denkerLaeuft) return;
          void side?.cancel?.(id);
          void invoke("voice_call_cancel", { requestId: id }).catch(
            () => undefined,
          );
          finish();
        },
      });
      run.sofortStop = () => sofortLauf.stop();
      const gctx = await this.contextFor(id, text, side);
      if (stale()) return;
      const frist = ersteFrist(
        TUNING.modelFristMs,
        this.projekt() ? TUNING.projektObergrenzeMs : undefined,
      );
      fristStop = () => frist.stop();
      if (sofortLauf.zustand() === "sprecher") frist.stop();
      let timedOut = false;
      let ueberbrueckt = false;
      let denkerGeliefert = false;
      // Das Tor im Projekt hat die Anfrage mit einem Code abgelehnt: Die Hinweisleiste sagt den Grund.
      let projektStopp = false;
      const onModel = (s: string) => {
        if (stale() || timedOut) return;
        // Der Sofort-Sprecher hat vollständig geantwortet: späte Sätze des Denkers verwerfen.
        if (sofortLauf.zustand() === "sprecher") return;
        // Überbrückungen (Orchestrator, Werkzeug, Buch-Seite, oder ein Modell, das eine sagt) auf
        // jedem Weg nur in der Denk-Anzeige, nie Chat oder Stimme. Die Frist läuft weiter, länger.
        if (istUeberbrueckung(s)) {
          ueberbrueckt = true;
          frist.verlaengern(TUNING.denkerFristMs);
          this.cb.onDenken?.({ art: "schritt", text: s.trim() });
          return;
        }
        sofortLauf.denkerKommt();
        denkerGeliefert = true;
        frist.stop();
        onPiece(s);
      };
      let full = "";
      // Auch bei einem Rücksprung (veraltet, abgebrochen) die Zeitgeber der Frist abräumen.
      try {
        if (sofortLauf.zustand() === "sprecher") {
          // Der Sofort-Sprecher hat vollständig geantwortet, bevor der Denker gefragt wurde.
        } else if (side) {
          denkerLaeuft = true;
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
          const r = await Promise.race([
            chat,
            frist.abgelaufen,
            sofortLauf.gewonnen,
          ]);
          if (r === FRIST) {
            timedOut = true;
            void side.cancel?.(id);
          } else full = r;
          frist.stop();
          if (stale()) return;
        } else {
          // Ab hier gilt der Denker als gefragt: ein Abbruch erreicht ihn auch vor dem Start (Rust
          // merkt sich vorgezogene Abbrüche).
          denkerLaeuft = true;
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
            if (e.kind === "reasoning") {
              this.denkText(id, e.text ?? "");
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
          const r = await Promise.race([
            chat,
            frist.abgelaufen,
            sofortLauf.gewonnen,
          ]);
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
      // Kam vom Denker nichts, während der Sprecher noch entscheidet: seine Entscheidung abwarten
      // (höchstens `SOFORT_MAX_MS`); eine Antwort von ihm gilt dann statt der Vorlage.
      if (!denkerGeliefert && sofortLauf.zustand() === "wartet")
        await sofortLauf.abwarten();
      sofortLauf.stop();
      if (stale()) return;
      const sprecherAllein = sofortLauf.zustand() === "sprecher";
      // Mit Orchestrator zählt, was durch den Filter lief (die Rückgabe kann die Überbrückung enthalten).
      const rueckgabe =
        timedOut || sprecherAllein || istUeberbrueckung(full)
          ? ""
          : full.trim();
      // Mit Orchestrator, nach einer gefilterten Überbrückung oder nach dem Sprecher zählt, was
      // angezeigt wurde; sonst die Rückgabe des Tors.
      const inhalt =
        sprecherAllein || ueberbrueckt
          ? modelPartial
          : side
            ? modelPartial || rueckgabe
            : rueckgabe || modelPartial;
      // Das Weitersprechen des Sprechers gehört nur zu einer Antwort, wenn der Denker geliefert hat.
      const answer = inhalt
        ? [sofortText, inhalt].filter(Boolean).join(" ")
        : "";
      if (sprecherAllein) this.ausgabeVergessen();
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
        // Unbelegte Sätze wie eine Überbrückung gibt das Belegtor im Projekt gar nicht frei; die
        // Rückgabe ist also schon bereinigt (Test `ueberbrueckung_faellt_vor_dem_register_weg…`).
        content: this.projekt() ? rueckgabe || answer : speakable(answer),
        ...tag,
        ...(run.withContext ? { ctx: true } : {}),
      });
      this.cb.onAssistant(answer, true, false);
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
    // Denker-Teil: der Lauf eines offenen Auftrags lebt weiter, auch wenn sein Vordergrund beendet ist.
    const denkerTot = () =>
      turn.cancelled || (this.turn !== turn && !turn.auftrag);
    // Ein Stoppwort verwirft einen offenen Auftrag (und geht nicht ans Modell).
    if (this.offenerAuftrag() && istStoppwort(text)) {
      this.verwerfeAuftrag();
      this.dropTurn(turn);
      return;
    }
    turn.userText = display ?? text;
    this.cb.onUser(turn.userText, turn.id);
    this.cb.onDenken?.({ art: "start", turnId: turn.id });

    const tChat = performance.now();
    let firstSentenceAt = 0;
    let firstAudioAt = 0;
    let chain: Promise<void> = Promise.resolve();
    let assistantText = "";
    // Was das Modell geliefert hat (ohne Vorlagen): Verlauf, Belege, „nichts geantwortet“.
    let modelText = "";
    // Weitersprechen des Sofort-Sprechers: gesprochen, aber keine Antwort (Verlauf nur mit dem Denker).
    let sofortText = "";
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
      if (kind === "inhalt") modelText += (modelText ? " " : "") + piece;
      if (kind === "sofort") {
        sofortText += (sofortText ? " " : "") + piece;
        turn.sofortText = sofortText;
      }
      assistantText += (assistantText ? " " : "") + piece;
      turn.angezeigt = true;
      this.cb.onAssistant(assistantText, false, false);
      turn.synthOffen++;
      const schritt = async () => {
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
        // Das Stück ist da: die Wartezeit auf es (`warteAufStueck`) ist vorbei.
        clearTimeout(turn.stueckTimer);
        turn.stueckTimer = undefined;
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
            this.beginnGehoert(turn);
            // Der Inhalt beginnt zu klingen: jetzt melden die Quellen (falls sie darauf gewartet haben).
            if (kind === "inhalt") this.quellenMeldenJetzt(turn);
          },
        );
        rec.queued = true;
      };
      chain = chain.then(schritt).finally(() => this.stueckFertig(turn));
      // Ist der Player schon leer (das Modell lieferte später), beginnt die Wartezeit auf dieses Stück jetzt.
      this.pruefeStueckwartezeit(turn, true);
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

    const { side, blocked, systemPrompt, sofort } = await this.orchestrate();
    if (dead()) return;
    // Ab S1: die Ansage ist nur ein Schritt der Denk-Anzeige, nie Chat oder Stimme.
    if (plan.kind === "ansage" && !blocked)
      this.denkSchritt(plan.absicht, plan.text);
    // Offener Auftrag: Die Äußerung geht zuerst NUR an den lokalen Sprecher (mit dem Block „offener
    // Suchauftrag“), ein neuer Denker startet erst bei „SUCHE“. Ohne lokalen Sprecher gilt sie wie „SUCHE“.
    const offen = this.offenerAuftrag();
    const sprecherModell = blocked ? null : sofort;
    const mitAuftrag =
      !!offen && sprecherModell?.local === true && !this.projekt();
    if (offen && !mitAuftrag) this.verwerfeAuftrag(offen, !blocked);
    // Sofort-Sprecher: startet vor dem Abruf des Kontexts, damit die erste Reaktion nicht auf ihn wartet.
    let fristStop = () => undefined as void;
    let denkerLaeuft = false;
    const sofortLauf = this.sofortStarten({
      model: sprecherModell,
      text,
      id: turn.id,
      datenAbsicht: plan.kind === "ansage" && plan.absicht !== "allgemein",
      tot: dead,
      sagen: (t, weiter) => {
        if (weiter) this.cb.onDenken?.({ art: "zwischen" });
        onPiece(t, weiter ? "sofort" : "inhalt");
      },
      denkerAbbrechen: () => {
        fristStop();
        if (!denkerLaeuft) return;
        void turn.side?.cancel?.(turn.id);
        void invoke("voice_call_cancel", { requestId: turn.id }).catch(
          () => undefined,
        );
        finishStream();
      },
      ...(mitAuftrag && offen
        ? {
            auftrag: {
              frage: offen.frage,
              ergebnisDa: offen.fertig,
              verwerfen: (art: "suche" | "abbruch") => {
                this.verwerfeAuftrag(offen, art === "suche");
                if (art === "abbruch") turn.auftragAbbruch = true;
              },
            },
          }
        : {}),
    });
    turn.sofortStop = () => sofortLauf.stop();
    turn.sprecher = sofortLauf;
    // Kein paralleler Denker: erst wenn der Sprecher entschieden hat (höchstens `SOFORT_MAX_MS`).
    if (mitAuftrag) {
      await sofortLauf.abwarten();
      if (dead()) return;
    }
    // Globaler Kontext: lokal geholt, schon beim spekulativen Start (Plan L1/M5).
    const gctx = blocked ? null : await this.contextFor(turn.id, text, side);
    turnCtx = gctx;
    turn.kontext = gctx;
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
    fristStop = () => frist.stop();
    turn.denkerFristStop = fristStop;
    if (sofortLauf.zustand() === "sprecher") frist.stop();
    let timedOut = false;
    let denkerGeliefert = false;
    // Das Tor im Projekt hat die Anfrage mit einem Code abgelehnt: Die Hinweisleiste sagt den Grund.
    let projektStopp = false;
    const onModel = (s: string) => {
      if (denkerTot() || timedOut) return;
      // Der Sofort-Sprecher hat vollständig geantwortet: späte Sätze des Denkers verwerfen.
      if (sofortLauf.zustand() === "sprecher") return;
      // Überbrückungen (Orchestrator, Werkzeug, Buch-Seite, oder ein Modell, das eine sagt) auf
      // jedem Weg nur in der Denk-Anzeige, nie Chat oder Stimme. Die Frist läuft weiter, länger.
      if (istUeberbrueckung(s)) {
        frist.verlaengern(TUNING.denkerFristMs);
        this.cb.onDenken?.({ art: "schritt", text: s.trim(), turnId: turn.id });
        return;
      }
      sofortLauf.denkerKommt();
      denkerGeliefert = true;
      frist.stop();
      // Ein offener Auftrag spricht nie direkt: seine Sätze warten im (gedeckelten) Puffer.
      if (turn.auftrag) this.puffere(turn.auftrag, s);
      else {
        (turn.denkerSaetze ??= []).push(s);
        onPiece(s);
      }
    };
    // Auch bei einem Rücksprung (veraltet, abgebrochen) die Zeitgeber der Frist abräumen.
    try {
      if (sofortLauf.zustand() === "sprecher") {
        // Der Sofort-Sprecher hat vollständig geantwortet, bevor der Denker gefragt wurde.
      } else if (side) {
        denkerLaeuft = true;
        turn.denkerStart = performance.now();
        // Denker oder Werkzeuge aktiv: der Orchestrator liefert die Sätze (auch die Überbrückung).
        turn.side = side;
        if (!this.ausgabeKey) {
          const id = await this.konversation();
          if (denkerTot()) return;
          this.schluesselDesLaufs(turn, { id });
        }
        const chat = side.chat(turn.id, this.historyFor(text), onModel).then(
          () => "",
          () => {
            turn.denkerFehler = true;
            return "";
          },
        );
        if (
          (await Promise.race([
            chat,
            frist.abgelaufen,
            sofortLauf.gewonnen,
          ])) === FRIST
        ) {
          timedOut = true;
          void side.cancel?.(turn.id);
        }
        frist.stop();
      } else {
        // Ab hier gilt der Denker als gefragt: ein Abbruch erreicht ihn auch vor dem Start (Rust
        // merkt sich vorgezogene Abbrüche).
        denkerLaeuft = true;
        turn.denkerStart = performance.now();
        // Mit App-Kontext startet das Modell erst nach dem Commit, und der
        // Kontext wird davor erneut geprüft (Turn, Transkript, Stand; Codex
        // M5, Auflagen 6–8). Ohne Kontext bleibt der spekulative Start.
        const projekt = this.projekt();
        let context = projekt ? null : speakerContext(gctx);
        if (context) {
          await turn.committedP;
          if (denkerTot()) {
            this.globalContext.discard(turn.id);
            return;
          }
          const fresh = await this.globalContext.take(turn.id, text);
          if (denkerTot()) return;
          turnCtx = fresh;
          turn.kontext = fresh;
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
          if (e.kind === "reasoning") {
            this.denkText(turn.id, e.text ?? "");
            return;
          }
          if (e.kind === "revoked") {
            // Freigabe entzogen: Antwort und Sprachausgabe sofort beenden (ein offener Auftrag fällt weg).
            if (projekt) this.meldeProjekt({ code: "projekt.widerrufen" });
            finishStream();
            if (turn.auftrag) this.verwerfeAuftrag(turn.auftrag);
            if (this.turn === turn) this.cancelTurn(false);
            if (projekt) this.cb.onVerwerfen?.();
            return;
          }
          if ((e.kind === "done" || e.kind === "failed") && e.usage)
            this.cb.onUsage?.(e.usage);
          if (e.kind === "failed") turn.denkerFehler = true;
          if (e.kind === "done" || e.kind === "failed") finishStream();
          if (e.kind !== "sentence" || denkerTot()) return;
          onModel(e.text ?? "");
        });
        const conversationId = await this.konversation();
        if (denkerTot()) return;
        this.schluesselDesLaufs(turn, { id: conversationId });
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
            turn.denkerFehler = true;
            const h = projekt ? hinweisAusFehler(e) : null;
            projektStopp = h !== null;
            this.meldeProjekt(h);
            return "";
          },
        );
        if (
          (await Promise.race([
            chat,
            frist.abgelaufen,
            sofortLauf.gewonnen,
          ])) === FRIST
        ) {
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
        } else if (!denkerTot() && !abgelehnt) {
          // Das letzte Häppchen kann nach dem Rückgabewert eintreffen: auf `done` warten.
          await warteMax(streamEnded, 3000);
        }
        frist.stop();
      }
    } finally {
      frist.stop();
    }
    turn.denkerFertig = true;
    // Kam vom Denker nichts, während der Sprecher noch entscheidet: seine Entscheidung abwarten
    // (höchstens `SOFORT_MAX_MS`); eine Antwort von ihm gilt dann statt der Vorlage.
    if (!denkerGeliefert && sofortLauf.zustand() === "wartet")
      await sofortLauf.abwarten();
    sofortLauf.stop();
    if (sofortLauf.zustand() === "sprecher") this.ausgabeVergessen();
    await chain;
    this.sentenceHandlers.delete(turn.id);
    // Der Denker eines offenen Auftrags ist zu Ende: sein Ergebnis wartet auf freien Vordergrund.
    if (turn.auftrag) {
      this.auftragDenkerEnde(turn.auftrag);
      return;
    }
    this.cb.onStatus?.(null);
    if (dead()) return;
    // „ABBRUCH“ ohne Bestätigung: der Schritt bleibt stumm (keine Vorlage, kein Verlauf).
    if (turn.auftragAbbruch && !modelText) {
      turn.noHistory = true;
      turn.done = true;
      this.dropTurn(turn);
      return;
    }
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
      : speakable([sofortText, modelText].filter(Boolean).join(" "));
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
    // Die Belege prüfen nur, was das Modell gesagt hat, nicht die Ansage. Kann der Schritt noch zum Auftrag
    // werden (nichts vom Denker zu hören), melden sie erst, wenn sein Inhalt beginnt: ein Auftrag meldet
    // sie beim Nachreichen und nie für eine Zeile, die sein Ergebnis nicht trägt.
    const quellen = () =>
      void this.reportCitations(turn.id, modelText, turnCtx);
    if (this.darfAuftrag(turn)) turn.quellenMelden = quellen;
    else quellen();
    // Ende der Wiedergabe meldet player.onIdle -> finishTurn.
    if (!this.player?.active) this.finishTurn();
  }

  /** Denktext eines Modells: nur, solange seine Anfrage der laufende Schritt oder der offene Auftrag ist. */
  private denkText(requestId: string, text: string): void {
    if (!text || requestId === this.denkStumm) return;
    const auftrag = this.auftrag;
    const fuerAuftrag =
      !!auftrag && !auftrag.verworfen && auftrag.turnId === requestId;
    if (
      !fuerAuftrag &&
      this.turn?.id !== requestId &&
      this.textRun?.id !== requestId
    )
      return;
    this.cb.onDenken?.({
      art: "gedanke",
      text,
      ...(fuerAuftrag ? { turnId: requestId } : {}),
    });
  }

  // -- Offener Auftrag (ADR-0028 E4.2c 7(e)) -----------------------------------------------------

  /** Werkzeug-Status des Orchestrators: bei offenem Auftrag gehört er zu dessen Anzeige im Hintergrund. */
  private statusWeiter(s: CallStatus | null): void {
    const a = this.auftrag;
    if (a && !a.verworfen && !a.fertig && s?.kind === "tool") {
      this.cb.onStatus?.(s, true);
      return;
    }
    this.cb.onStatus?.(s);
  }

  /** Der offene Auftrag, solange er weder verworfen ist noch gerade nachgereicht wird. */
  private offenerAuftrag(): Auftrag | null {
    const a = this.auftrag;
    return a && !a.verworfen && !a.nachreichend ? a : null;
  }

  /**
   * Darf der Vordergrund dieses Schritts enden, während sein Denker als offener Auftrag weiterläuft? Nur mit
   * Sofort-Sprecher: er hat „WEITER“ gesagt, der Denker läuft (nicht länger als `auftragMaxMs`), hat noch
   * nichts Hörbares geliefert und ist nicht gescheitert. Einen Auftrag gibt es höchstens einen je Gespräch.
   * Auch ein schon fertig erzeugtes, aber noch nicht gehörtes Ergebnis (`turn.done`: alles eingereiht, hinter
   * dem Weitersprechen) gehört dazu; `zumAuftrag` nimmt dann den Verlaufseintrag zurück.
   */
  private darfAuftrag(turn: Turn): boolean {
    if (this.auftrag || turn.cancelled || turn.noMerge) return false;
    if (turn.nachreichen || turn.denkerFehler) return false;
    // Getippter Text bleibt wie bisher: seine Antwortzeile geht schon beim Fertigwerden hinaus und
    // ließe sich nicht mehr zurücknehmen (die Äußerung ersetzt den Schritt, ein Auftrag wird es nie).
    if (!turn.fromSpeech) return false;
    if (!turn.sprecher?.weiter() || turn.denkerStart === undefined)
      return false;
    if (turn.denkerFertig && !turn.denkerSaetze?.length) return false;
    if (performance.now() - turn.denkerStart >= TUNING.auftragMaxMs)
      return false;
    return !this.gehoert(turn).some((p) => p.kind === "inhalt");
  }

  /** Legt einen Satz des Denkers in den Puffer; über dem Deckel wird er gezählt und verworfen. */
  private puffere(a: Auftrag, satz: string): void {
    if (a.zeichen + satz.length > TUNING.auftragMaxZeichen) {
      if (a.ueberlauf++ === 0)
        console.warn(
          "voice-call: Satzpuffer eines Auftrags voll, Rest verworfen",
        );
      return;
    }
    a.saetze.push(satz);
    a.zeichen += satz.length;
  }

  /**
   * Der Vordergrund von `turn` endet, sein Denker-Lauf bleibt ein offener Auftrag: Wiedergabe des Weitersprechens
   * stoppt, aber weder `voice_call_cancel` noch `side.cancel`; seine Sätze werden ab jetzt gepuffert.
   */
  private zumAuftrag(turn: Turn): void {
    const sofort = this.gehoert(turn).filter((p) => p.kind === "sofort");
    const gesagt = sofort.map((p) => p.text).join(" ");
    const a: Auftrag = {
      turn,
      turnId: turn.id,
      frage: turn.userText,
      gesagt,
      saetze: [],
      zeichen: 0,
      ueberlauf: 0,
      fertig: !!turn.denkerFertig,
      gestartet: turn.denkerStart ?? performance.now(),
      timer: undefined,
      verworfen: false,
      nachreichend: false,
      // Das Gedächtnis des Tors in Rust gehört ab jetzt dem Auftrag (siehe `Auftrag.ausgabe`).
      ausgabe: this.ausgabeKey,
    };
    this.ausgabeKey = null;
    // Was der Denker schon geliefert hat, aber noch nicht zu hören war, wartet jetzt im Puffer.
    for (const satz of turn.denkerSaetze ?? []) this.puffere(a, satz);
    turn.denkerSaetze = undefined;
    this.starteFrist(a);
    turn.denkerFristStop?.();
    if (turn.historyPushed) {
      // Das fertige Ergebnis war noch nicht zu hören: der Verlauf kennt es erst nach dem Nachreichen.
      const i = this.history.findIndex(
        (m) => m.role === "user" && m.turnId === turn.id,
      );
      if (i >= 0) this.history.splice(i, 2);
      turn.historyPushed = false;
    }
    turn.finalText = undefined;
    turn.quellenMelden = undefined;
    turn.auftrag = a;
    this.auftrag = a;
    this.turn = null;
    if (this.held === turn) {
      this.held = null;
      this.holdRun++;
      this.holdBusy = false;
      this.zaehleHaltezeit(turn);
    }
    this.player?.stop();
    this.cb.onDenken?.({ art: "auftrag", turnId: turn.id, frage: a.frage });
    // Die Zeile des Weitersprechens bleibt, soweit sie zu hören war; Ungehörtes verschwindet.
    if (turn.angezeigt) {
      if (gesagt) {
        const ganz = sofort.every((p) => p.ended) && gesagt === turn.sofortText;
        this.cb.onAssistant(gesagt, true, !ganz);
      } else this.cb.onVerwerfen?.();
    }
    this.merkeStille();
    // Der Denker war schon fertig (`respond` hat sein Ende bereits hinter sich): jetzt gilt das Ergebnis.
    if (turn.done) this.auftragDenkerEnde(a);
  }

  /**
   * Auftrag verwerfen: Ausgabe ab jetzt 0 (kein Satz, keine Zeile, kein Verlauf), der Denker-Lauf wird
   * abgebrochen, späte Ereignisse sind wirkungslos. `merken`: die ursprüngliche Frage bleibt als unterbrochener
   * Verlaufseintrag stehen (bei „SUCHE“ braucht der neue Denker sie als Zusammenhang). Mehrfach aufrufbar.
   */
  private verwerfeAuftrag(
    a: Auftrag | null = this.auftrag,
    merken = false,
  ): void {
    if (!a || a.verworfen) return;
    a.verworfen = true;
    clearTimeout(a.timer);
    if (this.auftrag === a) this.auftrag = null;
    const turn = a.turn;
    turn.auftrag = undefined;
    turn.cancelled = true;
    turn.finishStream?.();
    turn.denkerFristStop?.();
    this.sentenceHandlers.delete(turn.id);
    if (!turn.denkerFertig) {
      void turn.side?.cancel?.(turn.id);
      void invoke("voice_call_cancel", { requestId: turn.id }).catch(
        () => undefined,
      );
    }
    // Was das Tor in Rust von diesem Lauf gemerkt hat, darf „wiederholen“ nicht ausgeben.
    this.auftragAusgabe(a, false);
    this.cb.onDenken?.({ art: "ende", turnId: turn.id });
    if (merken) this.merkeAuftragFrage(a);
    // Wird das Ergebnis gerade nachgereicht, endet auch das: Gehörtes bleibt als unterbrochene Zeile, was nur
    // angezeigt war, verschwindet (`cancelTurn`). Der Schritt ist frei, die Anrufschleife hört wieder zu.
    if (a.nachreichend && this.turn?.nachreichen === a) {
      this.cancelTurn(true);
      if (!this.utt && this.inCall) this.setState("listening");
    }
  }

  /** Die verworfene Frage bleibt als unterbrochener Eintrag im Verlauf, damit der nächste Denker sie kennt. */
  private merkeAuftragFrage(a: Auftrag): void {
    const tag = this.historyTag;
    this.history.push({
      role: "user",
      content: a.frage,
      turnId: a.turnId,
      ...tag,
    });
    this.history.push({
      role: "assistant",
      content: `${speakable(a.gesagt)} …`.trim(),
      ...tag,
      interrupted: true,
    });
  }

  /** Das Ergebnis ist gesprochen oder unterbrochen: der Auftrag ist erledigt (kein Abbruch beim Denker nötig). */
  private auftragVerbraucht(a: Auftrag): void {
    if (a.verworfen) return;
    a.verworfen = true;
    clearTimeout(a.timer);
    if (this.auftrag === a) this.auftrag = null;
    a.turn.auftrag = undefined;
    // Das Ergebnis war zu hören (oder wurde unterbrochen): „wiederholen“ kennt es wieder.
    this.auftragAusgabe(a, true);
  }

  /**
   * Das Gedächtnis des Tors in Rust verlässt den Auftrag. `behalten`: die Engine übernimmt den Schlüssel (das
   * Ergebnis ist gesprochen oder unterbrochen, „wiederholen“ gibt es aus); sonst wird es vergessen (verworfen).
   */
  private auftragAusgabe(a: Auftrag, behalten: boolean): void {
    const key = a.ausgabe;
    a.ausgabe = null;
    if (key && !this.ausgabeKey) this.ausgabeKey = key;
    if (!behalten) this.ausgabeVergessen();
  }

  /** Frist des Auftrags: `auftragMaxMs` ab dem Start des Denkers, bis sein Ergebnis zu hören beginnt. */
  private starteFrist(a: Auftrag): void {
    clearTimeout(a.timer);
    const rest = TUNING.auftragMaxMs - (performance.now() - a.gestartet);
    a.timer = setTimeout(() => this.fristAbgelaufen(a), Math.max(0, rest));
  }

  /**
   * Die Frist des Auftrags ist um. Hat das Nachreichen schon zu klingen begonnen (der Beginn wird vom Player nur
   * auf Anfrage gemeldet), spricht es weiter. Sonst verfällt der Auftrag still: Ausgabe 0, der Schritt des
   * Nachreichens endet, späte Ergebnisse von Synthese und Player verhallen.
   */
  private fristAbgelaufen(a: Auftrag): void {
    if (a.verworfen) return;
    const lauf = this.turn?.nachreichen === a ? this.turn : null;
    if (lauf && this.warZuHoeren(lauf)) return;
    // Auch ein laufendes, noch stummes Nachreichen endet damit (Zeile, Verlauf, Player: `cancelTurn`).
    this.verwerfeAuftrag(a);
  }

  /** Das Nachreichen ist vor dem ersten Ton unterbrochen worden: der Auftrag wartet wieder, die Frist läuft weiter. */
  private zurueckgestellt(a: Auftrag): void {
    if (a.verworfen) return;
    a.nachreichend = false;
    this.starteFrist(a);
  }

  /** Der Denker eines offenen Auftrags ist zu Ende. */
  private auftragDenkerEnde(a: Auftrag): void {
    if (a.verworfen) return;
    // Fehler des Denkers oder nichts geliefert: still verwerfen, keine Vorlage, kein Verlauf.
    if (a.turn.denkerFehler || a.saetze.length === 0) {
      this.verwerfeAuftrag(a);
      return;
    }
    // Die Frist läuft weiter: auch ein fertiges, aber nie nachgereichtes Ergebnis verfällt (60 s ab Beginn).
    a.fertig = true;
    this.pruefeNachreichen();
  }

  /** Letztes Sprachende (Nutzer) bzw. Ende des Vordergrunds; `wall`: Wanduhrzeit, nie in der Zukunft. */
  private merkeStille(wall?: number): void {
    const jetzt = performance.now();
    this.stilleSeit = Math.max(this.stilleSeit, Math.min(wall ?? jetzt, jetzt));
  }

  /** Ist der Vordergrund frei: kein Schritt, keine offene Äußerung, kein Halten, Zustand „hört zu“? */
  private vordergrundFrei(): boolean {
    return (
      this.inCall &&
      this.player !== null &&
      this.turn === null &&
      this.utt === null &&
      this.held === null &&
      this.state === "listening"
    );
  }

  /**
   * Ist ein fertiges Ergebnis da und der Vordergrund frei, wird es nach `nachreichenStilleMs` Stille nachgereicht
   * (Stille: seit dem letzten Sprachende des Nutzers und seit dem Ende des Vordergrunds). Läuft bei jedem Wechsel
   * in „hört zu“ und wenn der Denker fertig wird.
   */
  private pruefeNachreichen(): void {
    clearTimeout(this.nachreichTimer);
    this.nachreichTimer = undefined;
    const a = this.auftrag;
    if (!a || a.verworfen || !a.fertig || a.nachreichend) return;
    if (!this.vordergrundFrei()) return;
    const rest =
      TUNING.nachreichenStilleMs - (performance.now() - this.stilleSeit);
    if (rest > 0) {
      this.nachreichTimer = setTimeout(() => this.pruefeNachreichen(), rest);
      return;
    }
    this.nachreichen(a);
  }

  /** Das Ergebnis eines Auftrags als eigener Schritt: eigene Assistentenzeile, gesprochen, mit Verlauf. */
  private nachreichen(a: Auftrag): void {
    a.nachreichend = true;
    // Die Frist läuft weiter, bis das erste Stück wirklich klingt (`nachreichenBegonnen`): eine Synthese oder ein
    // Player, der nie antwortet, hält den Auftrag nicht länger als `auftragMaxMs`.
    const turn = this.newTurn(this.frameIndex, performance.now());
    // Die Kennung des Auftrags: Verlauf und Zeile der Frage bleiben so einander zugeordnet.
    turn.id = a.turnId;
    turn.committed = true;
    turn.commit();
    turn.noMerge = true;
    turn.userText = a.frage;
    turn.nachreichen = a;
    if (a.turn.withContext) turn.withContext = true;
    this.turn = turn;
    this.setState("thinking");
    // Ein Fehler (etwa im Player) beendet das Nachreichen mit einer Meldung; er bleibt nie unbehandelt.
    void this.spieleNachgereicht(turn, a).catch((e) => {
      if (turn.cancelled || this.turn !== turn) return;
      this.cb.onError(String(e));
      this.auftragVerbraucht(a);
      // Was zu hören war, bleibt als unterbrochene Zeile; was nur angezeigt war, verschwindet mit dem Fehler.
      this.cancelTurn(true);
      this.dropTurn(turn);
    });
  }

  private async spieleNachgereicht(turn: Turn, a: Auftrag): Promise<void> {
    const dead = () => turn.cancelled || this.turn !== turn;
    let text = "";
    let kette: Promise<void> = Promise.resolve();
    turn.synthOffen = a.saetze.length;
    for (const piece of a.saetze) {
      const schritt = async () => {
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
        const player = this.player;
        if (dead() || !player) return;
        // Das nächste Stück ist da: die Wartezeit auf es (`warteAufStueck`) ist vorbei.
        clearTimeout(turn.stueckTimer);
        turn.stueckTimer = undefined;
        if (!turn.audible) {
          // Erst jetzt wird etwas hörbar: die Zeile entsteht, die Anzeige des Auftrags wird ihre.
          turn.audible = true;
          player.rearm();
          player.onFirstSound = () => this.setState("speaking");
          this.cb.onDenken?.({ art: "vordergrund", turnId: a.turnId });
        }
        text += (text ? " " : "") + piece;
        this.cb.onAssistant(text, false, false, a.turnId);
        turn.heard++;
        const rec: PieceRec = {
          text: piece,
          kind: "inhalt",
          ended: false,
          queued: false,
          started: false,
        };
        turn.pieces.push(rec);
        await player.enqueue(
          wav,
          () => {
            rec.ended = true;
            if (schliesstSatz(piece)) this.ersterSatzFertig(turn);
            else this.pruefeSatzende(turn);
          },
          () => {
            rec.started = true;
            this.nachreichenBegonnen(turn, a);
          },
        );
        rec.queued = true;
      };
      kette = kette.then(schritt).finally(() => this.stueckFertig(turn));
    }
    await kette;
    if (dead()) return;
    turn.done = true;
    this.pruefeSatzende(turn);
    if (turn.heard === 0) {
      // Nichts war hörbar (etwa die Synthese schlug fehl): kein Dauerversuch, der Auftrag ist erledigt.
      this.auftragVerbraucht(a);
      this.dropTurn(turn);
      return;
    }
    const tag = this.historyTag;
    this.history.push({
      role: "user",
      content: a.frage,
      turnId: a.turnId,
      ...tag,
    });
    this.history.push({
      role: "assistant",
      content: speakable(text),
      ...tag,
      ...(turn.withContext ? { ctx: true } : {}),
    });
    turn.historyPushed = true;
    this.schliesseAntwort(turn, text);
    // Die Quellen des Ergebnisses, wie bei einem gewöhnlichen Abschluss (nur der Text des Denkers), und erst mit
    // dem echten Beginn des Tons: ein zurückgestelltes Nachreichen meldet keine, nur der gültige Versuch.
    const quellen = () =>
      void this.reportCitations(a.turnId, text, a.turn.kontext ?? null, true);
    if (this.warZuHoeren(turn)) quellen();
    else turn.quellenMelden = quellen;
    if (!this.player?.active) this.finishTurn();
  }

  /**
   * Das erste Stück des Nachreichens klingt wirklich: die Frist des Auftrags endet (ein Ergebnis, das zu sprechen
   * begonnen hat, wird nicht abgeschnitten), die wartende Antwortzeile und die Quellen gehen jetzt hinaus.
   */
  private nachreichenBegonnen(turn: Turn, a: Auftrag): void {
    // Ein später Beginn nach dem Abbruch (zurückgestellt, verworfen) rührt die Frist nicht an: sie gehört seit der
    // Rückstellung wieder dem wartenden Auftrag. Meldungen eines abgebrochenen Schritts sind schon abgeräumt.
    if (!turn.cancelled) clearTimeout(a.timer);
    this.sendeAntwort(turn);
    this.quellenMeldenJetzt(turn);
  }

  /**
   * Der Player ist leer (gemeldet oder festgestellt). Ist die Antwort fertig erzeugt, endet der Schritt. Sonst
   * fehlt noch etwas: liegt ein Stück schon vor (seine Synthese läuft), wartet der Schritt höchstens
   * `stueckMaxMs` darauf; wartet er nur auf das Modell, gelten dessen Fristen.
   */
  private playerLeer(): void {
    const turn = this.turn;
    if (!turn?.audible) return;
    if (turn.done) this.finishTurn();
    else this.pruefeStueckwartezeit(turn);
  }

  /**
   * Läuft die Wartezeit auf das nächste Stück? Nur bei hörbarem Schritt, leerem Player und mindestens einem
   * offenen Stück; nicht in der Pause (`held`: die Entscheidung läuft, `releaseHold` nimmt sie danach wieder auf) und
   * nicht bei klingendem Ton. `behalten`: eine schon laufende Wartezeit nicht neu beginnen (ein weiteres Stück
   * kam hinzu, ohne dass der Player etwas gespielt hat), sonst beginnt sie von neuem.
   */
  private pruefeStueckwartezeit(turn: Turn, behalten = false): void {
    if (!turn.audible || turn.done || turn.cancelled || turn.synthOffen <= 0)
      return;
    if (this.turn !== turn || this.held === turn || this.player?.active) return;
    if (behalten && turn.stueckTimer !== undefined) return;
    this.warteAufStueck(turn);
  }

  /** Ein Stück ist erledigt (eingereiht, übersprungen, gescheitert oder verworfen): ohne offenes Stück keine Wartezeit. */
  private stueckFertig(turn: Turn): void {
    turn.synthOffen--;
    if (turn.synthOffen > 0) return;
    clearTimeout(turn.stueckTimer);
    turn.stueckTimer = undefined;
  }

  private warteAufStueck(turn: Turn): void {
    clearTimeout(turn.stueckTimer);
    turn.stueckTimer = setTimeout(
      () => this.stueckFrist(turn),
      TUNING.stueckMaxMs,
    );
  }

  /**
   * Das nächste Stück kam nicht: der Schritt endet wie ein Abbruch durch den Nutzer. Das Gehörte bleibt als
   * unterbrochene Zeile (beim Nachreichen mit der Kennung des Auftrags), der Rest ist verworfen, der Schritt ist
   * frei (das Modell wird abgebrochen), eine späte Synthese bleibt ohne Wirkung. Bei angehaltener Wiedergabe
   * (Entscheidung läuft) oder neuem Ton läuft nichts ab; die Pause verbraucht die Wartezeit nur, `releaseHold` und
   * der nächste Leerlauf setzen sie neu an.
   */
  private stueckFrist(turn: Turn): void {
    turn.stueckTimer = undefined;
    if (turn.cancelled || turn.done || this.turn !== turn) return;
    if (turn.synthOffen <= 0 || this.held === turn || this.player?.active)
      return;
    turn.abgelaufen = true;
    this.cancelTurn(true);
    if (!this.utt && this.inCall) this.setState("listening");
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
    // Das Nachreichen trägt die Kennung des Auftrags: dessen Anzeige gehört nicht ihm.
    if (!turn.nachreichen) this.cb.onDenken?.({ art: "ende", turnId: turn.id });
    this.merkeStille();
    if (!this.utt) this.setState("listening");
    this.pruefeNachreichen();
  }

  private finishTurn(): void {
    // Eine angehaltene Antwort endet nicht von selbst: erst die Entscheidung (`decideHeld`).
    if (this.held && this.held === this.turn) return;
    const turn = this.turn;
    const ende = turn?.onEnde;
    this.turn = null;
    if (turn) this.sendeAntwort(turn);
    ende?.();
    // Das Ergebnis eines nachgereichten Auftrags ist gesprochen: der Auftrag ist erledigt.
    if (turn?.nachreichen) this.auftragVerbraucht(turn.nachreichen);
    this.merkeStille();
    if (!this.utt) this.setState("listening");
    this.pruefeNachreichen();
  }

  /**
   * Laufenden Schritt verwerfen; `interrupted`: der Nutzer hat die KI unterbrochen. `merge`: die
   * Äußerung wird mit einer Ergänzung neu gestellt, der Schritt hinterlässt weder Verlauf noch Zeile.
   */
  private cancelTurn(interrupted: boolean, merge = false): void {
    const turn = this.turn;
    if (!turn) return;
    turn.cancelled = true;
    clearTimeout(turn.stueckTimer);
    turn.stueckTimer = undefined;
    turn.finishStream?.();
    this.turn = null;
    if (this.held === turn) {
      this.held = null;
      this.holdRun++;
      this.holdBusy = false;
    }
    this.globalContext.discard(turn.id);
    this.sentenceHandlers.delete(turn.id);
    turn.sofortStop?.();
    // Das Nachreichen hat keine Anfrage beim Denker (sein Lauf ist längst zu Ende).
    if (!turn.nachreichen) {
      void turn.side?.cancel?.(turn.id);
      void invoke("voice_call_cancel", { requestId: turn.id }).catch(
        () => undefined,
      );
    }
    // War vom Nachreichen noch nichts zu hören (Ton nie begonnen, auch wenn schon eingereiht), wird es nur
    // zurückgestellt und später erneut versucht; sonst ist es unterbrochen (Rest verworfen), der Auftrag erledigt.
    if (turn.nachreichen) {
      if (this.warZuHoeren(turn)) this.auftragVerbraucht(turn.nachreichen);
      else this.zurueckgestellt(turn.nachreichen);
    }
    this.merkeStille();
    this.cb.onStatus?.(null);
    if (!turn.nachreichen) this.cb.onDenken?.({ art: "ende", turnId: turn.id });
    turn.commit();
    const said = this.gehoert(turn)
      .filter((p) => p.kind === "inhalt")
      .map((p) => p.text);
    // Ein Nachreichen, von dem nichts zu hören war, hinterlässt nichts (Esc, Stoppwort, Wechsel, Fehler, Rückstellung):
    // die angezeigte Zeile mit dem Zwischenstand verschwindet, und seine Quellen werden nie gemeldet.
    if (turn.nachreichen && said.length === 0) {
      turn.quellenMelden = undefined;
      if (turn.audible) this.cb.onVerwerfen?.();
    }
    // Ein Nachreichen ohne Ton wird zurückgestellt: sein Verlaufseintrag geht zurück, der nächste Versuch legt ihn neu an.
    const zurueckgestellt =
      !!turn.nachreichen && said.length === 0 && turn.historyPushed;
    if (
      merge ||
      zurueckgestellt ||
      (interrupted && said.length === 0 && turn.historyPushed)
    ) {
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
      // Die Zeile zeigt alles Gehörte (auch das Weitersprechen davor), der Verlauf der KI nur den Inhalt.
      const angezeigt = this.gehoert(turn)
        .map((p) => p.text)
        .join(" ");
      this.cb.onAssistant(angezeigt, true, true, turn.nachreichen?.turnId);
    } else if (interrupted && !turn.nachreichen) {
      this.kuerzeAnzeige(turn);
    }
    this.player?.stop();
  }

  /**
   * Der Schritt endet unterbrochen, ohne dass Inhalt zu hören war (Vorlage wie „Ich wiederhole.“, Weitersprechen
   * des Sprechers, Ansage oder gar nichts): die angezeigte Zeile darf nichts Ungesprochenes behalten. Das Gehörte
   * bleibt als unterbrochene Zeile stehen; war nichts zu hören, verschwindet die noch offene Zeile. Eine schon
   * endgültige Zeile lässt sich nur kürzen, nicht zurücknehmen. (Mit Inhalt kürzt `cancelTurn` auf diesen.)
   */
  private kuerzeAnzeige(turn: Turn): void {
    if (!turn.angezeigt) return;
    const gehoert = this.gehoert(turn)
      .map((p) => p.text)
      .join(" ");
    if (gehoert) this.cb.onAssistant(gehoert, true, true);
    else if (!turn.finalSent) this.cb.onVerwerfen?.();
  }
}

function isPhantom(text: string, speechMs: number): boolean {
  if (text.length < 2) return true;
  if (/^[\s.,!?…–-]*$/.test(text)) return true;
  return speechMs < 1200 && PHANTOMS.test(text);
}

export { CAPTURE_RATE };
