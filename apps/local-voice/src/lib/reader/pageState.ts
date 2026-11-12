/**
 * Der eine Schreibpfad für `state.json` einer Seite (lva#530 I1, Codex Runde 5).
 *
 * Alles, was den Seitenzustand schreibt (Autosave, Rohfassungen sichern oder
 * löschen, Migration, Export, KI-Dokument), läuft als Auftrag durch EINE Kette,
 * nacheinander. Die Rohfassungen (`einfuegungen`) haben genau eine maßgebliche
 * Quelle: das „gesicherte Protokoll“ je Seite. Es wird nur durch ein
 * erfolgreiches Protokollschreiben erhöht (`version` steigt, `entries` ist der
 * gesicherte Stand). Ladevorgänge ergänzen es nur, wenn noch nichts vorliegt,
 * und setzen es nie auf einen älteren Stand zurück. Jeder Schreiber setzt
 * `einfuegungen` erst beim tatsächlichen Schreiben aus dieser Quelle ein, egal
 * ob die Seite gerade offen ist; so kann kein Schreiber eine gesicherte
 * Rohfassung mit einem älteren Stand überschreiben.
 *
 * Reine Logik ohne Tauri und React: Ein-/Ausgabe und die Sicht auf die offene
 * Seite kommen von außen (`io`, `live`), damit die Rennen in Tests
 * deterministisch nachgestellt werden können. Inhalte gehen nie ins Log.
 */
import { parsePasteLog, type PasteLogEntry } from "./pasteLog.ts";

export interface PageStateIo {
  /** Rohtext von `state.json` (leer, wenn die Seite noch keinen hat); `null` bei Fehler. */
  load(page: string): Promise<string | null>;
  /** `true`, wenn geschrieben. */
  save(page: string, json: string): Promise<boolean>;
}

export interface PageStateLive {
  /** Ist `page` die geöffnete Seite? */
  isActive(page: string): boolean;
  /** Geöffnet UND fertig geladen: dann ist der Zustand der Oberfläche
   *  maßgeblich für alle Felder außer dem Protokoll. */
  isLive(page: string): boolean;
  /** Ganzer Seitenzustand der geöffneten Seite mit genau diesem Protokoll. */
  build(entries: readonly PasteLogEntry[]): string;
  /** Die Datei der offenen Seite wurde von außen geändert: Oberfläche neu
   *  laden (sperrt bis dahin das Speichern der Oberfläche). */
  reload?(page: string): void;
}

/**
 * Was `mutate` mit einer gerade offenen Seite tut:
 * - `refuse`: nichts schreiben (`false`); für Aufrufer, die die offene Seite
 *   selbst über die Oberfläche ändern.
 * - `keep`: auf dem Zustand der Oberfläche schreiben; die Oberfläche kennt die
 *   Änderung schon oder wird vom Aufrufer benachrichtigt.
 * - `adopt`: wie `keep`, danach lädt die Oberfläche die Seite neu.
 */
export type OpenPageMode = "refuse" | "keep" | "adopt";

export interface CommittedLog {
  /** Steigt mit jedem erfolgreich gesicherten Protokollschreiben. */
  version: number;
  entries: PasteLogEntry[];
}

export interface PageStateStoreOptions {
  io: PageStateIo;
  live: PageStateLive;
  /** Ein Protokollschreiben ist gesichert (für die Anzeige der offenen Seite). */
  onCommit?: (page: string, log: CommittedLog) => void;
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

/** Zustand mit gesetztem Protokoll (ein leeres lässt das Feld weg); `null`, wenn unlesbar. */
function withLog(
  text: string,
  entries: readonly PasteLogEntry[],
): string | null {
  const state = parseState(text);
  if (!state) return null;
  state.einfuegungen = entries.length > 0 ? entries : undefined;
  return JSON.stringify(state);
}

export function createPageStateStore(options: PageStateStoreOptions) {
  const { io, live } = options;
  const committed = new Map<string, CommittedLog>();
  const pending = new Map<string, number>();
  let tail: Promise<unknown> = Promise.resolve();

  /** Auftrag für `page` in die Kette stellen. Ein Fehler bricht nur diesen
   *  Auftrag ab (er geht an den Aufrufer), die Kette läuft weiter. */
  function enqueue<T>(page: string, job: () => Promise<T>): Promise<T> {
    pending.set(page, (pending.get(page) ?? 0) + 1);
    const run = tail.then(job, job);
    tail = run.catch(() => undefined);
    const done = () => {
      const left = (pending.get(page) ?? 1) - 1;
      if (left > 0) {
        pending.set(page, left);
        return;
      }
      pending.delete(page);
      // Verlassene Seite ohne wartenden Auftrag: die Datei enthält alles Gesicherte.
      if (!live.isActive(page)) committed.delete(page);
    };
    void run.then(done, done);
    return run;
  }

  /** Das gesicherte Protokoll aus einem Ladevorgang ergänzen, wenn noch keines
   *  vorliegt; nie überschreiben. Liefert die maßgebliche Fassung. */
  function seed(page: string, entries: PasteLogEntry[]): CommittedLog {
    let log = committed.get(page);
    if (!log) {
      log = { version: 0, entries };
      committed.set(page, log);
    }
    return log;
  }

  /** Protokolle verlassener Seiten ohne wartenden Auftrag verwerfen. */
  function release(): void {
    for (const page of [...committed.keys()])
      if (!live.isActive(page) && !pending.has(page)) committed.delete(page);
  }

  /**
   * Protokoll der Seite ändern und sichern; `true`, wenn gesichert. `change`
   * bekommt das gesicherte Protokoll (bei einer noch nie gesehenen Seite wird
   * es in der Kette frisch aus der Datei gelesen). Auf der offenen Seite wird
   * der ganze Zustand der Oberfläche geschrieben, sonst die frisch gelesene
   * Datei mit ersetztem Feld.
   */
  function commitLog(
    page: string,
    change: (log: PasteLogEntry[]) => PasteLogEntry[],
  ): Promise<boolean> {
    return enqueue(page, async () => {
      let fileText: string | null = null;
      let base = committed.get(page);
      if (!base) {
        fileText = await io.load(page);
        if (fileText === null) return false;
        base = seed(page, logOfState(fileText));
      }
      const next = change(base.entries);
      let body: string | null;
      if (live.isLive(page)) body = live.build(next);
      else {
        fileText ??= await io.load(page);
        if (fileText === null) return false;
        body = live.isLive(page) ? live.build(next) : withLog(fileText, next);
      }
      if (body === null) return false;
      if (!(await io.save(page, body))) return false;
      const log: CommittedLog = { version: base.version + 1, entries: next };
      committed.set(page, log);
      options.onCommit?.(page, log);
      return true;
    });
  }

  /**
   * Seitenstand sichern (Autosave, Export, KI-Dokument). `snapshot` ist der
   * vorab gebaute Zustand; auf der offenen Seite wird er erst beim Schreiben
   * neu gebaut (neuester Text), sonst gilt er. In beiden Fällen kommt das
   * Protokoll aus dem gesicherten Stand. Schreibfehler gehen an den Aufrufer.
   */
  function save(page: string, snapshot: string): Promise<void> {
    return enqueue(page, async () => {
      const log = committed.get(page);
      let body = snapshot;
      if (live.isLive(page))
        body = live.build(log?.entries ?? logOfState(snapshot));
      else if (log) body = withLog(snapshot, log.entries) ?? snapshot;
      if (!(await io.save(page, body))) throw new Error("page state not saved");
    });
  }

  /**
   * Felder einer Seite ändern, ohne das Protokoll anzufassen (Migration der
   * alten Zusammenfassung, Sprecher ersetzen, Fassungen, Bibliothek): in der
   * Kette frisch lesen, `change` ändert das Objekt (`false` = nichts tun),
   * zurückschreiben. `einfuegungen` kommt aus dem gesicherten Stand, sonst aus
   * der frisch gelesenen Datei, nie aus einem älteren Lesen des Aufrufers.
   * Ist die Seite offen, entscheidet `open` (siehe `OpenPageMode`); bei `keep`
   * und `adopt` ist der Zustand der Oberfläche die Grundlage (ungesicherte
   * Eingaben gehen nicht verloren).
   */
  function mutate(
    page: string,
    change: (state: StateObject) => boolean,
    open: OpenPageMode = "refuse",
  ): Promise<boolean> {
    return enqueue(page, async () => {
      if (open === "refuse" && live.isLive(page)) return false;
      const text = await io.load(page);
      if (text === null) return false;
      const isOpen = live.isLive(page);
      if (isOpen && open === "refuse") return false;
      const log = committed.get(page);
      const entries = log ? log.entries : logOfState(text);
      const state = parseState(isOpen ? live.build(entries) : text);
      if (!state || !change(state)) return false;
      if (log) state.einfuegungen = entries.length > 0 ? entries : undefined;
      if (!(await io.save(page, JSON.stringify(state)))) return false;
      if (isOpen && open === "adopt") live.reload?.(page);
      return true;
    });
  }

  return {
    commitLog,
    save,
    mutate,
    seed,
    release,
    /** Gesichertes Protokoll der Seite (leer, wenn keines vorliegt). */
    entries: (page: string): PasteLogEntry[] =>
      committed.get(page)?.entries ?? [],
    committed: (page: string): CommittedLog | undefined => committed.get(page),
  };
}

export type PageStateStore = ReturnType<typeof createPageStateStore>;
