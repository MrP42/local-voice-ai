import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import {
  commands,
  type LibraryBook,
  type PostProcessProvider,
  type SyncPoint,
} from "@/bindings";
import i18n from "@/i18n";
import { anchorAt } from "@/lib/reader/resumePoint";
import { freshBook } from "./importFile";
import { registerSettler, settleBook, withBookLock } from "./bookLock";
import {
  hashText,
  isLocalEndpoint,
  pageText,
  remapOffset,
  remapSync,
  type ParagraphEdit,
  replaceParagraphs,
} from "./proofread";
import {
  getLocal,
  loadProgress,
  saveProgress,
  setLocal,
  textProgressKey,
  type ProgressRecord,
  type Range,
} from "./progress";

/**
 * Korrekturlesen des Transkripts (lva#208 B): Ablauf auf der Oberfläche.
 * Der Kern liest absatzweise Korrektur (Cache, Prüfung, Abbruch); hier wird
 * das Ergebnis in die Seite geschrieben und alles umgerechnet, was an
 * Zeichenstellen hängt: Zeitachse, Lesestelle, Hörfortschritt, Lesezeichen.
 */

const AUTO_KEY = "lva.ui.library.proofreadAuto";

/** Automatisch nach der Transkription starten (Einstellung, Vorgabe: an). */
export function proofreadAuto(): boolean {
  try {
    return window.localStorage.getItem(AUTO_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setProofreadAuto(on: boolean): void {
  try {
    window.localStorage.setItem(AUTO_KEY, on ? "on" : "off");
  } catch {
    /* ohne Speicher gilt die Vorgabe */
  }
  publish();
}

/** Eingerichteter Anbieter für die Nachbearbeitung und wohin der Text geht. */
export interface ProviderInfo {
  ready: boolean;
  /** Läuft auf diesem Rechner (nach der Adresse, siehe {@link isLocalEndpoint}). */
  local: boolean;
  label: string;
}

export async function providerInfo(): Promise<ProviderInfo> {
  const none = { ready: false, local: false, label: "" };
  const r = await commands.getAppSettings();
  if (r.status !== "ok") return none;
  const active = (r.data.post_process_providers ?? []).find(
    (p) => p.id === r.data.post_process_provider_id,
  );
  if (!active || !providerConfigured(r.data)) return none;
  return { ready: true, local: isLocalEndpoint(active), label: active.label };
}

/** Ist ein Anbieter mit Modell für die Nachbearbeitung eingerichtet? */
export async function providerReady(): Promise<boolean> {
  return (await providerInfo()).ready;
}

export function providerConfigured(settings: {
  post_process_provider_id?: string;
  post_process_providers?: PostProcessProvider[];
  post_process_models?: Partial<Record<string, string>>;
}): boolean {
  const id = settings.post_process_provider_id;
  if (!id) return false;
  if (!(settings.post_process_providers ?? []).some((p) => p.id === id))
    return false;
  return !!settings.post_process_models?.[id]?.trim();
}

export interface ProofreadProgress {
  bookId: string;
  done: number;
  total: number;
}

export interface ProofreadView {
  /** Buch, das gerade korrekturgelesen wird (und wie weit). */
  current: ProofreadProgress | null;
  /** Bücher in der Warteschlange (ohne das laufende). */
  waiting: string[];
  /** Letzter Fehler je Buch. */
  errors: Record<string, string>;
  /** Bücher, bei denen der automatische Start unterblieben ist (Anbieter nicht lokal). */
  skipped: Record<string, true>;
  auto: boolean;
}

let current: ProofreadProgress | null = null;
/** Ein Auftrag in der Kette: Abbruch (Zähler) und Verwerfen (Zustand) wirken je Auftrag. */
interface Job {
  bookId: string;
  gen: number;
  dropped: boolean;
}
let generation = 0;
const queue: Job[] = [];
let running: Job | null = null;
let waiting: string[] = [];
const isStale = (job: Job) => job.dropped || job.gen !== generation;
let errors: Record<string, string> = {};
let skipped: Record<string, true> = {};
let snapshot: ProofreadView = {
  current,
  waiting,
  errors,
  skipped,
  auto: true,
};
const listeners = new Set<() => void>();

function publish() {
  waiting = queue.filter((j) => !j.dropped).map((j) => j.bookId);
  snapshot = { current, waiting, errors, skipped, auto: proofreadAuto() };
  listeners.forEach((l) => l());
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

/** „Wird korrekturgelesen · 12 von 80“ für die Liste „In Arbeit“. */
export function proofreadLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  p: ProofreadProgress,
): string {
  return p.total > 0
    ? t("library.proofread.step", { done: p.done, total: p.total })
    : t("library.proofread.stepStart");
}

/** Stand des Korrekturlesens für die Oberfläche. */
export function useProofread(): ProofreadView {
  return useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => snapshot,
  );
}

let listening = false;
function ensureListener() {
  if (listening) return;
  listening = true;
  void listen<{ book_id: string; done: number; total: number }>(
    "library-proofread-progress",
    (e) => {
      if (!current || current.bookId !== e.payload.book_id) return;
      // Abbruch, der vor dem Start des Laufs im Kern einging, ging dort
      // verloren: jetzt, wo der Lauf sicher steht, noch einmal melden.
      if (running && isStale(running)) void commands.libraryProofreadCancel();
      current = {
        bookId: current.bookId,
        done: e.payload.done,
        total: e.payload.total,
      };
      publish();
    },
  ).catch(() => {
    listening = false;
  });
}

/** Meldung zu einem Fehlercode des Kerns. */
function describe(error: string): string {
  const known = [
    "proofread:busy",
    "proofread:no_text",
    "proofread:book_missing",
    "proofread:not_local",
  ];
  return known.includes(error)
    ? i18n.t(`library.proofread.error.${error.slice("proofread:".length)}`)
    : error;
}

export const isCancelled = (error: unknown) =>
  error instanceof Error && error.message === "proofread:cancelled";

/** Eine Stelle (Zeichen) einer Fortschrittsdatei umrechnen. */
function remapRecord(
  rec: ProgressRecord,
  oldText: string,
  newText: string,
): ProgressRecord {
  const map = (n: number) => remapOffset(oldText, newText, n);
  const ranges = (list: Range[]): Range[] =>
    list.map(([a, b]): Range => [map(a), map(b)]).filter(([a, b]) => b > a);
  const offset = map(rec.offset);
  return {
    ...rec,
    offset,
    anchor: rec.anchor === undefined ? undefined : anchorAt(newText, offset),
    length: rec.length > 0 ? newText.length : rec.length,
    heard: ranges(rec.heard),
    read: ranges(rec.read),
  };
}

/**
 * Lese- und Hörstellen einer Seite nach einer Textänderung mit gleicher
 * Absatzzahl umrechnen: Fortschrittsdatei, Weiterlese-Stelle, Lesezeichen.
 */
export function remapReaderPositions(
  pageId: string,
  oldText: string,
  newText: string,
): void {
  const progressKey = textProgressKey(pageId);
  const rec = loadProgress(progressKey);
  if (rec && (rec.unit ?? "chars") === "chars")
    saveProgress(progressKey, remapRecord(rec, oldText, newText));

  const resumeKey = `tts.resume.${pageId}:original`;
  const resume = getLocal(resumeKey);
  if (resume) {
    try {
      const v = JSON.parse(resume) as { offset: number; anchor: string };
      const offset = remapOffset(oldText, newText, v.offset);
      setLocal(
        resumeKey,
        JSON.stringify({ offset, anchor: anchorAt(newText, offset) }),
      );
    } catch {
      /* kaputter Eintrag: der Anker findet die Stelle selbst */
    }
  }

  const marksKey = `tts.bookmarks.${pageId}:original`;
  const marks = getLocal(marksKey);
  if (marks) {
    try {
      const list = (
        JSON.parse(marks) as { offset: number; anchor: string }[]
      ).map((b) => {
        const offset = remapOffset(oldText, newText, b.offset);
        return { ...b, offset, anchor: anchorAt(newText, offset) };
      });
      setLocal(marksKey, JSON.stringify(list));
    } catch {
      /* siehe oben */
    }
  }
}

type ProofreadInfo = NonNullable<NonNullable<LibraryBook["text"]>["proofread"]>;

const changedMessage = () => i18n.t("library.proofread.error.changed");

interface CommitOptions {
  /** Neuer Vermerk an der Textfassung (Vorgabe: alter Vermerk mit neuer Prüfsumme). */
  info?: (text: string, old: ProofreadInfo | null) => ProofreadInfo | null;
  /** Zeitachse zum neuen Text; `null` oder fehlend: Stützpunkte umrechnen. */
  sync?: (text: string) => SyncPoint[] | null;
  /** Auch schreiben, wenn kein Absatz ersetzt wurde (Vermerk setzen). */
  always?: boolean;
}

export interface Committed {
  text: string;
  applied: number[];
  /** Absätze, die inzwischen anders lauten und deshalb stehen blieben. */
  skipped: number[];
}

/**
 * Absätze der Seite ersetzen, unter der Sperre des Buchs: frisch lesen, nur
 * unveränderte Absätze ersetzen, Seite und Buch schreiben. Scheitert das
 * Buch, kommt der Text der Seite zurück (frisch gelesen, nur `text`), und ein
 * gescheiterter Rückweg wird in der Fehlermeldung genannt.
 */
export function commitParagraphs(
  bookId: string,
  edits: ParagraphEdit[],
  options: CommitOptions = {},
): Promise<Committed> {
  return withBookLock(bookId, async () => {
    const book = await freshBook(bookId);
    if (!book?.text) throw new Error(changedMessage());
    const pageId = book.text.page_id;
    const loaded = await commands.pageStateLoad(pageId);
    if (loaded.status !== "ok") throw new Error(loaded.error);
    const oldText = pageText(loaded.data);
    const { text, applied, skipped } = replaceParagraphs(oldText, edits);
    if (applied.length === 0 && !options.always)
      return { text, applied, skipped };
    const stored = await commands.pageStateSave(
      pageId,
      JSON.stringify({
        ...(JSON.parse(loaded.data) as Record<string, unknown>),
        text,
      }),
    );
    if (stored.status !== "ok") throw new Error(stored.error);
    try {
      const fresh = (await freshBook(bookId)) ?? book;
      if (!fresh.text || fresh.text.page_id !== pageId)
        throw new Error(changedMessage());
      const old = fresh.text.proofread ?? null;
      const info = options.info
        ? options.info(text, old)
        : old && { ...old, text_hash: hashText(text) };
      const sync =
        options.sync?.(text) ??
        remapSync(oldText, text, fresh.audio?.sync ?? []);
      const saved = await commands.librarySave({
        ...fresh,
        text: { ...fresh.text, proofread: info },
        audio: fresh.audio ? { ...fresh.audio, sync } : fresh.audio,
      });
      if (saved.status !== "ok") throw new Error(saved.error);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      const back = await rollback(pageId, oldText);
      throw new Error(
        back
          ? message
          : `${message} ${i18n.t("library.proofread.error.rollback")}`,
      );
    }
    remapReaderPositions(pageId, oldText, text);
    return { text, applied, skipped };
  });
}

/** Text der Seite auf den alten Stand setzen; frisch lesen, nur `text` ersetzen. */
async function rollback(pageId: string, oldText: string): Promise<boolean> {
  try {
    const now = await commands.pageStateLoad(pageId);
    if (now.status !== "ok") return false;
    const back = await commands.pageStateSave(
      pageId,
      JSON.stringify({
        ...(JSON.parse(now.data) as Record<string, unknown>),
        text: oldText,
      }),
    );
    return back.status === "ok";
  } catch {
    return false;
  }
}

/** Einzelne Absätze annehmen, ablehnen oder übernehmen (Änderungsansicht). */
export const editParagraphs = (bookId: string, edits: ParagraphEdit[]) =>
  commitParagraphs(bookId, edits);

/** Aufträge laufen nacheinander (der Kern erlaubt nur einen Lauf). */
let chain: Promise<unknown> = Promise.resolve();
const tasks = new Map<string, { job: Job; task: Promise<void> }>();

export interface ProofreadOptions {
  /** Der Kern bricht ab, wenn der Anbieter nicht lokal ist (automatischer Start). */
  requireLocal?: boolean;
}

/**
 * Buch korrekturlesen. Läuft nach einem anderen Buch weiter. Das Ergebnis
 * steht danach an der Textfassung (`text.proofread`) und im Text der Seite.
 * Bei einem erneuten Lauf bleiben von Hand bearbeitete Absätze stehen.
 */
export function proofreadBook(
  bookId: string,
  options: ProofreadOptions = {},
): Promise<void> {
  const existing = tasks.get(bookId);
  if (existing && !existing.job.dropped) return existing.task;
  ensureListener();
  const job: Job = { bookId, gen: generation, dropped: false };
  queue.push(job);
  errors = Object.fromEntries(
    Object.entries(errors).filter(([id]) => id !== bookId),
  );
  skipped = Object.fromEntries(
    Object.entries(skipped).filter(([id]) => id !== bookId),
  );
  publish();
  const task = chain.then(() => runJob(job, !!options.requireLocal));
  tasks.set(bookId, { job, task });
  const done = () => {
    if (tasks.get(bookId)?.task === task) tasks.delete(bookId);
  };
  chain = task.then(done, done);
  return task;
}

async function runJob(job: Job, requireLocal: boolean): Promise<void> {
  const { bookId } = job;
  const at = queue.indexOf(job);
  if (at >= 0) queue.splice(at, 1);
  if (isStale(job)) {
    publish();
    throw new Error("proofread:cancelled");
  }
  running = job;
  current = { bookId, done: 0, total: 0 };
  publish();
  try {
    const book = await freshBook(bookId);
    if (!book?.text) throw new Error(describe("proofread:no_text"));
    const pageId = book.text.page_id;
    const rerun = !!book.text.proofread;
    const loaded = await commands.pageStateLoad(pageId);
    if (loaded.status !== "ok") throw new Error(loaded.error);
    const page = pageText(loaded.data);
    const locked = rerun
      ? await editedParagraphs(book, page)
      : new Set<number>();
    const result = await commands.libraryProofread(
      bookId,
      page,
      rerun,
      requireLocal,
    );
    if (result.status !== "ok") {
      if (result.error === "proofread:not_local") throw new Error(result.error);
      throw new Error(describe(result.error));
    }
    if (isStale(job)) throw new Error("proofread:cancelled");
    const r = result.data;
    if (!rerun) {
      // Zeitachse des Originals neben dem Text sichern; Wiederherstellen
      // schreibt sie genau so zurück.
      await commands.libraryProofreadSaveSync(
        bookId,
        JSON.stringify(book.audio?.sync ?? []),
      );
    }
    const before = page.split("\n\n");
    const after = r.text.split("\n\n");
    if (before.length !== after.length) throw new Error(changedMessage());
    const edits: ParagraphEdit[] = [];
    before.forEach((text, index) => {
      if (!locked.has(index) && after[index] !== text)
        edits.push({ index, expect: text, next: after[index] });
    });
    await commitParagraphs(bookId, edits, {
      always: true,
      info: (text) => ({
        model: r.model,
        at_ms: r.at_ms,
        changed: r.changed,
        rejected: r.rejected,
        unchanged: r.unchanged,
        text_hash: hashText(text),
      }),
    });
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    if (message === "proofread:not_local") {
      skipped = { ...skipped, [bookId]: true };
    } else if (message !== "proofread:cancelled") {
      errors = { ...errors, [bookId]: message };
    }
    throw e;
  } finally {
    running = null;
    current = null;
    publish();
  }
}

/**
 * Absätze, die der Nutzer nach dem Korrekturlesen selbst geändert hat (weder
 * Original noch Korrektur des Modells): ein erneuter Lauf lässt sie stehen.
 */
async function editedParagraphs(
  book: LibraryBook,
  page: string,
): Promise<Set<number>> {
  const material = await commands.libraryProofreadMaterial(book.id);
  if (material.status !== "ok" || material.data.original === null)
    throw new Error(i18n.t("library.proofread.error.noOriginal"));
  const corrected =
    material.data.corrected ??
    (hashText(page) === book.text?.proofread?.text_hash ? page : null);
  if (corrected === null) throw new Error(changedMessage());
  const o = material.data.original.split("\n\n");
  const c = corrected.split("\n\n");
  const p = page.split("\n\n");
  if (o.length !== p.length || c.length !== p.length)
    throw new Error(changedMessage());
  const edited = new Set<number>();
  p.forEach((text, i) => {
    if (text !== o[i] && text !== c[i]) edited.add(i);
  });
  return edited;
}

/**
 * Abbrechen: das laufende Korrekturlesen und alle wartenden Aufträge. Der
 * Zähler macht auch Aufträge ungültig, die gerade erst starten; fertige
 * Blöcke bleiben für den Neustart im Cache.
 */
export async function cancelProofread(): Promise<void> {
  generation++;
  queue.length = 0;
  publish();
  await commands.libraryProofreadCancel();
}

/**
 * Ein Buch aus dem Korrekturlesen nehmen und warten, bis nichts mehr läuft:
 * Voraussetzung für Verwerfen, Wiederherstellen, Anhängen und Ersetzen.
 */
async function settle(bookId: string): Promise<void> {
  const entry = tasks.get(bookId);
  if (!entry) return;
  entry.job.dropped = true;
  const at = queue.indexOf(entry.job);
  if (at >= 0) queue.splice(at, 1);
  tasks.delete(bookId);
  publish();
  if (running === entry.job) {
    await commands.libraryProofreadCancel();
    await entry.task.catch(() => undefined);
  }
}
registerSettler(settle);

export interface Restored {
  /** Absätze, die zurück aufs Original gesetzt wurden. */
  restored: number;
  /** Von Hand bearbeitete Absätze, die stehen blieben. */
  kept: number;
}

/**
 * Original wiederherstellen: nur Absätze, die noch genau die Korrektur des
 * Modells sind, gehen aufs Original zurück. Von Hand bearbeitete Absätze
 * bleiben stehen (und der Vermerk samt Sicherung mit ihnen). Ist danach alles
 * Original, kommen auch die Zeitachse von damals zurück und die Sicherung
 * wird gelöscht.
 */
export async function restoreOriginal(bookId: string): Promise<Restored> {
  await settleBook(bookId);
  const book = await freshBook(bookId);
  if (!book?.text?.proofread) return { restored: 0, kept: 0 };
  const pageId = book.text.page_id;
  const material = await commands.libraryProofreadMaterial(bookId);
  if (material.status !== "ok" || material.data.original === null)
    throw new Error(i18n.t("library.proofread.error.noOriginal"));
  const original = material.data.original;
  const loaded = await commands.pageStateLoad(pageId);
  if (loaded.status !== "ok") throw new Error(loaded.error);
  const page = pageText(loaded.data);
  const corrected =
    material.data.corrected ??
    (hashText(page) === book.text.proofread.text_hash ? page : null);
  if (corrected === null) throw new Error(changedMessage());
  const o = original.split("\n\n");
  const c = corrected.split("\n\n");
  const p = page.split("\n\n");
  if (o.length !== p.length || c.length !== p.length)
    throw new Error(changedMessage());
  const edits: ParagraphEdit[] = [];
  p.forEach((text, index) => {
    if (c[index] !== o[index] && text === c[index])
      edits.push({ index, expect: text, next: o[index] });
  });
  const originalSync = parseSync(material.data.original_sync);
  const done = await commitParagraphs(bookId, edits, {
    always: true,
    info: (text, old) =>
      text === original || !old ? null : { ...old, text_hash: hashText(text) },
    sync: (text) => (text === original ? originalSync : null),
  });
  const kept = done.text.split("\n\n").filter((t, i) => t !== o[i]).length;
  if (done.text === original)
    await withBookLock(bookId, async () => {
      await commands.libraryProofreadDiscard(bookId);
    });
  return { restored: done.applied.length, kept };
}

function parseSync(raw: string | null): SyncPoint[] | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as SyncPoint[];
    return Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

/**
 * Nach der Transkription: gleich korrekturlesen, wenn die Einstellung an und
 * das Modell auf diesem Rechner eingerichtet ist. Der Anbieter wird bei JEDEM
 * Start frisch geprüft, und der Kern prüft ihn beim Lauf noch einmal
 * (`requireLocal`): Das Einverständnis zum lokalen Start gilt nie für einen
 * Anbieter im Netz.
 */
export async function autoProofread(bookId: string): Promise<void> {
  if (!proofreadAuto()) return;
  const info = await providerInfo();
  if (!info.ready) return;
  if (!info.local) {
    skipped = { ...skipped, [bookId]: true };
    publish();
    return;
  }
  const book = await freshBook(bookId);
  if (!book?.text || book.text.origin !== "transcript") return;
  try {
    await proofreadBook(bookId, { requireLocal: true });
  } catch {
    /* Fehler und Abbruch stehen im Buchdetail */
  }
}
