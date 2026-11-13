/**
 * Der eine Schreibpfad für `state.json` einer Seite (lva#530 I1, Codex Runde 7).
 *
 * Alles, was den Seitenzustand schreibt (Autosave, Rohfassungen sichern oder
 * löschen, Migration, Export, KI-Dokument, Bibliothek), läuft als Auftrag durch
 * EINE Kette, nacheinander. Jeder Auftrag liest die Datei frisch, ändert nur
 * das, was ihm gehört, und schreibt sie zurück:
 *
 * - Die Datei ist die EINZIGE Quelle für die Rohfassungen (`einfuegungen`).
 *   Es gibt keinen Zwischenspeicher, der beim Schreiben etwas ersetzt; was ein
 *   anderer Schreiber (Sync, Bibliothek) zwischendurch an das Feld gebracht hat,
 *   bleibt erhalten, was gelöscht wurde, kommt nicht wieder.
 * - Protokolländerungen (`commitLog`) sind Operationen auf dem frisch gelesenen
 *   Protokoll (hinzufügen, löschen, alle löschen). Sie ändern nur dieses Feld:
 *   alles andere (Text usw.) bleibt der Dateistand, nie der der Oberfläche.
 *   Das Hinzufügen (`addLogEntry`) prüft den Platz in derselben Kette und
 *   meldet „voll“, statt etwas zu verdrängen.
 * - `save` (Autosave, Export, KI-Dokument) baut den Zustand wie bisher, setzt
 *   aber `einfuegungen` aus der frisch gelesenen Datei ein.
 * - `mutate` ändert Felder der Datei; `einfuegungen` bleibt der Dateiwert.
 *
 * Scheitert das Sichern eines Auftrags für eine Seite, die nicht (mehr) offen ist
 * (Verlassen, Autosave beim Seitenwechsel), hinterlegt der Auftrag SEINEN
 * geschriebenen Stand als Rettungssnapshot (`failedSaves`). Wer die Seite öffnet
 * (`read`), wartet zuerst auf alle Aufträge dieser Seite und prüft erst danach,
 * ob ein Snapshot aussteht. Dann gilt:
 *
 * - Hat seither kein anderer Auftrag den Seiteninhalt geschrieben (Laufnummern
 *   `seq`: `save`/`mutate`, nicht das Protokoll, das ein Snapshot nie ersetzt),
 *   wird der Snapshot erneut geschrieben. Gelingt das, liest `read` die Datei.
 *   Scheitert es, liefert `read` den Snapshot als UNGESPEICHERTEN Stand
 *   (`unsaved`), nie die ältere Datei; erst ein gelungenes Schreiben der
 *   Oberfläche (Lade-Autosave) löscht ihn.
 * - Hat inzwischen ein anderer Auftrag einen neueren Stand geschrieben (auch ein
 *   wartendes `save`), wird der Snapshot NIE blind zurückgeschrieben. Sein Text
 *   wird als eigene Fassung gesichert (`io.saveCopy`) oder, scheitert das, im
 *   Speicher gehalten (`held`) und gemeldet (`onRescue`); die Datei bleibt.
 *
 * Für die Anzeige gibt es eine Marke je Seite: Ein Ladevorgang, der gelesen hat,
 * bevor ein Protokollschreiben fertig wurde, darf die Anzeige nicht auf den
 * älteren Stand zurücksetzen (`mark`/`changedSince`). Sie betrifft nur die
 * Anzeige, nie das Schreiben.
 *
 * Reine Logik ohne Tauri und React: Ein-/Ausgabe und die Sicht auf die offene
 * Seite kommen von außen (`io`, `live`), damit die Rennen in Tests
 * deterministisch nachgestellt werden können. Inhalte gehen nie ins Log.
 */
import {
  appendPasteEntry,
  parsePasteLog,
  type PasteLogEntry,
} from "./pasteLog.ts";

export interface PageStateIo {
  /** Rohtext von `state.json` (leer, wenn die Seite noch keinen hat); `null` bei Fehler. */
  load(page: string): Promise<string | null>;
  /** `true`, wenn geschrieben. */
  save(page: string, json: string): Promise<boolean>;
  /** Text als eigene Fassung der Seite sichern (nicht in `state.json`); `true`,
   *  wenn gesichert. Fehlt sie, bleibt ein verdrängter Stand im Speicher. */
  saveCopy?(page: string, text: string): Promise<boolean>;
}

export interface PageStateLive {
  /** Geöffnet UND fertig geladen: dann baut `save` den Zustand aus der Oberfläche. */
  isLive(page: string): boolean;
  /** Ganzer Seitenzustand der geöffneten Seite (ohne maßgebliches Protokoll). */
  build(): string;
}

/** Die Operation verweigert die Änderung (z. B. Protokoll voll): nichts wird geschrieben. */
export interface LogRefusal {
  ok: false;
  reason: "full" | "unfit";
}

/** Operation auf dem frisch gelesenen Protokoll: hinzufügen, löschen, alle
 *  löschen. Eine Verweigerung ändert nichts und schreibt nichts. */
export type LogOp = (log: PasteLogEntry[]) => PasteLogEntry[] | LogRefusal;

/** Ausgang eines Protokollauftrags: gesichert, Lese-/Schreibfehler oder verweigert. */
export type LogOutcome = "saved" | "failed" | LogRefusal["reason"];

/** Wie ein verdrängter, nicht gespeicherter Stand gesichert wurde: als eigene
 *  Fassung (`copy`) oder nur im Speicher gehalten (`held`, geht beim Beenden verloren). */
export type RescueKind = "copy" | "held";

export interface PageStateStoreOptions {
  io: PageStateIo;
  live: PageStateLive;
  /** Ein Protokollschreiben ist gesichert (für die Anzeige der offenen Seite). */
  onCommit?: (page: string, entries: PasteLogEntry[]) => void;
  /** Ein älterer, nicht gespeicherter Stand wurde gesichert oder gehalten. */
  onRescue?: (page: string, kind: RescueKind) => void;
}

/** Ergebnis von `read`: Rohtext der Seite, Marke der Anzeige und ob der Stand
 *  noch NICHT in der Datei steht (Rettungssnapshot einer verlassenen Seite). */
export interface PageRead {
  text: string;
  mark: number;
  unsaved: boolean;
}

/** Ein Schreibauftrag ist gescheitert. `kept`: der Store hält den Stand dieses
 *  Auftrags als Rettungssnapshot und bietet ihn beim nächsten Öffnen an. */
export class PageSaveError extends Error {
  readonly kept: boolean;
  constructor(message: string, kept: boolean) {
    super(message);
    this.name = "PageSaveError";
    this.kept = kept;
  }
}

/** Rettungssnapshot: der Zustand, den ein gescheiterter Auftrag schreiben wollte. */
interface FailedSave {
  /** Gebauter Zustand (ohne `einfuegungen`, die kommen aus der Datei). */
  snapshot: string;
  /** Laufnummer des gescheiterten Auftrags: jeder spätere Schreibauftrag ist neuer. */
  seq: number;
  /** `read` hat ihn der Oberfläche als ungespeicherten Stand geliefert. */
  handedOut: boolean;
  /** Text der Datei, der berücksichtigt ist: der, auf dem der Snapshot beruht
   *  (beim Scheitern gelesen; `null`, wenn die Datei nicht lesbar war), oder ein
   *  schon gesicherter. Nur ein davon abweichender Dateitext gilt als neuer. */
  known: string | null;
}

/** Wie oft `read` einen Snapshot erneut versucht (jeder höchstens einmal). */
const MAX_READ_ROUNDS = 4;

type StateObject = Record<string, unknown>;

/** `state.json` lesen: leer ist `{}`, Unlesbares (Altformat, Rohtext) `null`. */
function parseState(text: string): StateObject | null {
  if (text.trim() === "") return {};
  try {
    const value: unknown = JSON.parse(text);
    return typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as StateObject)
      : null;
  } catch {
    return null;
  }
}

/** `einfuegungen` eines gelesenen Zustands; unlesbar oder fehlend ist leer. */
export const logOfState = (text: string): PasteLogEntry[] =>
  parsePasteLog(parseState(text)?.einfuegungen);

/** Textfeld eines gelesenen Zustands; Altformat (reiner Text) ist der Text selbst. */
const textField = (json: string): string => {
  const state = parseState(json);
  if (!state) return json;
  return typeof state.text === "string" ? state.text : "";
};

/** `built` (Zustand ohne Protokoll) mit dem Protokoll der gelesenen Datei. */
function withFileLog(built: string, fileText: string): string {
  const state = parseState(built);
  if (!state) return built;
  state.einfuegungen = parseState(fileText)?.einfuegungen;
  return JSON.stringify(state);
}

export function createPageStateStore(options: PageStateStoreOptions) {
  const { io, live } = options;
  /** Anzahl gesicherter Protokollschreiben je Seite (nur für die Anzeige). */
  const writes = new Map<string, number>();
  let tail: Promise<unknown> = Promise.resolve();
  /** Letzter eingereihter Auftrag je Seite: ein Lesen dieser Seite wartet darauf. */
  const pageTails = new Map<string, Promise<unknown>>();
  /** Laufnummer der Aufträge, in Reihenfolge des Einreihens (= der Ausführung). */
  let seqCounter = 0;
  /** Laufnummer des letzten Auftrags, der den Seiteninhalt (`save`, `mutate`,
   *  Retry) erfolgreich geschrieben hat. Das Protokoll zählt nicht: ein Snapshot
   *  ersetzt `einfuegungen` nie, also kann ihn kein Protokollschreiben veralten. */
  const lastWrite = new Map<string, number>();
  /** Rettungssnapshots: der Stand, den ein gescheiterter Auftrag für eine nicht
   *  (mehr) offene Seite schreiben wollte. Wird beim Öffnen (`read`) erneut
   *  geschrieben, solange nichts Neueres gelungen ist, und erst nach einem
   *  gelungenen Schreiben dieses oder eines neueren Stands gelöscht. */
  const failedSaves = new Map<string, FailedSave>();
  /** Verdrängte ältere Stände, die sich nicht als Fassung sichern ließen: nur im
   *  Speicher, `read` versucht es erneut. */
  const heldTexts = new Map<string, string[]>();

  /** Auftrag in die Kette stellen. Ein Fehler bricht nur diesen Auftrag ab
   *  (er geht an den Aufrufer), die Kette läuft weiter. */
  function enqueue<T>(
    job: (seq: number) => Promise<T>,
    page: string,
  ): Promise<T> {
    const seq = (seqCounter += 1);
    const run = tail.then(
      () => job(seq),
      () => job(seq),
    );
    const settled = run.catch(() => undefined);
    tail = settled;
    pageTails.set(page, settled);
    void settled.then(() => {
      if (pageTails.get(page) === settled) pageTails.delete(page);
    });
    return run;
  }

  /** Wartet, bis kein eingereihter Auftrag dieser Seite mehr aussteht (auch
   *  nicht einer, der währenddessen dazukam). */
  async function settle(page: string): Promise<void> {
    for (let p = pageTails.get(page); p; p = pageTails.get(page)) await p;
  }

  /**
   * Protokoll der Seite ändern und sichern. `op` wird auf das frisch gelesene
   * Protokoll der Datei angewendet; geschrieben wird die frisch gelesene Datei
   * mit ersetztem Feld `einfuegungen`, sonst nichts. Verweigert `op` (Protokoll
   * voll), bleibt die Datei unberührt und der Grund geht an den Aufrufer.
   */
  function commitLogOutcome(page: string, op: LogOp): Promise<LogOutcome> {
    return enqueue(async () => {
      const text = await io.load(page);
      if (text === null) return "failed";
      const state = parseState(text);
      if (!state) return "failed";
      const next = op(parsePasteLog(state.einfuegungen));
      if (!Array.isArray(next)) return next.reason;
      state.einfuegungen = next.length > 0 ? next : undefined;
      if (!(await io.save(page, JSON.stringify(state)))) return "failed";
      writes.set(page, (writes.get(page) ?? 0) + 1);
      options.onCommit?.(page, next);
      return "saved";
    }, page);
  }

  /** Wie {@link commitLogOutcome}; `true`, wenn gesichert. */
  const commitLog = async (page: string, op: LogOp): Promise<boolean> =>
    (await commitLogOutcome(page, op)) === "saved";

  /**
   * Rohfassung anhängen, atomar mit der Prüfung auf Platz: ist die Grenze
   * erreicht, meldet der Auftrag `full`, statt etwas zu verdrängen. Der Editor
   * hat den Rohtext zu diesem Zeitpunkt schon eingefügt und bereinigt ihn erst
   * nach diesem Ergebnis, und nur bei `saved`.
   */
  const addLogEntry = (
    page: string,
    entry: PasteLogEntry,
  ): Promise<LogOutcome> =>
    commitLogOutcome(page, (log) => {
      const added = appendPasteEntry(log, entry);
      return added.ok ? added.log : added;
    });

  /** Verdrängten Text nur im Speicher halten UND melden: jeder verdrängte Text
   *  wird entweder dauerhaft gesichert oder ausdrücklich als „nur im Speicher“
   *  gemeldet, nie stillschweigend gehalten. */
  function hold(page: string, text: string): void {
    const list = heldTexts.get(page) ?? [];
    if (!list.includes(text)) heldTexts.set(page, [...list, text]);
    options.onRescue?.(page, "held");
  }

  /** Gehaltene Texte der Seite als Fassung sichern; was gelingt, wird frei. Bleibt
   *  etwas übrig (Sichern scheitert oder es gibt keinen Schreiber), wird das
   *  erneut gemeldet: der Nutzer erfährt beim Öffnen, dass es nur im Speicher liegt. */
  async function flushHeld(page: string): Promise<void> {
    const list = heldTexts.get(page);
    if (!list) return;
    const left: string[] = [];
    let copied = 0;
    for (const text of list) {
      const ok = io.saveCopy
        ? await io.saveCopy(page, text).catch(() => false)
        : false;
      if (ok) copied += 1;
      else left.push(text);
    }
    // Erst den Bestand setzen, dann melden: eine Meldung sieht den Endstand
    // (`held`), und die Warnung über übrig gebliebene Texte kommt zuletzt.
    if (left.length === 0) heldTexts.delete(page);
    else heldTexts.set(page, left);
    if (copied > 0) options.onRescue?.(page, "copy");
    if (left.length > 0) options.onRescue?.(page, "held");
  }

  /** Text als eigene Fassung sichern; gelingt das nicht, im Speicher halten und
   *  melden. Die Datei bleibt unberührt. */
  async function secureText(page: string, text: string): Promise<void> {
    const ok = io.saveCopy
      ? await io.saveCopy(page, text).catch(() => false)
      : false;
    if (ok) options.onRescue?.(page, "copy");
    else hold(page, text);
  }

  /**
   * Ein Snapshot, der nicht mehr geschrieben werden darf, weil die Datei einen
   * neueren Stand hat (`fileText`): sein Text wird als eigene Fassung gesichert
   * oder im Speicher gehalten und gemeldet. Gleicher oder leerer Text: nichts zu
   * retten. Die Datei bleibt immer unberührt.
   */
  async function rescue(
    page: string,
    entry: FailedSave,
    fileText: string,
  ): Promise<void> {
    if (failedSaves.get(page) === entry) failedSaves.delete(page);
    const own = textField(entry.snapshot);
    if (own.trim() === "" || own === textField(fileText)) return;
    await secureText(page, own);
  }

  /**
   * Die Oberfläche hält einen ausgehändigten, ungespeicherten Snapshot, während
   * die Datei inzwischen einen NEUEREN Stand hat (ein `mutate` ist nach dem
   * Scheitern gelungen, z. B. Bibliothek). Ihr nächstes `save` (Lade-Autosave,
   * Export, KI-Dokument) überschreibt die Datei mit dem Stand der Oberfläche:
   * deren eigener, noch ungesicherter Text hat Vorrang, wie beim Übernehmen nach
   * KeepAlive. Damit der Dateitext dabei nicht still verschwindet, wird er vorher
   * als eigene Fassung gesichert (sonst gehalten und gemeldet). Beide Texte
   * bleiben erhalten. Gleiche oder bekannte Texte brauchen nichts.
   */
  async function secureOverwrittenFile(
    page: string,
    fileText: string,
    built: string,
  ): Promise<void> {
    const entry = failedSaves.get(page);
    if (!entry?.handedOut) return;
    if ((lastWrite.get(page) ?? 0) <= entry.seq) return;
    const onFile = textField(fileText);
    if (
      onFile.trim() === "" ||
      onFile === entry.known ||
      onFile === textField(built)
    )
      return;
    entry.known = onFile;
    await secureText(page, onFile);
  }

  /** Rettungssnapshot für `page` hinterlegen. Ein noch nicht angezeigter älterer
   *  Snapshot mit anderem Text wird dabei nicht verdrängt, sondern gehalten. */
  function keep(
    page: string,
    built: string,
    seq: number,
    base: string | null,
  ): void {
    const older = failedSaves.get(page);
    if (older && !older.handedOut) {
      const olderText = textField(older.snapshot);
      if (olderText.trim() !== "" && olderText !== textField(built))
        hold(page, olderText);
    }
    failedSaves.set(page, {
      snapshot: built,
      seq,
      handedOut: false,
      known: base,
    });
  }

  /**
   * Nach einem erfolgreichen Schreiben des Seiteninhalts (`save`, `mutate`):
   * Laufnummer merken und einen ausstehenden Snapshot entscheiden. Hat die
   * Oberfläche ihn erhalten (`handedOut`), ist ein `save` ihr Nachfolger-Stand
   * und ersetzt ihn. Sonst ist der Snapshot jetzt veraltet (die Datei hat einen
   * neueren Stand ohne ihn): sichern statt später blind zurückzuschreiben.
   */
  async function afterWrite(
    page: string,
    seq: number,
    kind: "save" | "mutate",
    written: string,
  ): Promise<void> {
    lastWrite.set(page, seq);
    const entry = failedSaves.get(page);
    if (!entry) return;
    if (entry.handedOut) {
      if (kind === "save") failedSaves.delete(page);
      return;
    }
    await rescue(page, entry, written);
  }

  /**
   * Seitenstand sichern (Autosave, Export, KI-Dokument). `snapshot` ist der
   * vorab gebaute Zustand. Wird er für die gerade OFFENE Seite eingereiht, wird
   * er erst beim Schreiben neu gebaut (neuester Text), solange sie offen ist.
   * Wird er für eine nicht offene (verlassene) Seite eingereiht, gilt immer der
   * Snapshot, auch wenn die Seite bis zur Ausführung wieder offen ist: er ist der
   * Stand beim Verlassen, die Oberfläche lädt danach erst hinter diesem Auftrag.
   * `einfuegungen` kommt in beiden Fällen aus der frisch gelesenen Datei.
   * Schreib- oder Lesefehler gehen als {@link PageSaveError} an den Aufrufer.
   * Scheitert der Auftrag und ist die Seite dann nicht (mehr) offen, hinterlegt
   * er SEINEN geschriebenen Stand als Rettungssnapshot (`kept`), auch wenn er
   * als Autosave der damals offenen Seite begonnen hat; ist sie noch offen, hält
   * die Oberfläche den Stand und es bleibt kein Snapshot.
   */
  function save(page: string, snapshot: string): Promise<void> {
    const openAtEnqueue = live.isLive(page);
    return enqueue(async (seq) => {
      let built = snapshot;
      let fileText: string | null = null;
      let failure: unknown = null;
      try {
        fileText = await io.load(page);
        if (fileText === null) throw new Error("page state not read");
        built = openAtEnqueue && live.isLive(page) ? live.build() : snapshot;
        await secureOverwrittenFile(page, fileText, built).catch(
          () => undefined,
        );
        if (!(await io.save(page, withFileLog(built, fileText))))
          throw new Error("page state not saved");
      } catch (error) {
        failure = error;
      }
      if (failure === null && fileText !== null) {
        await afterWrite(page, seq, "save", withFileLog(built, fileText)).catch(
          () => undefined,
        );
        return;
      }
      const kept = !openAtEnqueue || !live.isLive(page);
      if (kept)
        keep(page, built, seq, fileText === null ? null : textField(fileText));
      throw new PageSaveError(
        failure instanceof Error ? failure.message : "page state not saved",
        kept,
      );
    }, page);
  }

  /** Steht für die Seite ein gescheitertes Sichern aus (Rettungssnapshot)? */
  const hasFailedSave = (page: string): boolean => failedSaves.has(page);

  /**
   * Ausstehenden Snapshot der Seite entscheiden (in der Kette): ist seither kein
   * Seiteninhalt geschrieben worden, erneut schreiben (`einfuegungen` aus der
   * Datei); sonst als Fassung sichern. Scheitern lässt ihn stehen.
   */
  function resolveFailed(page: string): Promise<void> {
    return enqueue(async (seq) => {
      const entry = failedSaves.get(page);
      if (!entry) return;
      const text = await io.load(page);
      if (text === null) return;
      if ((lastWrite.get(page) ?? 0) > entry.seq) {
        await rescue(page, entry, text);
        return;
      }
      if (!(await io.save(page, withFileLog(entry.snapshot, text)))) return;
      lastWrite.set(page, seq);
      if (failedSaves.get(page) === entry) failedSaves.delete(page);
    }, page);
  }

  /**
   * Seite lesen, hinter allen eingereihten Aufträgen dieser Seite: wer eine
   * Seite (wieder) öffnet, sieht so immer den Stand nach allen ausstehenden
   * Schreibvorgängen, nicht den Dateistand von davor. ERST DANACH wird geprüft,
   * ob ein Rettungssnapshot aussteht (ein Flush kann auch erst nach dem Beginn
   * dieses Lesens scheitern); er wird entschieden (siehe `resolveFailed`),
   * jeder Snapshot höchstens einmal je Lesen. Bleibt er ausstehend (erneut
   * gescheitert), liefert `read` ihn als ungespeicherten Stand statt der älteren
   * Datei, auch auf der bereits geladenen (offenen) Seite, die ihn schon erhalten
   * hat (KeepAlive): er behält Vorrang vor der Datei, solange er nicht gespeichert
   * ist. Dort schreibt `read` ihn nicht (der Stand gehört der Oberfläche); wer
   * `unsaved` bekommt, ersetzt den Stand der Oberfläche nicht.
   * `mark` ist die Marke der Anzeige VOR dem Lesen (siehe `changedSince`);
   * `null` bei Lesefehler.
   */
  async function read(page: string): Promise<PageRead | null> {
    const tried = new Set<FailedSave>();
    let pending: FailedSave | undefined;
    await settle(page);
    if (heldTexts.has(page))
      await enqueue(() => flushHeld(page), page).catch(() => undefined);
    for (let round = 0; ; round += 1) {
      await settle(page);
      pending = failedSaves.get(page);
      if (!pending) break;
      // Auf der geladenen Seite gehört der Stand der Oberfläche: kein erneutes
      // Schreiben, der Snapshot bleibt aber maßgeblich (unten).
      if (pending.handedOut && live.isLive(page)) break;
      if (tried.has(pending) || round >= MAX_READ_ROUNDS) break;
      tried.add(pending);
      await resolveFailed(page).catch(() => undefined);
    }
    const mark = writes.get(page) ?? 0;
    const text = await io.load(page);
    if (text === null) return null;
    if (!pending || failedSaves.get(page) !== pending)
      return { text, mark, unsaved: false };
    pending.handedOut = true;
    return { text: withFileLog(pending.snapshot, text), mark, unsaved: true };
  }

  /**
   * Felder einer Seite in der Datei ändern, ohne das Protokoll anzufassen
   * (Migration der alten Zusammenfassung, Sprecher ersetzen, Fassungen,
   * Bibliothek): in der Kette frisch lesen, `change` ändert das Objekt
   * (`false` = nichts tun; der zweite Parameter ist der gelesene Rohtext, z. B.
   * um eine leere Datei zu erkennen), zurückschreiben. `einfuegungen` bleibt
   * der frisch gelesene Dateiwert, auch wenn `change` sonst alles ersetzt. Der Zustand einer offenen Seite spielt keine Rolle: wie
   * ein Direktschreiben in die Datei, nur eingereiht. Nur die Migration der
   * Oberfläche selbst setzt `refuseOpen`: dort gilt die Oberfläche, eine
   * inzwischen offene Seite wird nicht geschrieben (`false`). `unreadable`
   * liefert den Grundzustand, wenn die Datei kein Objekt enthält (Altformat):
   * die Datei wird dann damit überschrieben, wie es der Aufrufer vorher selbst
   * getan hätte. Ein Lesefehler (`null`) schreibt nie.
   */
  function mutate(
    page: string,
    change: (state: StateObject, raw: string) => boolean,
    options: {
      refuseOpen?: boolean;
      unreadable?: () => StateObject;
    } = {},
  ): Promise<boolean> {
    const refused = () => options.refuseOpen === true && live.isLive(page);
    return enqueue(async (seq) => {
      if (refused()) return false;
      const text = await io.load(page);
      if (text === null || refused()) return false;
      const state = parseState(text) ?? options.unreadable?.() ?? null;
      const log = state?.einfuegungen;
      if (!state || !change(state, text)) return false;
      state.einfuegungen = log;
      const body = JSON.stringify(state);
      if (!(await io.save(page, body))) return false;
      await afterWrite(page, seq, "mutate", body).catch(() => undefined);
      return true;
    }, page);
  }

  return {
    commitLog,
    commitLogOutcome,
    addLogEntry,
    save,
    read,
    hasFailedSave,
    /** Verdrängte ältere Stände, die nur im Speicher liegen (Sichern als Fassung gescheitert). */
    held: (page: string): string[] => heldTexts.get(page) ?? [],
    mutate,
    /** Marke der Anzeige: vor dem Lesen einer Seite holen. */
    mark: (page: string): number => writes.get(page) ?? 0,
    /** Hat seit `mark` ein Protokollschreiben die Anzeige neu gesetzt? Dann
     *  zeigt ein später eintreffender Ladevorgang einen älteren Stand. */
    changedSince: (page: string, mark: number): boolean =>
      (writes.get(page) ?? 0) !== mark,
  };
}

export type PageStateStore = ReturnType<typeof createPageStateStore>;
