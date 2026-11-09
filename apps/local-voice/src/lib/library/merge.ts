import { commands, type LibraryBook } from "@/bindings";
import type { SplitGroup } from "./audioGroups";
import { assetUrl, pendingJobs, totalDuration, tracksOf } from "./books";
import {
  flushProgress,
  loadProgress,
  recordPosition,
  removeProgress,
  textProgressKey,
} from "./progress";
import { primaryAudioKey } from "./editions";
import { mergedPosition, type MergeEntry } from "./position";
import { probeDuration } from "./probe";
import { findWork, ImportError } from "./importFile";
import { analyzeName } from "./audioGroups";
import { cleanTitle, guessFromFileName } from "./names.ts";
import { joinTranscripts, type TranscriptPiece } from "./transcript";
import i18n from "@/i18n";

/**
 * Zusammenführen (lva#204): einzeln importierte Teile eines Hörbuchs werden
 * ein Buch mit Teileliste. Das Ziel ist ein vorhandenes Werk mit passendem
 * Titel und Autor, sonst das erste Buch der Folge.
 */

export interface MergePlan {
  /** Buch, das alle Teile bekommt. */
  target: LibraryBook;
  /** Das Ziel ist ein schon vorhandenes Werk (nicht Teil der Gruppe). */
  existing: boolean;
  /** Ein vorhandenes Hörbuch des Ziels wird dabei ersetzt. */
  replacesAudio: boolean;
  /** Quellbücher in Hörreihenfolge; sie verschwinden. */
  sources: LibraryBook[];
  /** Titel des Hörbuchs nach dem Zusammenführen. */
  title: string;
  /**
   * Was mit den Transkripten der Teile geschieht: `join` = zu einem Text
   * verbinden, `drop` = fallen weg (das Ziel hat einen Buchtext), `none` =
   * keine vorhanden.
   */
  transcripts: "join" | "drop" | "none";
}

const isTranscript = (b: LibraryBook) => b.text?.origin === "transcript";
const isDocument = (b: LibraryBook) => !!b.text && !isTranscript(b);

/** Teile in Hörreihenfolge, deren Audio im Ergebnis bleibt. */
const audioMembers = (plan: Pick<MergePlan, "target" | "sources" | "replacesAudio">) =>
  [...(!plan.replacesAudio && plan.target.audio ? [plan.target] : []), ...plan.sources];

function transcriptMode(
  plan: Pick<MergePlan, "target" | "sources" | "replacesAudio">,
): MergePlan["transcripts"] {
  const members = audioMembers(plan);
  if (!members.some(isTranscript)) return "none";
  return isDocument(plan.target) ? "drop" : "join";
}

/**
 * Werden Transkripte verbunden, darf keiner der Teile noch transkribieren:
 * sein Text käme sonst später ans Ende statt an seine Stelle.
 */
function assertIdle(books: LibraryBook[]) {
  if (!books.some(isTranscript)) return;
  const busy = books.find((b) => pendingJobs(b).length > 0);
  if (busy)
    throw new ImportError(
      i18n.t("library.merge.busy", { title: busy.title }),
    );
}

export async function planMerge(group: SplitGroup): Promise<MergePlan> {
  assertIdle(group.books);
  const ids = new Set(group.books.map((b) => b.id));
  const work = await findWork(group.title, group.authors);
  const existing = work && !ids.has(work.id) ? work : null;
  if (existing) {
    const base = {
      target: existing,
      existing: true,
      replacesAudio: !!existing.audio,
      sources: group.books,
      title: existing.title,
    };
    return { ...base, transcripts: transcriptMode(base) };
  }
  const [first, ...rest] = group.books;
  const base = {
    target: first,
    existing: false,
    replacesAudio: false,
    sources: rest,
    title: group.title,
  };
  return { ...base, transcripts: transcriptMode(base) };
}

const listeningOrder = (books: LibraryBook[]) =>
  books
    .map((book) => ({ book, a: analyzeName(book.title) }))
    .sort(
      (x, y) =>
        x.a.disc - y.a.disc ||
        x.a.track - y.a.track ||
        x.book.title.localeCompare(y.book.title, "de", { numeric: true }),
    )
    .map((x) => x.book);

/**
 * Von Hand gewählte Einträge zusammenführen (Mehrfachauswahl): Ziel ist das
 * Buch mit Buchtext, sonst das erste in Hörreihenfolge. Alle anderen brauchen ein Hörbuch; sie werden
 * dessen Teile. Wirft {@link ImportError} mit einer verständlichen Meldung.
 */
export function planManual(books: LibraryBook[]): {
  group: SplitGroup;
  plan: MergePlan;
} {
  if (books.length < 2) throw new ImportError(i18n.t("library.merge.needTwo"));
  const documents = books.filter(isDocument);
  if (documents.length > 1)
    throw new ImportError(
      i18n.t("library.merge.twoTexts", {
        a: documents[0].title,
        b: documents[1].title,
      }),
    );
  assertIdle(books);
  const ordered = listeningOrder(books);
  const doc = documents[0] ?? null;
  // Ohne Buchtext ist der erste Teil das Ziel (Hörreihenfolge = Zeitachse);
  // Reihenfolge lässt sich in der Rückfrage noch ändern.
  const target = doc ?? ordered.find((b) => b.audio) ?? ordered[0];
  const silent = books.find((b) => b !== target && !b.audio);
  if (silent)
    throw new ImportError(
      i18n.t("library.merge.noAudio", { title: silent.title }),
    );
  const audio = ordered.filter((b) => b.audio);
  const stem = analyzeName(target.title).stem || target.title;
  // „Autor - Titel“ wie beim Import trennen; der Titel trägt keinen Autor.
  const title = doc
    ? doc.title
    : cleanTitle(guessFromFileName(`${stem}.mp3`).title || stem);
  const group: SplitGroup = {
    key: `manual:${books.map((b) => b.id).join(",")}`,
    title,
    authors: target.authors.length
      ? target.authors
      : (books.find((b) => b.authors.length)?.authors ?? []),
    books: doc ? audio.filter((b) => b !== doc) : audio,
  };
  const base = doc
    ? {
        target: doc,
        existing: true,
        // Hat das Buch schon ein Hörbuch, kommen die anderen als Teile dazu.
        replacesAudio: false,
        sources: group.books,
        title,
      }
    : {
        target,
        existing: false,
        replacesAudio: false,
        sources: audio.filter((b) => b !== target),
        title,
      };
  // Ohne Buchtext beginnt das Hörbuch mit dem Ziel: es steht vorn.
  if (!doc) group.books = [target, ...base.sources];
  return { group, plan: { ...base, transcripts: transcriptMode(base) } };
}

/** Teile in neuer Reihenfolge (Rückfrage: Hoch/Runter). */
export function reorder(
  group: SplitGroup,
  plan: MergePlan,
  from: number,
  to: number,
): { group: SplitGroup; plan: MergePlan } {
  const books = [...group.books];
  if (to < 0 || to >= books.length) return { group, plan };
  const [moved] = books.splice(from, 1);
  books.splice(to, 0, moved);
  if (plan.existing)
    return { group: { ...group, books }, plan: { ...plan, sources: books } };
  const [target, ...sources] = books;
  const next = { ...plan, target, sources };
  return {
    group: { ...group, books },
    plan: { ...next, transcripts: transcriptMode(next) },
  };
}

interface TextLoad {
  state: Record<string, unknown>;
  text: string;
}

async function loadText(pageId: string): Promise<TextLoad | null> {
  const r = await commands.pageStateLoad(pageId);
  if (r.status !== "ok") return null;
  try {
    const state = JSON.parse(r.data) as Record<string, unknown>;
    return { state, text: typeof state.text === "string" ? state.text : "" };
  } catch {
    return null;
  }
}

/**
 * Gemeinsamer Text der Transkripte in Hörreihenfolge. Teile ohne Transkript
 * fehlen im Text, ihre Dauer zählt trotzdem mit.
 */
async function joinedText(
  plan: MergePlan,
  durations: Map<string, (number | null)[]>,
): Promise<{ text: string; sync: { chars: number; ms: number }[] } | null> {
  if (plan.transcripts !== "join") return null;
  const pieces: TranscriptPiece[] = [];
  let offset = 0;
  for (const [i, book] of audioMembers(plan).entries()) {
    if (isTranscript(book)) {
      const loaded = await loadText(book.text!.page_id);
      if (loaded)
        pieces.push({
          heading: i18n.t("library.merge.partHeading", { n: i + 1 }),
          text: loaded.text,
          sync: book.audio?.sync ?? [],
          offsetMs: offset,
        });
    }
    const own = durations.get(book.id) ?? [];
    const known = own.reduce<number>((a, d) => a + (d ?? 0), 0);
    // Unbekannte Dauer: wenigstens bis zum letzten Stützpunkt des Teils.
    const lastSync = Math.max(0, ...(book.audio?.sync ?? []).map((p) => p.ms));
    offset += own.every((d) => d && d > 0) ? known : Math.max(known, lastSync);
  }
  if (!pieces.length) return null;
  const author = plan.target.authors.length
    ? `\n\nvon ${plan.target.authors.join(", ")}`
    : "";
  const joined = joinTranscripts(`# ${plan.title}${author}\n\n`, pieces);
  return joined.sync.length ? joined : null;
}

/** Teile (Dateien) des Ziels, die ein Zusammenführen ersetzt oder behält. */
export const partCount = (book: LibraryBook) => tracksOf(book.audio).length;

/** Dauern je Teil: gespeichert, sonst aus der Datei ermittelt (`null` = unbekannt). */
async function durationsOf(book: LibraryBook): Promise<(number | null)[]> {
  const tracks = tracksOf(book.audio);
  const paths = book.audio_paths ?? [];
  const out: (number | null)[] = [];
  for (let i = 0; i < tracks.length; i++) {
    const known = tracks[i].duration_ms;
    if (known != null && known > 0) {
      out.push(known);
      continue;
    }
    const url = assetUrl(paths[i]);
    out.push(url ? await probeDuration(url) : null);
  }
  return out;
}

export async function mergeGroup(
  group: SplitGroup,
  plan: MergePlan,
): Promise<LibraryBook> {
  const { target, sources } = plan;
  // Alles, was nach einem Erfolg aufzuräumen ist, vorher festhalten.
  const oldJobs = plan.replacesAudio ? pendingJobs(target) : [];
  const oldText =
    plan.replacesAudio && target.text?.origin === "transcript"
      ? target.text
      : null;
  const entries: MergeEntry[] = [];
  const durations = new Map<string, (number | null)[]>();
  for (const book of audioMembers(plan)) {
    const own = await durationsOf(book);
    durations.set(book.id, own);
    entries.push({
      durations: own,
      progress: loadProgress(primaryAudioKey(book)),
    });
  }
  const position = mergedPosition(entries);
  // Texte der Teile vorher lesen: danach sind die Quellbücher fort.
  const joined = await joinedText(plan, durations);
  const sourcePages = sources
    .filter(isTranscript)
    .map((b) => b.text!.page_id);

  if (!plan.existing) {
    // Das erste Buch bekommt den Stamm-Titel (ohne Disc-/Track-Angaben).
    const retitled = await commands.librarySave({
      ...target,
      title: plan.title,
      authors: target.authors.length ? target.authors : group.authors,
    });
    if (retitled.status !== "ok") throw new ImportError(retitled.error);
  }
  // Ersetzen entscheidet der Kern atomar: Dateien und buch.json zuerst,
  // Aufräumen hier erst nach Erfolg.
  const merged = await commands.libraryMergeAudio(
    target.id,
    sources.map((b) => b.id),
    plan.replacesAudio,
  );
  if (merged.status !== "ok") throw new ImportError(merged.error);
  let result = merged.data;
  if (joined) result = await storeJoined(result, joined, plan);
  // Die Transkripte der Quellen stecken jetzt im gemeinsamen Text (oder
  // fallen neben einem Buchtext weg).
  for (const pageId of sourcePages) {
    if (pageId === result.text?.page_id) continue;
    removeProgress(textProgressKey(pageId));
    await commands.pagesDelete(pageId);
  }
  for (const id of oldJobs) await commands.meetingsDelete(id);
  if (oldText) {
    removeProgress(textProgressKey(oldText.page_id));
    await commands.pagesDelete(oldText.page_id);
  }
  if (plan.replacesAudio) removeProgress(primaryAudioKey(target));
  for (const source of sources) removeProgress(primaryAudioKey(source));
  if (position) {
    const total = durationsSum(result);
    recordPosition(primaryAudioKey(target), {
      to: position.ms,
      length: total ?? 0,
      unit: "ms",
    });
  }
  await flushProgress();
  return result;
}

/**
 * Gemeinsamen Text ablegen: in die Seite des Ziels, wenn sie ein Transkript
 * ist (Lesezeichen und Stelle bleiben), sonst in eine neue Seite.
 */
async function storeJoined(
  book: LibraryBook,
  joined: { text: string; sync: { chars: number; ms: number }[] },
  plan: MergePlan,
): Promise<LibraryBook> {
  let pageId = book.text?.origin === "transcript" ? book.text.page_id : null;
  let state: Record<string, unknown> = {
    summary: "",
    sourceUrl: "",
    tab: "original",
  };
  if (pageId) {
    const loaded = await loadText(pageId);
    if (loaded) state = loaded.state;
  } else {
    const page = await commands.pagesCreate(plan.title);
    if (page.status !== "ok") throw new ImportError(page.error);
    pageId = page.data.id;
  }
  const stored = await commands.pageStateSave(
    pageId,
    JSON.stringify({ ...state, text: joined.text }),
  );
  if (stored.status !== "ok") throw new ImportError(stored.error);
  const saved = await commands.librarySave({
    ...book,
    text: {
      page_id: pageId,
      origin: "transcript",
      source_name: book.text?.source_name ?? null,
    },
    audio: book.audio ? { ...book.audio, sync: joined.sync } : book.audio,
  });
  if (saved.status !== "ok") throw new ImportError(saved.error);
  return saved.data;
}

const durationsSum = (book: LibraryBook): number | null =>
  totalDuration(book.audio);
