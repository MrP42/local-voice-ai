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

export interface PageStateStoreOptions {
  io: PageStateIo;
  live: PageStateLive;
  /** Ein Protokollschreiben ist gesichert (für die Anzeige der offenen Seite). */
  onCommit?: (page: string, entries: PasteLogEntry[]) => void;
}

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

export function createPageStateStore(options: PageStateStoreOptions) {
  const { io, live } = options;
  /** Anzahl gesicherter Protokollschreiben je Seite (nur für die Anzeige). */
  const writes = new Map<string, number>();
  let tail: Promise<unknown> = Promise.resolve();
  /** Letzter eingereihter Auftrag je Seite: ein Lesen dieser Seite wartet darauf. */
  const pageTails = new Map<string, Promise<unknown>>();
  /** Snapshots verlassener Seiten, deren Sichern gescheitert ist: werden beim
   *  nächsten Lesen (Öffnen) der Seite erneut versucht, bis ein Sichern gelingt. */
  const failedSaves = new Map<string, string>();

  /** Auftrag in die Kette stellen. Ein Fehler bricht nur diesen Auftrag ab
   *  (er geht an den Aufrufer), die Kette läuft weiter. */
  function enqueue<T>(job: () => Promise<T>, page: string): Promise<T> {
    const run = tail.then(job, job);
    const settled = run.catch(() => undefined);
    tail = settled;
    pageTails.set(page, settled);
    void settled.then(() => {
      if (pageTails.get(page) === settled) pageTails.delete(page);
    });
    return run;
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

  /**
   * Seitenstand sichern (Autosave, Export, KI-Dokument). `snapshot` ist der
   * vorab gebaute Zustand. Wird er für die gerade OFFENE Seite eingereiht, wird
   * er erst beim Schreiben neu gebaut (neuester Text), solange sie offen ist.
   * Wird er für eine nicht offene (verlassene) Seite eingereiht, gilt immer der
   * Snapshot, auch wenn die Seite bis zur Ausführung wieder offen ist: er ist der
   * Stand beim Verlassen, die Oberfläche lädt danach erst hinter diesem Auftrag.
   * `einfuegungen` kommt in beiden Fällen aus der frisch gelesenen Datei.
   * Schreib- oder Lesefehler gehen an den Aufrufer. Scheitert das Sichern einer
   * nicht offenen Seite, bleibt der Snapshot für einen erneuten Versuch beim
   * nächsten Öffnen (`read`); ein später gelungenes Sichern ersetzt ihn.
   */
  function save(page: string, snapshot: string): Promise<void> {
    const openAtEnqueue = live.isLive(page);
    return enqueue(async () => {
      try {
        const text = await io.load(page);
        if (text === null) throw new Error("page state not read");
        const state = parseState(
          openAtEnqueue && live.isLive(page) ? live.build() : snapshot,
        );
        let body = snapshot;
        if (state) {
          state.einfuegungen = parseState(text)?.einfuegungen;
          body = JSON.stringify(state);
        }
        if (!(await io.save(page, body)))
          throw new Error("page state not saved");
        failedSaves.delete(page);
      } catch (error) {
        if (!openAtEnqueue) failedSaves.set(page, snapshot);
        throw error;
      }
    }, page);
  }

  /** Steht für die Seite ein gescheitertes Sichern aus (Snapshot beim Verlassen)? */
  const hasFailedSave = (page: string): boolean => failedSaves.has(page);

  /**
   * Seite lesen, hinter allen bereits eingereihten Aufträgen dieser Seite: wer
   * eine Seite (wieder) öffnet, sieht so immer den Stand nach allen ausstehenden
   * Schreibvorgängen, nicht den Dateistand von davor. Ein früher gescheitertes
   * Sichern dieser Seite wird zuerst erneut versucht. `mark` ist die Marke der
   * Anzeige VOR dem Lesen (siehe `changedSince`); `null` bei Lesefehler.
   */
  async function read(
    page: string,
  ): Promise<{ text: string; mark: number } | null> {
    const failed = failedSaves.get(page);
    if (failed !== undefined) await save(page, failed).catch(() => undefined);
    await pageTails.get(page);
    const mark = writes.get(page) ?? 0;
    const text = await io.load(page);
    return text === null ? null : { text, mark };
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
    return enqueue(async () => {
      if (refused()) return false;
      const text = await io.load(page);
      if (text === null || refused()) return false;
      const state = parseState(text) ?? options.unreadable?.() ?? null;
      const log = state?.einfuegungen;
      if (!state || !change(state, text)) return false;
      state.einfuegungen = log;
      return io.save(page, JSON.stringify(state));
    }, page);
  }

  return {
    commitLog,
    commitLogOutcome,
    addLogEntry,
    save,
    read,
    hasFailedSave,
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
