/**
 * Kurzbuch (lva#205): ein KI-Dokument mit fester Gliederung
 * (`## Worum es geht`, `## Für wen`, je Kernaussage `## Schlagwort: Satz`,
 * `## Fazit`, siehe `book_ai/kurzbuch.rs`).
 *
 * Die Ansicht wertet die Überschriften aus; fehlt die Gliederung (von Hand
 * umgeschrieben, unvollständig), bleibt es bei der normalen Dokumentansicht.
 */

export const KURZBUCH_TEMPLATE_ID = "builtin:kurzbuch";
const HEAD_ABOUT = "worum es geht";
const HEAD_AUDIENCE = "für wen";
const HEAD_CONCLUSION = "fazit";
/** Lesetempo in Wörtern je Minute. */
export const READ_WPM = 200;
/** Vorlesetempo bei normaler Geschwindigkeit. */
export const LISTEN_WPM = 150;

export interface KurzbuchSection {
  /** Stabile Kennung aus dem Titel (gehört-Zustand übersteht Umsortieren). */
  id: string;
  title: string;
  /** Teil vor dem ersten Doppelpunkt, sonst leer. */
  keyword: string;
  /** Rest des Titels. */
  statement: string;
  /** Offset der Überschrift im Dokumenttext. */
  start: number;
  /** Ende des Abschnitts (Beginn der nächsten Überschrift). */
  end: number;
}

export interface Kurzbuch {
  about: string;
  audience: string[];
  ideas: KurzbuchSection[];
  conclusion: KurzbuchSection;
  words: number;
}

const words = (s: string) => s.split(/\s+/).filter(Boolean).length;

/** Kurze, stabile Kennung (FNV-1a) für einen Titel. */
export function titleId(title: string): string {
  let h = 0x811c9dc5;
  for (const ch of title.trim().toLowerCase()) {
    h ^= ch.codePointAt(0) ?? 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(36);
}

function section(title: string, start: number, end: number): KurzbuchSection {
  const colon = title.indexOf(":");
  const keyword = colon > 0 && colon < 60 ? title.slice(0, colon).trim() : "";
  return {
    id: titleId(title),
    title,
    keyword,
    statement: keyword ? title.slice(colon + 1).trim() : title,
    start,
    end,
  };
}

/** Gliederung aus dem Markdown, oder `null`, wenn es kein Kurzbuch ist. */
export function parseKurzbuch(text: string): Kurzbuch | null {
  const heads: { title: string; start: number; bodyStart: number }[] = [];
  const re = /^##[ \t]+(.+?)[ \t]*$/gm;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    heads.push({
      title: m[1].replace(/[*_]/g, "").trim(),
      start: m.index,
      bodyStart: m.index + m[0].length,
    });
  }
  let about = "";
  let audience: string[] = [];
  let conclusion: KurzbuchSection | null = null;
  const ideas: KurzbuchSection[] = [];
  heads.forEach((h, i) => {
    const end = i + 1 < heads.length ? heads[i + 1].start : text.length;
    const body = text.slice(h.bodyStart, end).trim();
    const key = h.title.toLowerCase();
    if (key === HEAD_ABOUT) about = body.replace(/\s+/g, " ");
    else if (key === HEAD_AUDIENCE)
      audience = body
        .split("\n")
        .map((l) => l.replace(/^\s*[-*•]\s*/, "").trim())
        .filter(Boolean);
    else if (key === HEAD_CONCLUSION)
      conclusion = section(h.title, h.start, end);
    else if (!conclusion) ideas.push(section(h.title, h.start, end));
  });
  if (!conclusion || ideas.length === 0 || (!about && audience.length === 0))
    return null;
  return { about, audience, ideas, conclusion, words: words(text) };
}

/** Minuten, mindestens 1. */
export const minutes = (wordCount: number, wpm: number) =>
  Math.max(1, Math.round(wordCount / wpm));

/** Abschnitt (Kernaussage oder Fazit), in dem `offset` liegt. */
export function sectionAt(
  kb: Kurzbuch,
  offset: number,
): KurzbuchSection | null {
  return (
    [...kb.ideas, kb.conclusion].find(
      (s) => offset >= s.start && offset < s.end,
    ) ?? null
  );
}

// ---------------------------------------------------------------------------
// Gehört je Kernaussage
// ---------------------------------------------------------------------------

/**
 * Eine Kernaussage gilt als gehört, wenn das Vorlesen in ihrem ersten Viertel
 * war und danach ihr letztes Sechstel erreicht hat. Ein Sprung über sie
 * hinweg markiert nichts (Codex-Auflage 5). Ablage: `heard.ts`.
 */
/**
 * Nächster Stand nach einem vorgelesenen Satz `[start, end)`. `entered`
 * sammelt Abschnitte, deren Anfang gehört wurde; liefert die Kennung, die
 * jetzt neu als gehört gilt, oder `null`.
 */
export function trackSentence(
  kb: Kurzbuch,
  entered: Set<string>,
  start: number,
  end: number,
): string | null {
  const s = sectionAt(kb, start);
  if (!s) return null;
  const len = Math.max(1, s.end - s.start);
  if ((start - s.start) / len <= 0.25) entered.add(s.id);
  if (entered.has(s.id) && (end - s.start) / len >= 5 / 6) {
    entered.delete(s.id);
    return s.id;
  }
  return null;
}
