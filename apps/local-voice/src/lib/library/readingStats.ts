/**
 * Lesestatistik (lva#354): Vorlagen der Kategorien und reine Auswertung der
 * Zeilen aus `reading_log_rows`. Ohne Tauri, damit Tests sie direkt prüfen.
 */
import type { LibraryBook, ReadingRow } from "@/bindings";

export type Format = "lesen" | "hoeren" | "vorlesen";
export const FORMATS: Format[] = ["lesen", "hoeren", "vorlesen"];

/** Normseite (30 Zeilen × 60 Anschläge), wenn das Buch keine Seitenzahl hat. */
export const NORM_PAGE_CHARS = 1800;

export interface Category {
  key: string;
  /** Übersetzungsschlüssel `readingStats.cat.<key>`; `de` als Rückfall. */
  de: string;
  group: "sachbuch" | "belletristik";
}

/** 19 Vorlagen (Spec lva#354). Eigene Kategorien heißen `eigen:<name>`. */
export const CATEGORIES: Category[] = [
  {
    key: "persoenlichkeit",
    de: "Persönlichkeitsentwicklung",
    group: "sachbuch",
  },
  { key: "kommunikation", de: "Kommunikation & Rhetorik", group: "sachbuch" },
  { key: "fuehrung", de: "Führung & Management", group: "sachbuch" },
  { key: "business", de: "Business & Unternehmertum", group: "sachbuch" },
  { key: "finanzen", de: "Finanzen & Wirtschaft", group: "sachbuch" },
  { key: "psychologie", de: "Psychologie", group: "sachbuch" },
  { key: "gesundheit", de: "Gesundheit & Sport", group: "sachbuch" },
  { key: "wissenschaft", de: "Wissenschaft & Natur", group: "sachbuch" },
  { key: "technik", de: "Technik, IT & KI", group: "sachbuch" },
  {
    key: "fachbuch",
    de: "Fachbuch (Beruf, Recht, Verwaltung)",
    group: "sachbuch",
  },
  { key: "geschichte", de: "Geschichte & Politik", group: "sachbuch" },
  { key: "gesellschaft", de: "Gesellschaft & Kultur", group: "sachbuch" },
  { key: "philosophie", de: "Philosophie & Spiritualität", group: "sachbuch" },
  { key: "ratgeber", de: "Ratgeber & Alltag", group: "sachbuch" },
  { key: "biografie", de: "Biografie & Erinnerungen", group: "sachbuch" },
  { key: "roman", de: "Roman & Erzählung", group: "belletristik" },
  { key: "spannung", de: "Krimi & Thriller", group: "belletristik" },
  { key: "fantastik", de: "Fantasy & Science-Fiction", group: "belletristik" },
  { key: "kinder", de: "Kinder- & Jugendbuch", group: "belletristik" },
];

const BY_KEY = new Map(CATEGORIES.map((c) => [c.key, c]));

/** Anzeigename einer Kategorie; `t` übersetzt Vorlagen, eigene bleiben wörtlich. */
export function categoryLabel(
  key: string | null | undefined,
  t?: (k: string, o?: { defaultValue: string }) => string,
): string | null {
  if (!key) return null;
  if (key.startsWith("eigen:")) return key.slice(6);
  const c = BY_KEY.get(key);
  if (!c) return key;
  return t ? t(`readingStats.cat.${c.key}`, { defaultValue: c.de }) : c.de;
}

export const isTemplateCategory = (key: string) => BY_KEY.has(key);

// ---------------------------------------------------------------------------
// Datum
// ---------------------------------------------------------------------------

const pad = (n: number) => String(n).padStart(2, "0");

/** Ortsdatum `JJJJ-MM-TT`. */
export function localDay(d: Date = new Date()): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function parseDay(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(day: string, n: number): string {
  const d = parseDay(day);
  d.setDate(d.getDate() + n);
  return localDay(d);
}

/** Wochentag-Bit (Montag = 0 … Sonntag = 6). */
export const weekdayBit = (day: string) => (parseDay(day).getDay() + 6) % 7;

export type PeriodKind = "week" | "month" | "year";

export interface Period {
  kind: PeriodKind;
  /** Erster Tag (einschließlich). */
  from: string;
  /** Letzter Tag (einschließlich). */
  to: string;
}

/** Zeitraum, der `day` enthält; `shift` blättert um ganze Zeiträume. */
export function periodOf(kind: PeriodKind, day: string, shift = 0): Period {
  const d = parseDay(day);
  if (kind === "week") {
    const monday = addDays(day, -weekdayBit(day) + shift * 7);
    return { kind, from: monday, to: addDays(monday, 6) };
  }
  if (kind === "month") {
    const first = new Date(d.getFullYear(), d.getMonth() + shift, 1);
    const last = new Date(first.getFullYear(), first.getMonth() + 1, 0);
    return { kind, from: localDay(first), to: localDay(last) };
  }
  const y = d.getFullYear() + shift;
  return { kind, from: `${y}-01-01`, to: `${y}-12-31` };
}

/** Alle Tage des Zeitraums. */
export function daysOf(p: Period): string[] {
  const out: string[] = [];
  for (let d = p.from; d <= p.to; d = addDays(d, 1)) out.push(d);
  return out;
}

/** Vorheriger gleich langer Zeitraum (zum Vergleich). */
export const previousPeriod = (p: Period) => periodOf(p.kind, p.from, -1);

// ---------------------------------------------------------------------------
// Seiten
// ---------------------------------------------------------------------------

export interface BookLengths {
  /** Zeichen des Buchtexts (aus dem Fortschritt), 0 = unbekannt. */
  textChars: number;
  /** Dauer des Hörbuchs in ms, 0 = unbekannt. */
  audioMs: number;
}

/**
 * Seiten einer Zeile. Text: Buchseiten, wenn Seitenzahl und Textlänge bekannt,
 * sonst Normseiten. Hörbuch: nur mit Seitenzahl und Dauer, sonst 0 (nur Zeit,
 * Codex 06.11.). `estimated` meldet Normseiten.
 */
export function pagesOf(
  row: Pick<ReadingRow, "chars" | "audio_ms" | "format">,
  pages: number | null | undefined,
  len: BookLengths,
): { pages: number; estimated: boolean } {
  if (row.format === "hoeren") {
    if (pages && len.audioMs > 0 && row.audio_ms > 0)
      return { pages: (row.audio_ms / len.audioMs) * pages, estimated: false };
    return { pages: 0, estimated: false };
  }
  if (row.chars <= 0) return { pages: 0, estimated: false };
  if (pages && len.textChars > 0)
    return { pages: (row.chars / len.textChars) * pages, estimated: false };
  return { pages: row.chars / NORM_PAGE_CHARS, estimated: true };
}

// ---------------------------------------------------------------------------
// Auswertung
// ---------------------------------------------------------------------------

export interface Share {
  key: string;
  sec: number;
}

export interface BookShare {
  bookId: string;
  sec: number;
  pages: number;
  formats: Format[];
  langs: string[];
  lastDay: string;
  days: number;
}

export interface PeriodSummary {
  sec: number;
  pages: number;
  pagesEstimated: boolean;
  books: BookShare[];
  /** Minuten je Tag und Format. */
  perDay: Map<string, Record<Format, number>>;
  byCategory: Share[];
  byTopic: Share[];
  byLang: Share[];
  byFormat: Record<Format, number>;
  /** Anteil Belletristik an der eingeordneten Zeit (0..1), null ohne Einordnung. */
  fictionShare: number | null;
}

const emptyFormats = (): Record<Format, number> => ({
  lesen: 0,
  hoeren: 0,
  vorlesen: 0,
});

const sorted = (m: Map<string, number>): Share[] =>
  [...m.entries()]
    .map(([key, sec]) => ({ key, sec }))
    .sort((a, b) => b.sec - a.sec || a.key.localeCompare(b.key));

/** Zeilen eines Zeitraums auswerten. `lengths` liefert Textlänge/Dauer je Buch. */
export function summarize(
  rows: ReadingRow[],
  books: LibraryBook[],
  period: Pick<Period, "from" | "to">,
  lengths: (book: LibraryBook) => BookLengths,
): PeriodSummary {
  const byId = new Map(books.map((b) => [b.id, b]));
  const lenCache = new Map<string, BookLengths>();
  const perDay = new Map<string, Record<Format, number>>();
  const cat = new Map<string, number>();
  const topic = new Map<string, number>();
  const lang = new Map<string, number>();
  const per = new Map<string, BookShare & { dayset: Set<string> }>();
  const byFormat = emptyFormats();
  let sec = 0;
  let pages = 0;
  let pagesEstimated = false;
  let fiction = 0;
  let classified = 0;
  for (const r of rows) {
    if (r.day < period.from || r.day > period.to) continue;
    const f = r.format as Format;
    if (!FORMATS.includes(f)) continue;
    sec += r.sec;
    byFormat[f] += r.sec;
    const day = perDay.get(r.day) ?? emptyFormats();
    day[f] += r.sec / 60;
    perDay.set(r.day, day);
    lang.set(r.lang, (lang.get(r.lang) ?? 0) + r.sec);
    const book = byId.get(r.book_id);
    let p = 0;
    if (book) {
      let len = lenCache.get(book.id);
      if (!len) {
        len = lengths(book);
        lenCache.set(book.id, len);
      }
      const got = pagesOf(r, book.pages, len);
      p = got.pages;
      pagesEstimated ||= got.estimated && p > 0;
      if (book.category) {
        cat.set(book.category, (cat.get(book.category) ?? 0) + r.sec);
        classified += r.sec;
        if (BY_KEY.get(book.category)?.group === "belletristik")
          fiction += r.sec;
      }
      for (const t of book.topics ?? [])
        topic.set(t, (topic.get(t) ?? 0) + r.sec);
    }
    pages += p;
    const b = per.get(r.book_id) ?? {
      bookId: r.book_id,
      sec: 0,
      pages: 0,
      formats: [],
      langs: [],
      lastDay: r.day,
      days: 0,
      dayset: new Set<string>(),
    };
    b.sec += r.sec;
    b.pages += p;
    if (!b.formats.includes(f)) b.formats.push(f);
    if (!b.langs.includes(r.lang)) b.langs.push(r.lang);
    if (r.day > b.lastDay) b.lastDay = r.day;
    b.dayset.add(r.day);
    per.set(r.book_id, b);
  }
  const booksOut = [...per.values()]
    .map(({ dayset, ...b }) => ({
      ...b,
      days: dayset.size,
      formats: FORMATS.filter((f) => b.formats.includes(f)),
    }))
    .sort((a, b) => b.sec - a.sec || a.bookId.localeCompare(b.bookId));
  return {
    sec,
    pages,
    pagesEstimated,
    books: booksOut,
    perDay,
    byCategory: sorted(cat),
    byTopic: sorted(topic),
    byLang: sorted(lang),
    byFormat,
    fictionShare: classified > 0 ? fiction / classified : null,
  };
}

/** Minuten je Tag über alle Formate (Zielzeit). */
export function minutesPerDay(rows: ReadingRow[]): Map<string, number> {
  const m = new Map<string, number>();
  for (const r of rows) m.set(r.day, (m.get(r.day) ?? 0) + r.sec / 60);
  return m;
}

export interface GoalState {
  goal: number;
  todayMin: number;
  /** Tage in Folge mit erreichtem Ziel (Tage ohne Ziel überspringen die Kette). */
  streak: number;
  avg7: number;
  avg30: number;
  /** Ziel erreicht an `hit` von `due` Zieltagen im Zeitraum. */
  hit: number;
  due: number;
}

const isDue = (day: string, mask: number) =>
  (mask & (1 << weekdayBit(day))) !== 0;

/**
 * Stand des Tagesziels. `mask` = Wochentage mit Ziel. Heute zählt zur Serie,
 * sobald erreicht; noch nicht erreicht bricht sie nicht ab.
 */
export function goalState(
  rows: ReadingRow[],
  goal: number,
  mask: number,
  today: string,
  period?: Pick<Period, "from" | "to">,
): GoalState {
  const per = minutesPerDay(rows);
  const min = (d: string) => per.get(d) ?? 0;
  const todayMin = min(today);
  let streak = 0;
  if (goal > 0 && mask & 0x7f) {
    let d = today;
    if (!(isDue(d, mask) && min(d) >= goal)) d = addDays(d, -1);
    for (let guard = 0; guard < 3660; guard++, d = addDays(d, -1)) {
      if (!isDue(d, mask)) continue;
      if (min(d) >= goal) streak++;
      else break;
    }
  }
  const avg = (n: number) => {
    let s = 0;
    for (let i = 0; i < n; i++) s += min(addDays(today, -i));
    return s / n;
  };
  let hit = 0;
  let due = 0;
  if (goal > 0 && period) {
    for (let d = period.from; d <= period.to && d <= today; d = addDays(d, 1)) {
      if (!isDue(d, mask)) continue;
      due++;
      if (min(d) >= goal) hit++;
    }
  }
  return { goal, todayMin, streak, avg7: avg(7), avg30: avg(30), hit, due };
}

// ---------------------------------------------------------------------------
// Darstellung
// ---------------------------------------------------------------------------

/** „6:52 h“ bzw. „42 Min.“ */
export function formatDuration(sec: number): string {
  const m = Math.round(sec / 60);
  if (m < 60) return `${m} Min.`;
  return `${Math.floor(m / 60)}:${pad(m % 60)} h`;
}

/** CSV (Semikolon, Excel-tauglich) der Zeilen im Zeitraum. */
export function toCsv(
  rows: ReadingRow[],
  books: LibraryBook[],
  period: Pick<Period, "from" | "to">,
  lengths: (book: LibraryBook) => BookLengths,
): string {
  const byId = new Map(books.map((b) => [b.id, b]));
  // Formeln (=, +, -, @) aus Buchtiteln nicht ausführen lassen.
  const q = (s: string) =>
    `"${(/^[=+\-@\t\r]/.test(s) ? `'${s}` : s).replace(/"/g, '""')}"`;
  const lines = ["Tag;Buch;Format;Sprache;Minuten;Seiten"];
  for (const r of rows) {
    if (r.day < period.from || r.day > period.to) continue;
    const b = byId.get(r.book_id);
    const p = b ? pagesOf(r, b.pages, lengths(b)).pages : 0;
    lines.push(
      [
        r.day,
        q(b?.title ?? r.book_id),
        r.format,
        r.lang,
        (r.sec / 60).toFixed(1).replace(".", ","),
        p ? p.toFixed(1).replace(".", ",") : "",
      ].join(";"),
    );
  }
  return lines.join("\r\n") + "\r\n";
}
