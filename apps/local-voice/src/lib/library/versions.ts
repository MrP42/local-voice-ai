/**
 * Fassungen eines Buchs (lva#236): alles, was die App zu einem Buch hat und
 * einzeln weitergeben kann – Buchtext, Übersetzungen, Kurzbuch und weitere
 * KI-Dokumente, das importierte Hörbuch und vertonte Dateien des Vorlesens.
 *
 * Quelle ist immer die Vorlesen-Seite des Buchs (`state.json`: Text,
 * übersetzte Sprachen, Buchsprache), die KI-Dokumente der Seite, der
 * Übersetzungs-Zwischenspeicher des Kerns und die Seitendateien.
 */
import { commands, type KiDocument, type LibraryBook } from "@/bindings";
import { TTS_TARGET_LANGS } from "@/lib/constants/languages";
import { KURZBUCH_TEMPLATE_ID } from "@/lib/kurzbuch/kurzbuch";
import { getPageStateStore } from "@/lib/reader/pageStore";
import { tracksOf } from "./names";

export type VersionKind =
  "original" | "translation" | "kurzbuch" | "kidoc" | "audiobook" | "narration";

export interface BookVersion {
  /** Eindeutig je Buch: `original`, `translation:English`, `doc:<id>` … */
  key: string;
  kind: VersionKind;
  /** Sprachcode (`de`, `en`), soweit bekannt. */
  lang: string | null;
  /** Übersetzung: Zielsprache, wie sie der Kern erwartet („English“). */
  target?: string;
  /** KI-Dokument (auch Kurzbuch). */
  doc?: KiDocument;
  /** Hörbuch: alle Teile; Vertonung: die eine Datei. */
  paths?: string[];
  /** Vertonung: Dateiname im Seitenordner. */
  file?: string;
}

/** Arbeitsstand einer Vorlesen-Seite, soweit der Export ihn braucht. */
export interface PageState {
  text: string;
  translatedLangs: string[];
  lang: string | null;
}

export async function loadPageState(pageId: string): Promise<PageState> {
  const empty: PageState = { text: "", translatedLangs: [], lang: null };
  const result = await commands.pageStateLoad(pageId);
  if (result.status !== "ok" || !result.data) return empty;
  try {
    const s = JSON.parse(result.data) as Partial<{
      text: string;
      translatedLangs: string[];
      lang: string;
    }>;
    return {
      text: typeof s.text === "string" ? s.text : "",
      translatedLangs: Array.isArray(s.translatedLangs)
        ? s.translatedLangs.filter((x) => typeof x === "string")
        : [],
      lang: typeof s.lang === "string" ? s.lang : null,
    };
  } catch {
    // Sehr alte Seiten speicherten den reinen Text.
    return { ...empty, text: result.data };
  }
}

/** Zielsprache („English“) → Sprachcode („en“). */
export const targetToCode = (target: string): string | null =>
  TTS_TARGET_LANGS.find((l) => l.value === target)?.code ?? null;

/** Sprachcode → Anzeigename („Deutsch“). */
export const langLabel = (code: string | null | undefined): string =>
  (code && TTS_TARGET_LANGS.find((l) => l.code === code)?.label) || "";

/** Sprachcode aus der Buchangabe („Deutsch“, „de“, „German“). */
export function bookLangCode(
  book: LibraryBook,
  page: PageState,
): string | null {
  const raw = (page.lang ?? book.language ?? "").trim();
  if (!raw) return null;
  const lower = raw.toLowerCase();
  const hit = TTS_TARGET_LANGS.find(
    (l) =>
      l.code === lower ||
      l.label.toLowerCase() === lower ||
      l.value.toLowerCase() === lower,
  );
  return hit?.code ?? (lower.length === 2 ? lower : null);
}

const AUDIO_FILE = /\.(mp3|wav|m4a|m4b|ogg|opus|flac)$/i;

/**
 * Alle Fassungen eines Buchs in fester Reihenfolge: Text, Übersetzungen,
 * Kurzbücher, weitere KI-Dokumente, Hörbuch, Vertonungen.
 */
export async function listVersions(book: LibraryBook): Promise<BookVersion[]> {
  const out: BookVersion[] = [];
  const pageId = book.text?.page_id ?? null;
  if (pageId) {
    const [page, docs, files, dir] = await Promise.all([
      loadPageState(pageId),
      commands.bookAiDocuments(pageId),
      commands.pageFiles(pageId),
      commands.pageDir(pageId),
    ]);
    if (page.text.trim())
      out.push({
        key: "original",
        kind: "original",
        lang: bookLangCode(book, page),
      });
    // Übersetzungen: was die Seite kennt, nur wenn der Zwischenspeicher sie hat.
    const translations = await Promise.all(
      page.translatedLangs.map(async (target) => {
        const hit = page.text.trim()
          ? await commands.ttsCachedTranslation(page.text, target)
          : null;
        return hit ? target : null;
      }),
    );
    for (const target of translations) {
      if (!target) continue;
      out.push({
        key: `translation:${target}`,
        kind: "translation",
        lang: targetToCode(target),
        target,
      });
    }
    const list: KiDocument[] =
      docs.status === "ok" && Array.isArray(docs.data) ? docs.data : [];
    const sorted = [...list].sort((a, b) => b.modified_ms - a.modified_ms);
    for (const doc of sorted.filter(
      (d) => d.template_id === KURZBUCH_TEMPLATE_ID,
    ))
      out.push({
        key: `doc:${doc.id}`,
        kind: "kurzbuch",
        lang: doc.language ?? null,
        doc,
      });
    for (const doc of sorted.filter(
      (d) => d.template_id !== KURZBUCH_TEMPLATE_ID,
    ))
      out.push({
        key: `doc:${doc.id}`,
        kind: "kidoc",
        lang: doc.language ?? null,
        doc,
      });
    if (files.status === "ok" && dir.status === "ok") {
      const sep = dir.data.includes("\\") ? "\\" : "/";
      for (const f of files.data.filter((x) => AUDIO_FILE.test(x.name)))
        out.push({
          key: `file:${f.name}`,
          kind: "narration",
          lang: null,
          file: f.name,
          paths: [`${dir.data}${sep}${f.name}`],
        });
    }
  }
  const parts = (
    book.audio_paths?.length
      ? book.audio_paths
      : book.audio_path
        ? [book.audio_path]
        : []
  ).filter((p) => !!p);
  if (parts.length)
    out.push({ key: "audiobook", kind: "audiobook", lang: null, paths: parts });
  return out;
}

/** Text einer Textfassung (Markdown); `null`, wenn er fehlt. */
export async function versionText(
  book: LibraryBook,
  v: BookVersion,
): Promise<string | null> {
  const pageId = book.text?.page_id;
  if (!pageId) return null;
  if (v.kind === "original") return (await loadPageState(pageId)).text || null;
  if (v.kind === "translation" && v.target) {
    const page = await loadPageState(pageId);
    return (await commands.ttsCachedTranslation(page.text, v.target)) ?? null;
  }
  if ((v.kind === "kurzbuch" || v.kind === "kidoc") && v.doc) {
    const r = await commands.bookAiDocumentText(pageId, v.doc.id);
    return r.status === "ok" ? r.data : null;
  }
  return null;
}

export const isTextVersion = (v: BookVersion) =>
  v.kind === "original" ||
  v.kind === "translation" ||
  v.kind === "kurzbuch" ||
  v.kind === "kidoc";

/** Hörbuch-Teile als Kapitelmarken (Titel und Dauer je Teil). */
export const audioChapters = (book: LibraryBook) =>
  tracksOf(book.audio).map((t, i) => ({
    title: t.title || `Teil ${i + 1}`,
    duration_ms: t.duration_ms ?? null,
  }));

/**
 * Nach einer Übersetzung aus der Bibliothek: die Sprache in die Seite
 * eintragen (dann zeigt Vorlesen sie als Sprachchip) und ein offenes
 * Vorlesen benachrichtigen, das den Stand sonst beim nächsten Speichern
 * mit seiner alten Liste überschriebe.
 */
export async function markTranslated(pageId: string, target: string) {
  // Erst speichern, dann melden: ein offenes Vorlesen speichert danach seinen
  // (frischeren) Stand samt neuer Sprache und gewinnt damit das Rennen.
  const notify = () =>
    window.dispatchEvent(
      new CustomEvent("lv-page-translated", {
        detail: { page: pageId, lang: target },
      }),
    );
  // Wie bisher in die Datei, nur über den einen Schreibpfad (Kette, frisch
  // gelesen), damit eine gesicherte Rohfassung der Seite erhalten bleibt.
  // Eine Seite ohne Stand (oder im Altformat) bleibt unberührt: Vorlesen findet
  // die Übersetzung über den Zwischenspeicher.
  await getPageStateStore()
    .mutate(pageId, (state, raw) => {
      if (raw.trim() === "") return false;
      const langs = Array.isArray(state.translatedLangs)
        ? (state.translatedLangs as unknown[]).filter(
            (x): x is string => typeof x === "string",
          )
        : [];
      if (langs.includes(target)) return false;
      state.translatedLangs = [...langs, target];
      return true;
    })
    .catch(() => false);
  notify();
}
