import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import {
  commands,
  type LibraryBook,
  type PostProcessProvider,
} from "@/bindings";
import i18n from "@/i18n";
import { anchorAt } from "@/lib/reader/resumePoint";
import { freshBook } from "./importFile";
import { pageText, remapOffset, remapSync } from "./proofread";
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
  /** Läuft auf diesem Rechner (wie `translator::active_model_is_local`). */
  local: boolean;
  label: string;
}

const LOCAL_HOSTS = ["localhost", "127.0.0.1", "[::1]", "::1", "0.0.0.0"];

export function isLocalEndpoint(
  provider: Pick<PostProcessProvider, "id" | "base_url">,
): boolean {
  if (["local", "ollama", "vllm"].includes(provider.id)) return true;
  const url = (provider.base_url ?? "").toLowerCase();
  const rest = url.includes("://") ? url.split("://")[1] : url;
  return LOCAL_HOSTS.includes(rest.split(/[/:]/)[0] ?? "");
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
  auto: boolean;
}

let current: ProofreadProgress | null = null;
let waiting: string[] = [];
let errors: Record<string, string> = {};
let snapshot: ProofreadView = { current, waiting, errors, auto: true };
const listeners = new Set<() => void>();

function publish() {
  snapshot = { current, waiting, errors, auto: proofreadAuto() };
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

/** Seite und Buch auf `newText` setzen; bei einem Fehler zurück auf den alten Stand. */
async function apply(
  book: LibraryBook,
  pageId: string,
  stateJson: string,
  oldText: string,
  newText: string,
  info: NonNullable<NonNullable<LibraryBook["text"]>["proofread"]> | null,
): Promise<void> {
  const state = JSON.parse(stateJson) as Record<string, unknown>;
  // Zwischenzeitlich geändert (Text angehängt, im Reader bearbeitet)?
  const live = await commands.pageStateLoad(pageId);
  if (live.status !== "ok" || pageText(live.data) !== oldText)
    throw new Error(i18n.t("library.proofread.error.changed"));
  const stored = await commands.pageStateSave(
    pageId,
    JSON.stringify({ ...state, text: newText }),
  );
  if (stored.status !== "ok") throw new Error(stored.error);
  try {
    const fresh = (await freshBook(book.id)) ?? book;
    if (!fresh.text || fresh.text.page_id !== pageId)
      throw new Error(i18n.t("library.proofread.error.changed"));
    const saved = await commands.librarySave({
      ...fresh,
      text: { ...fresh.text, proofread: info },
      audio: fresh.audio
        ? {
            ...fresh.audio,
            sync: remapSync(oldText, newText, fresh.audio.sync ?? []),
          }
        : fresh.audio,
    });
    if (saved.status !== "ok") throw new Error(saved.error);
  } catch (e) {
    await commands.pageStateSave(pageId, stateJson);
    throw e;
  }
  remapReaderPositions(pageId, oldText, newText);
}

/** Aufträge laufen nacheinander (der Kern erlaubt nur einen Lauf). */
let chain: Promise<unknown> = Promise.resolve();

/**
 * Buch korrekturlesen. Läuft nach einem anderen Buch weiter. Das Ergebnis
 * steht danach an der Textfassung (`text.proofread`) und im Text der Seite.
 */
export function proofreadBook(bookId: string): Promise<void> {
  ensureListener();
  waiting = [...waiting, bookId];
  errors = Object.fromEntries(
    Object.entries(errors).filter(([id]) => id !== bookId),
  );
  publish();
  const task = chain.then(async () => {
    waiting = waiting.filter((id) => id !== bookId);
    current = { bookId, done: 0, total: 0 };
    publish();
    try {
      const book = await freshBook(bookId);
      if (!book?.text) throw new Error(describe("proofread:no_text"));
      const pageId = book.text.page_id;
      const loaded = await commands.pageStateLoad(pageId);
      if (loaded.status !== "ok") throw new Error(loaded.error);
      const oldText = pageText(loaded.data);
      const result = await commands.libraryProofread(
        bookId,
        oldText,
        !!book.text.proofread,
      );
      if (result.status !== "ok") throw new Error(describe(result.error));
      const r = result.data;
      await apply(book, pageId, loaded.data, oldText, r.text, {
        model: r.model,
        at_ms: r.at_ms,
        changed: r.changed,
        rejected: r.rejected,
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      if (message !== "proofread:cancelled")
        errors = { ...errors, [bookId]: message };
      throw e;
    } finally {
      current = null;
      publish();
    }
  });
  chain = task.catch(() => undefined);
  return task;
}

/** Laufendes Korrekturlesen abbrechen; Fertiges bleibt für den Neustart im Cache. */
export async function cancelProofread(): Promise<void> {
  waiting = [];
  publish();
  await commands.libraryProofreadCancel();
}

/**
 * Original wiederherstellen: Text der Seite, Zeitachse und Stellen zurück,
 * Vermerk an der Textfassung weg, Sicherung gelöscht.
 */
export async function restoreOriginal(bookId: string): Promise<void> {
  const book = await freshBook(bookId);
  if (!book?.text?.proofread) return;
  const pageId = book.text.page_id;
  const original = await commands.libraryProofreadOriginal(bookId);
  if (original.status !== "ok" || original.data === null)
    throw new Error(i18n.t("library.proofread.error.noOriginal"));
  const loaded = await commands.pageStateLoad(pageId);
  if (loaded.status !== "ok") throw new Error(loaded.error);
  await apply(
    book,
    pageId,
    loaded.data,
    pageText(loaded.data),
    original.data,
    null,
  );
  await commands.libraryProofreadDiscard(bookId);
}

/**
 * Nach der Transkription: gleich korrekturlesen, wenn die Einstellung an und
 * ein lokales Modell eingerichtet ist. Ohne Anbieter geschieht nichts; das
 * Buchdetail weist darauf hin.
 */
export async function autoProofread(bookId: string): Promise<void> {
  if (!proofreadAuto()) return;
  // Ein Anbieter außerhalb dieses Rechners bekommt den Text nur nach einem
  // bewussten Klick im Buchdetail, nie automatisch.
  const info = await providerInfo();
  if (!info.ready || !info.local) return;
  const book = await freshBook(bookId);
  if (!book?.text || book.text.origin !== "transcript") return;
  try {
    await proofreadBook(bookId);
  } catch {
    /* Fehler und Abbruch stehen im Buchdetail */
  }
}
