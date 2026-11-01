/**
 * Lesedokument: der Seitentext als Folge von Blöcken für die Buchansicht.
 *
 * Der Seitentext bleibt die einzige Quelle (abwärtskompatibel, siehe lva#148).
 * Er kennt drei Arten Auszeichnung:
 *
 * - Sprecher `<Name>` / `<Name:Stil>` (und das Alt-Format `Name:` am
 *   Zeilenanfang) — dieselbe Regel wie `speakerMarkers.ts` und die Pipeline.
 * - Tags `[…]` (Ausdruck, Pausen).
 * - Ein schlankes Markdown-Subset für die Darstellung: `#`/`##`/`###`
 *   Überschriften, `> ` Zitat, `- `/`* ` Aufzählung, `---`/`***`/`⁂`
 *   Trenner, `**fett**`, `*kursiv*`/`_kursiv_`.
 * - Seitenmarken `{{S. 12}}` aus dem PDF-Import: die Seitenzahl des Originals.
 *   Die Buchansicht zeigt sie klein im Text, die Synthese liest sie nicht.
 *
 * Die Buchansicht zeigt Sprecher am Rand und blendet Tags aus; die Synthese
 * bekommt den Text ohne Markdown-Zeichen (`spokenScript`). Alle Offsets sind
 * UTF-16-Offsets im Seitentext, damit Hervorhebung, Befunde und Sprünge an
 * dieselbe Stelle zeigen wie der Editor.
 */
import {
  resolveSpeaker,
  scanMarkerCandidates,
  type SpeakerRef,
} from "@/lib/voices/speakerMarkers";

export interface ReaderRun {
  text: string;
  /** Offsets des sichtbaren Texts im Seitentext. */
  start: number;
  end: number;
  bold?: boolean;
  italic?: boolean;
  /** `[…]`-Tag: im Lesemodus ausgeblendet, im Bearbeiten-Modus ein Chip.
   *  Auch Seitenmarken sind `tag` (kein Lesetext), zusätzlich mit `page`. */
  tag?: boolean;
  /** Seitenzahl des Originals aus `{{S. n}}`. */
  page?: number;
}

/** Seitenmarke `{{S. 12}}` (lva#149), am Anfang des restlichen Texts. */
export const PAGE_MARK = /^\{\{S\. ?(\d{1,5})\}\}/;

export type ReaderBlockKind =
  "heading" | "paragraph" | "quote" | "item" | "rule";

export interface ReaderSpeaker {
  name: string;
  style?: string;
  /** Aufgelöste Stimme; fehlt, wenn der Name keiner bekannten Stimme entspricht. */
  ref?: SpeakerRef;
}

export interface ReaderBlock {
  kind: ReaderBlockKind;
  /** 1–3 bei Überschriften. */
  level?: number;
  /** Sprecher, der diesen Block liest (gilt fort bis zum nächsten Marker). */
  speaker?: ReaderSpeaker;
  /** Gleicher Sprecher wie im Block davor: der Rand zeigt nur den Farbstrich. */
  continues: boolean;
  /** Beginnt in diesem Block ausdrücklich ein Marker? */
  marked: boolean;
  runs: ReaderRun[];
  start: number;
  end: number;
}

const RULE = /^\s*(?:-{3,}|\*{3,}|_{3,}|⁂|\* \* \*)\s*$/;

/** Teilt eine Zeile in Abschnitte an Sprechermarkern. */
function splitAtMarkers(
  line: string,
  offset: number,
  speakers: SpeakerRef[],
): { text: string; start: number; marker?: ReaderSpeaker }[] {
  const out: { text: string; start: number; marker?: ReaderSpeaker }[] = [];
  let from = 0;
  let pending: ReaderSpeaker | undefined;

  const lead = line.length - line.trimStart().length;
  const colon = line.indexOf(":", lead);
  if (colon > lead) {
    const name = line.slice(lead, colon).trim();
    const ref = resolveSpeaker(name, speakers);
    if (ref) {
      pending = { name, ref };
      from = colon + 1;
    }
  }

  for (const c of scanMarkerCandidates(line.slice(from))) {
    const inner = line.slice(from + c.start + 1, from + c.end - 1);
    if (/^\s/.test(inner) || /\s$/.test(inner)) continue;
    const s = from + c.start;
    const before = line.slice(from, s);
    if (before.trim() || pending) {
      out.push({ text: before, start: offset + from, marker: pending });
    }
    pending = {
      name: c.name.trim(),
      style: c.style,
      ref: resolveSpeaker(c.name, speakers) ?? undefined,
    };
    from = from + c.end;
  }
  out.push({ text: line.slice(from), start: offset + from, marker: pending });
  return out.filter((p) => p.text.trim() || p.marker);
}

/** Inline-Auszeichnung (`**`, `*`, `_`, `[…]`) in Läufe zerlegen. */
export function parseInline(text: string, offset: number): ReaderRun[] {
  const runs: ReaderRun[] = [];
  let bold = false;
  let italic = false;
  let buf = "";
  let bufStart = offset;
  const flush = (at: number) => {
    if (buf)
      runs.push({
        text: buf,
        start: bufStart,
        end: offset + at,
        bold: bold || undefined,
        italic: italic || undefined,
      });
    buf = "";
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === "{" && text[i + 1] === "{") {
      const m = PAGE_MARK.exec(text.slice(i, i + 16));
      if (m) {
        flush(i);
        runs.push({
          text: m[0],
          start: offset + i,
          end: offset + i + m[0].length,
          tag: true,
          page: Number(m[1]),
        });
        i += m[0].length;
        bufStart = offset + i;
        continue;
      }
    }
    if (c === "[") {
      const close = text.indexOf("]", i + 1);
      const nl = text.indexOf("\n", i + 1);
      if (close > i + 1 && (nl === -1 || close < nl)) {
        flush(i);
        runs.push({
          text: text.slice(i, close + 1),
          start: offset + i,
          end: offset + close + 1,
          tag: true,
        });
        i = close + 1;
        bufStart = offset + i;
        continue;
      }
    }
    if (c === "*" && text[i + 1] === "*") {
      flush(i);
      bold = !bold;
      i += 2;
      bufStart = offset + i;
      continue;
    }
    // Einzelnes `*`/`_` nur als Hervorhebung, wenn es an einer Wortgrenze steht
    // ("3*4" und "snake_case" bleiben Text).
    if ((c === "*" || c === "_") && isEmphasisDelimiter(text, i, italic)) {
      flush(i);
      italic = !italic;
      i += 1;
      bufStart = offset + i;
      continue;
    }
    if (!buf) bufStart = offset + i;
    buf += c;
    i += 1;
  }
  flush(text.length);
  return runs;
}

function isEmphasisDelimiter(text: string, i: number, open: boolean): boolean {
  const prev = text[i - 1];
  const next = text[i + 1];
  const word = (ch: string | undefined) => !!ch && /[\p{L}\p{N}]/u.test(ch);
  if (open) return !!prev && !/\s/.test(prev) && !word(next);
  if (!next || /\s/.test(next)) return false;
  if (word(prev)) return false;
  return text.indexOf(text[i], i + 1) !== -1;
}

/** Den Seitentext in Blöcke zerlegen. */
export function parseReaderDoc(
  text: string,
  speakers: SpeakerRef[],
): ReaderBlock[] {
  const blocks: ReaderBlock[] = [];
  let current: ReaderSpeaker | undefined;
  let lineStart = 0;
  while (lineStart <= text.length) {
    const nl = text.indexOf("\n", lineStart);
    const lineEnd = nl === -1 ? text.length : nl;
    const line = text.slice(lineStart, lineEnd);

    if (RULE.test(line)) {
      blocks.push({
        kind: "rule",
        continues: false,
        marked: false,
        runs: [],
        start: lineStart,
        end: lineEnd,
      });
    } else if (line.trim()) {
      for (const part of splitAtMarkers(line, lineStart, speakers)) {
        let body = part.text;
        let bodyStart = part.start;
        let kind: ReaderBlockKind = "paragraph";
        let level: number | undefined;
        // Führende Leerzeichen (nach `<Name>`) gehören nicht zum Lesetext:
        // sonst rückt die erste Zeile gegenüber den folgenden ein.
        const lead = body.length - body.trimStart().length;
        const rest = body.slice(lead);
        bodyStart += lead;
        body = rest;
        const h = /^(#{1,3})\s+/.exec(rest);
        const q = /^>\s?/.exec(rest);
        const li = /^[-*•]\s+/.exec(rest);
        const m = h ?? q ?? li;
        if (m) {
          kind = h ? "heading" : q ? "quote" : "item";
          level = h ? h[1].length : undefined;
          bodyStart += m[0].length;
          body = rest.slice(m[0].length);
        }
        if (part.marker) current = part.marker;
        const prev = blocks[blocks.length - 1];
        const speaker = kind === "heading" ? undefined : current;
        const continues =
          !part.marker &&
          !!speaker &&
          !!prev?.speaker &&
          prev.speaker.name === speaker.name;
        const runs = parseInline(body, bodyStart);
        if (!runs.length && !part.marker) continue;
        blocks.push({
          kind,
          level,
          speaker,
          continues,
          marked: !!part.marker,
          runs,
          start: part.start,
          end: part.start + part.text.length,
        });
      }
    }
    if (nl === -1) break;
    lineStart = nl + 1;
  }
  return blocks;
}

/** Sichtbarer Text eines Blocks ohne Tags (für Suche und Barrierefreiheit). */
export const blockPlainText = (block: ReaderBlock): string =>
  block.runs
    .filter((r) => !r.tag)
    .map((r) => r.text)
    .join("");
