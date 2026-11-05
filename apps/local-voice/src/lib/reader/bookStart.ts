/**
 * Wo das Vorlesen eines Buchs beginnt (Patrick 05.11.2026): nicht bei
 * Titelei, Inhaltsverzeichnis, Impressum, ISBN oder Widmung, sondern beim
 * ersten echten Inhalt. Vorwort und Einleitung zählen als Inhalt, solange
 * `readPreface` gilt (Standard).
 *
 * Arbeitet allein auf dem gespeicherten Seitentext (Markdown-Teilmenge mit
 * `#`-Überschriften, Absätze durch Leerzeilen). Der Vorspann bleibt im Text
 * und über das Inhaltsverzeichnis erreichbar; nur der Startpunkt ohne
 * gemerkte Stelle wandert.
 *
 * Dieselben Regeln stehen in Swift unter
 * `apps/apple/VoiceCore/Sources/VoiceCore/BookStart.swift`; beide prüfen
 * gegen `tests/fixtures/book-start.json`. Offsets sind UTF-16-Einheiten wie
 * überall im Reader.
 */

export interface BookStartOptions {
  /** Vorwort, Einleitung, Prolog mitlesen (Standard: ja). */
  readPreface?: boolean;
}

/** Ergebnis samt Begründung, für Anzeige und Tests. */
export interface BookStart {
  /** Offset, an dem Play ohne gemerkte Stelle beginnt (0 = vorn). */
  offset: number;
  /** Ob das Buch ein Vorwort/eine Einleitung vor dem ersten Kapitel hat. */
  hasPreface: boolean;
}

interface Para {
  start: number;
  text: string;
}

interface Section {
  /** Offset der Überschrift (oder des ersten Absatzes ohne Überschrift). */
  start: number;
  heading: string | null;
  level: number;
  body: Para[];
}

type Kind = "front" | "preface" | "content" | "neutral";

/** Überschriften, die nie Lesetext sind (ganze Überschrift, normalisiert). */
const FRONT_HEADING = new RegExp(
  "^(?:" +
    [
      "inhalt",
      "inhaltsverzeichnis",
      "inhaltsübersicht",
      "inhaltsangabe",
      "verzeichnis",
      "contents",
      "table of contents",
      "toc",
      "impressum",
      "copyright",
      "copyrightseite",
      "copyright page",
      "rechtliche hinweise",
      "titel",
      "titelseite",
      "titelblatt",
      "title",
      "title page",
      "cover",
      "umschlag",
      "frontmatter",
      "front matter",
      "widmung",
      "dedication",
      "motto",
      "epigraph",
      "epigraf",
      "über das buch",
      "über dieses buch",
      "zum buch",
      "das buch",
      "buch",
      "about the book",
      "about this book",
      "über den autor",
      "über die autorin",
      "über die autoren",
      "über die autorinnen",
      "über den verfasser",
      "zum autor",
      "zur autorin",
      "zu den autoren",
      "der autor",
      "die autorin",
      "die autoren",
      "autor",
      "autorin",
      "autoren",
      "about the author",
      "about the authors",
      "bibliografische information",
      "bibliographische information",
      "abbildungsverzeichnis",
      "tabellenverzeichnis",
      "abkürzungsverzeichnis",
      "verzeichnis der abbildungen",
      "list of figures",
      "list of tables",
      "weitere titel",
      "weitere bücher",
      "also by",
      "praise for",
      "lob für",
      "pressestimmen",
      "stimmen zum buch",
    ]
      .map((w) => w.replace(/ /g, "\\s+"))
      .join("|") +
    ")$",
  "u",
);

/** Vorwort & Co.: Inhalt, aber nur mit `readPreface`. */
// Ganze Überschrift: „Vorwort zur zweiten Auflage“ ja, „Einführung in die
// Algebra“ oder „Introduction to Algorithms“ nicht (das sind Kapitel).
const PREFACE_HEADING =
  /^(?:vorwort|geleitwort|vorrede|vorbemerkung|einleitung|einführung|zur einführung|prolog|prologue|preface|foreword|introduction)(?:\s+(?:zur|zum|des|der|von|to the|of the|by)\s.*)?$/u;

/** Starke Zeichen für Impressum/Titelei im Text eines Abschnitts. */
const FRONT_BODY = [
  /\bISBN(?:-1[03])?\b/u,
  /©|\(c\)\s*(?:19|20)\d\d|\bcopyright\b/iu,
  /alle rechte vorbehalten|all rights reserved/iu,
  /deutsche(?:n)?\s+nationalbibliothek|library of congress/iu,
  /originalausgabe|originaltitel|original title|erstausgabe|first published/iu,
  /umschlaggestaltung|covergestaltung|umschlagmotiv|cover design|satz und layout|printed in|gedruckt in/iu,
];

/** Abschnitte darüber sind Kapitel, auch wenn ein Impressum-Wort vorkommt. */
const FRONT_BODY_MAX = 2500;
/** Unter einer gewöhnlichen Überschrift: nur kurze Abschnitte. */
const FRONT_SECTION_MAX = 800;
/** Genre-Zeile einer Titelei. */
const GENRE =
  /^(?:roman|novel|a novel|erzählung|erzählungen|novelle|sachbuch|thriller|krimi|kriminalroman|gedichte|poems|stories)$/iu;
/** Ein Startpunkt hinter so viel Text ist eher ein Irrtum als Vorspann. */
const MAX_SHARE = 0.5;

const PAGE_MARKS = /\{\{S\. ?\d{1,5}\}\}/gu;

/** Überschrift ohne Markdown, Seitenmarken und Satzzeichen, klein. */
export function normalizeHeading(raw: string): string {
  return raw
    .replace(PAGE_MARKS, " ")
    .replace(/^#{1,6}\s+/u, "")
    .replace(/[*_`]/gu, "")
    .replace(/\s+/gu, " ")
    .trim()
    .replace(/[\s.:;,!?…–—-]+$/u, "")
    .replace(/^[\s.:;,…–—-]+/u, "")
    .toLowerCase();
}

function paragraphs(text: string): Para[] {
  const out: Para[] = [];
  const re = /\n[ \t\r]*\n/gu;
  let from = 0;
  const push = (end: number) => {
    const raw = text.slice(from, end);
    const lead = raw.length - raw.trimStart().length;
    const body = raw.trim();
    // Absätze nur aus Seitenmarken zählen nicht.
    if (body && body.replace(PAGE_MARKS, "").trim())
      out.push({ start: from + lead, text: body });
  };
  for (let m = re.exec(text); m; m = re.exec(text)) {
    push(m.index);
    from = m.index + m[0].length;
  }
  push(text.length);
  return out;
}

const HEADING = /^(#{1,6})\s+(.*)$/u;

function sections(paras: Para[]): Section[] {
  const out: Section[] = [];
  for (const p of paras) {
    const first = p.text.split("\n", 1)[0];
    const h = HEADING.exec(first.replace(PAGE_MARKS, "").trim());
    if (h) {
      out.push({ start: p.start, heading: h[2], level: h[1].length, body: [] });
      // Text unter der Überschrift im selben Absatz gehört zum Abschnitt.
      const rest = p.text.slice(first.length).trim();
      if (rest)
        out[out.length - 1].body.push({
          start: p.text.indexOf(rest, first.length) + p.start,
          text: rest,
        });
      continue;
    }
    if (out.length === 0)
      out.push({ start: p.start, heading: null, level: 0, body: [] });
    out[out.length - 1].body.push(p);
  }
  return out;
}

const bodyText = (s: Section) => s.body.map((p) => p.text).join("\n");
const lines = (s: Section) =>
  s.body
    .flatMap((p) => p.text.split(/\r?\n/u))
    .map((l) => l.replace(PAGE_MARKS, "").trim())
    .filter(Boolean);

/** Autorzeile der Titelei, wie sie der EPUB-Import schreibt („von …“). */
const BYLINE = /^(?:von|by)\s+\p{Lu}/u;

/** Ein Satz, der wie Fließtext endet (nicht bloß ein Name oder Titel). */
const PROSE = /[.!?…:"“”»«')]\s*$/u;

/** Indizes der Impressum-Muster, die im Text vorkommen (0 = ISBN). */
function frontHits(text: string): Set<number> {
  const hits = new Set<number>();
  FRONT_BODY.forEach((r, i) => {
    if (r.test(text)) hits.add(i);
  });
  return hits;
}

/** Klar Impressum: eine ISBN oder zwei verschiedene Merkmale. */
const strongFront = (hits: Set<number>) => hits.has(0) || hits.size >= 2;

function looksLikeFront(text: string, max: number): boolean {
  return text.length <= max && strongFront(frontHits(text));
}

/** Inhaltsverzeichnis ohne Überschrift: die meisten Zeilen sind spätere Überschriften. */
function looksLikeToc(s: Section, later: () => Set<string>): boolean {
  const ls = lines(s);
  if (ls.length < 3) return false;
  const laterHeadings = later();
  let hits = 0;
  for (const l of ls) {
    const n = normalizeHeading(
      l.replace(/^[-*•]\s+/u, "").replace(/[\s.·…]*\d{1,4}$/u, ""),
    );
    if (n && laterHeadings.has(n)) hits++;
  }
  return hits * 2 >= ls.length;
}

/**
 * Titelei: erste Überschrift eines Buchs mit nur kurzen Zeilen darunter
 * („von …“, „Roman“, Verlag) — kein Absatz Fließtext.
 */
function looksLikeTitlePage(s: Section): boolean {
  if (s.heading === null || s.level !== 1 || s.body.length === 0) return false;
  const text = bodyText(s);
  // Ohne Autor- oder Genrezeile ist ein kurzer Abschnitt ein kurzes Kapitel.
  const marked = lines(s).some((l) => BYLINE.test(l) || GENRE.test(l));
  return (
    marked &&
    text.length <= 400 &&
    s.body.every(
      (p) => p.text.length <= 200 && !/[.!?…]\s+\p{Lu}/u.test(p.text),
    )
  );
}

function classify(
  s: Section,
  index: number,
  count: number,
  title: string | null,
  later: () => Set<string>,
): Kind {
  const text = bodyText(s);
  if (s.heading !== null) {
    const h = normalizeHeading(s.heading);
    if (FRONT_HEADING.test(h)) return "front";
    if (title && h === title && index > 0) return "front";
    if (PREFACE_HEADING.test(h)) return "preface";
    // Titelei: kurze Seite mit Autor- oder Genrezeile („von …“, „Roman“).
    if (count > 1 && looksLikeTitlePage(s)) return "front";
    if (s.body.length === 0) return "neutral";
    // Unter einer Kapitelüberschrift nur ein kurzes, klares Impressum.
    if (looksLikeFront(text, FRONT_SECTION_MAX)) return "front";
    return looksLikeToc(s, later) ? "front" : "content";
  }
  if (looksLikeFront(text, FRONT_BODY_MAX) || looksLikeToc(s, later))
    return "front";
  return "content";
}

/**
 * Vorspann ohne Überschriften (etwa PDF): führende Absätze überspringen,
 * wenn darunter ein klares Impressum steht (ISBN oder zwei Merkmale).
 * Liefert den Offset des ersten Fließtexts danach, null (nichts
 * überspringen) oder -1 (der ganze Abschnitt ist Impressum).
 */
function preambleStart(s: Section): number | null {
  let lastFront = -1;
  const hits = new Set<number>();
  for (let i = 0; i < s.body.length; i++) {
    const t = s.body[i].text;
    if (t.length > 400 && PROSE.test(t)) break;
    const h = t.length <= FRONT_BODY_MAX ? frontHits(t) : new Set<number>();
    if (h.size === 0) continue;
    h.forEach((x) => hits.add(x));
    lastFront = i;
  }
  if (lastFront < 0 || !strongFront(hits)) return null;
  const next = s.body[lastFront + 1];
  return next ? next.start : -1;
}

/** Offset hinter einem „# “ am Startpunkt (dort steht der sichtbare Text). */
export function skipHeadingMark(text: string, at: number): number {
  return at + (/^#{1,6}[ \t]+/u.exec(text.slice(at, at + 8))?.[0].length ?? 0);
}

/** Startpunkt eines Buchtexts; 0, wenn kein Vorspann erkennbar ist. */
export function bookStart(
  text: string,
  options: BookStartOptions = {},
): BookStart {
  const readPreface = options.readPreface ?? true;
  const secs = sections(paragraphs(text));
  const title =
    secs[0] && secs[0].heading !== null && secs[0].level === 1
      ? normalizeHeading(secs[0].heading)
      : null;
  const norm = secs.map((s) =>
    s.heading === null ? null : normalizeHeading(s.heading),
  );
  // Erst bauen, wenn ein Abschnitt wie ein Verzeichnis aussieht.
  const headingsFrom = (i: number) => () =>
    new Set(norm.slice(i + 1).filter((h): h is string => h !== null));
  // Überschrift ohne Text („# Erster Teil“ vor „## Kapitel 1“): Start dort,
  // wenn der nächste Inhalt tiefer gegliedert ist — nicht bei „# Für Lena“.
  let pending: { start: number; level: number } | null = null;
  // Unterabschnitte eines übersprungenen Abschnitts („# Über den Autor“ →
  // „## Veröffentlichungen“) bleiben übersprungen.
  let skipBelow: number | null = null;
  let offset = 0;
  let hasPreface = false;
  let found = false;
  for (let i = 0; i < secs.length; i++) {
    const s = secs[i];
    if (skipBelow !== null && s.heading !== null && s.level > skipBelow)
      continue;
    skipBelow = null;
    const kind = classify(s, i, secs.length, title, headingsFrom(i));
    if (kind === "neutral") {
      if (!pending || s.level <= pending.level)
        pending = { start: s.start, level: s.level };
      continue;
    }
    if (kind === "preface") hasPreface = true;
    if (kind === "front" || (kind === "preface" && !readPreface)) {
      pending = null;
      if (s.heading !== null) skipBelow = s.level;
      continue;
    }
    const head = pending && pending.level < s.level ? pending.start : null;
    if (s.heading === null) {
      const at = preambleStart(s);
      if (at === -1) continue; // nur Impressum, weiter zum nächsten Abschnitt
      offset = at ?? s.start;
    } else {
      offset = head ?? s.start;
    }
    found = true;
    break;
  }
  if (!found || offset > text.length * MAX_SHARE)
    return { offset: 0, hasPreface };
  return { offset, hasPreface };
}
