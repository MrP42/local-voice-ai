/**
 * KI-Aufträge aus der Bibliothek (lva#236): Übersetzung, Kurzbuch und weitere
 * KI-Dokumente starten, ohne ins Vorlesen zu wechseln.
 *
 * Die Arbeit macht der Kern (`tts_translate`, `book_ai_summarize`); hier
 * steht nur die Warteschlange mit Fortschritt, damit Karten, Buchseite und
 * „In Arbeit“ denselben Stand zeigen – auch nach einem Seitenwechsel, denn
 * der Speicher lebt auf Modulebene, nicht in einer Komponente. Der Kern kann
 * je Art nur einen Lauf zugleich; deshalb zwei Warteschlangen (Übersetzung,
 * KI-Dokument), die nebeneinander laufen.
 */
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { commands, type KiDocument } from "@/bindings";
import { markTranslated } from "./versions";

export type BookJobKind = "translation" | "kurzbuch" | "kidoc";

export interface BookJob {
  id: string;
  bookId: string;
  pageId: string;
  kind: BookJobKind;
  /** Übersetzung: Zielsprache des Kerns („English“). */
  target?: string;
  /** KI-Dokument: Vorlage; Sprache des Ergebnisses ("" = wie das Buch). */
  templateId?: string;
  templateTitle?: string;
  lang?: string;
  status: "queued" | "running";
  done: number;
  total: number;
}

export interface BookJobResult {
  job: BookJob;
  ok: boolean;
  error?: string;
  doc?: KiDocument;
}

type Listener = () => void;

let jobs: BookJob[] = [];
const listeners = new Set<Listener>();
const finishers = new Set<(r: BookJobResult) => void>();
let counter = 0;
/** Abgebrochene Aufträge, die gerade auf einen fremden Lauf warten. */
const cancelled = new Set<string>();
/** Nur während des eigenen Aufrufs zählen Übersetzungsschritte (Vorlesen meldet dieselben Events). */
let ownTranslation: string | null = null;
let ownKi: string | null = null;
const BUSY_RETRY_MS = 4000;

/** Der Kern kann je Art nur einen Lauf zugleich – läuft gerade einer aus dem Vorlesen? */
const isBusy = (error: string) =>
  error === "book_ai:busy" ||
  error.startsWith("Es läuft bereits eine Übersetzung");

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Aufruf wiederholen, solange ein fremder Lauf den Kern belegt; abbrechbar. */
async function whenFree<T>(
  job: BookJob,
  call: () => Promise<
    { status: "ok"; data: T } | { status: "error"; error: string }
  >,
): Promise<{ status: "ok"; data: T } | { status: "error"; error: string }> {
  for (;;) {
    const r = await call();
    if (r.status === "ok" || !isBusy(r.error)) return r;
    await sleep(BUSY_RETRY_MS);
    if (cancelled.has(job.id)) return { status: "error", error: "cancelled" };
  }
}

const emit = () => {
  jobs = [...jobs];
  listeners.forEach((l) => l());
};

const update = (id: string, patch: Partial<BookJob>) => {
  jobs = jobs.map((j) => (j.id === id ? { ...j, ...patch } : j));
  listeners.forEach((l) => l());
};

const lane = (kind: BookJobKind) => (kind === "translation" ? "t" : "k");

let eventsBound = false;
function bindEvents() {
  if (eventsBound) return;
  eventsBound = true;
  void listen<{ page_id: string; kind: string; done: number; total: number }>(
    "book-ai-progress",
    (e) => {
      if (e.payload.kind !== "summary") return;
      const run = jobs.find(
        (j) => j.id === ownKi && j.pageId === e.payload.page_id,
      );
      if (run) update(run.id, { done: e.payload.done, total: e.payload.total });
    },
  );
  void listen<{ index: number; total: number }>("tts-translate-chunk", (e) => {
    const run = jobs.find((j) => j.id === ownTranslation);
    if (run)
      update(run.id, {
        done: Math.min(run.done + 1, e.payload.total),
        total: e.payload.total,
      });
  });
}

/** Nächsten wartenden Auftrag einer Spur starten. */
function pump(kind: BookJobKind) {
  const l = lane(kind);
  if (jobs.some((j) => j.status === "running" && lane(j.kind) === l)) return;
  const next = jobs.find((j) => j.status === "queued" && lane(j.kind) === l);
  if (!next) return;
  update(next.id, { status: "running" });
  void run(next).then((result) => {
    jobs = jobs.filter((j) => j.id !== next.id);
    emit();
    finishers.forEach((f) => f(result));
    pump(kind);
  });
}

async function run(job: BookJob): Promise<BookJobResult> {
  try {
    if (job.kind === "translation") {
      const state = await commands.pageStateLoad(job.pageId);
      const text = state.status === "ok" ? pageText(state.data) : "";
      if (!text.trim())
        return { job, ok: false, error: "book_ai:page_without_text" };
      const r = await whenFree(job, async () => {
        ownTranslation = job.id;
        try {
          return await commands.ttsTranslate(text, job.target!);
        } finally {
          ownTranslation = null;
        }
      });
      if (r.status === "error") return { job, ok: false, error: r.error };
      await markTranslated(job.pageId, job.target!);
      return { job, ok: true };
    }
    const r = await whenFree(job, async () => {
      ownKi = job.id;
      try {
        return await commands.bookAiSummarize(
          job.pageId,
          { kind: "book" },
          "Buch",
          job.templateId!,
          job.lang || null,
        );
      } finally {
        ownKi = null;
      }
    });
    if (r.status === "error") return { job, ok: false, error: r.error };
    window.dispatchEvent(
      new CustomEvent("lv-kidocs-changed", { detail: { page: job.pageId } }),
    );
    return { job, ok: true, doc: r.data };
  } catch (e) {
    return { job, ok: false, error: String(e) };
  } finally {
    cancelled.delete(job.id);
  }
}

function pageText(raw: string): string {
  if (!raw) return "";
  try {
    const s = JSON.parse(raw) as { text?: unknown };
    return typeof s.text === "string" ? s.text : "";
  } catch {
    return raw;
  }
}

/** Auftrag einreihen. Ein gleicher Auftrag (Buch, Art, Sprache, Vorlage) läuft nur einmal. */
export function startBookJob(
  input: Omit<BookJob, "id" | "status" | "done" | "total">,
): { started: boolean; queued: boolean } {
  bindEvents();
  const same = jobs.find(
    (j) =>
      j.pageId === input.pageId &&
      j.kind === input.kind &&
      (j.target ?? "") === (input.target ?? "") &&
      (j.templateId ?? "") === (input.templateId ?? "") &&
      (j.lang ?? "") === (input.lang ?? ""),
  );
  if (same) return { started: false, queued: same.status === "queued" };
  const l = lane(input.kind);
  const busy = jobs.some((j) => lane(j.kind) === l);
  jobs = [
    ...jobs,
    { ...input, id: `job${++counter}`, status: "queued", done: 0, total: 0 },
  ];
  emit();
  pump(input.kind);
  return { started: true, queued: busy };
}

/** Laufenden Auftrag abbrechen (endet nach dem aktuellen Schritt) oder wartenden streichen. */
export function cancelBookJob(id: string) {
  const job = jobs.find((j) => j.id === id);
  if (!job) return;
  if (job.status === "queued") {
    jobs = jobs.filter((j) => j.id !== id);
    emit();
    return;
  }
  cancelled.add(id);
  // Wartet der Auftrag noch auf einen Lauf aus dem Vorlesen, reicht die Marke;
  // sonst würde der fremde Lauf mit abgebrochen.
  if (job.kind === "translation") {
    if (ownTranslation === id) void commands.ttsTransformCancel("translation");
  } else if (ownKi === id) void commands.bookAiCancel();
}

export function onBookJobFinished(fn: (r: BookJobResult) => void): () => void {
  finishers.add(fn);
  return () => finishers.delete(fn);
}

const subscribe = (l: Listener) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const snapshot = () => jobs;

export function useBookJobs(): BookJob[] {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export const jobsOfBook = (all: BookJob[], bookId: string) =>
  all.filter((j) => j.bookId === bookId);

/** Anteil 0–1, solange der Kern noch keine Schritte gemeldet hat 0. */
export const jobFraction = (j: BookJob) => (j.total > 0 ? j.done / j.total : 0);

/** Nur für Tests: Speicher leeren. */
export function resetBookJobs() {
  jobs = [];
  emit();
}
