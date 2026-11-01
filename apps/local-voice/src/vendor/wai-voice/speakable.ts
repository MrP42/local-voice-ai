// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
/**
 * Sprechbarer Text: Markdown, Listen, Links und Code entfernen, in Sätze teilen, auf N Sätze kürzen.
 * Der Strom arbeitet inkrementell (Sätze gehen heraus, sobald ihr Ende feststeht) und liefert
 * bei jeder Zerlegung der Eingabe in Stücke dasselbe wie die Stapelfassung.
 */

const ABBREVIATIONS = new Set([
  "bzw",
  "ca",
  "dr",
  "evtl",
  "ggf",
  "inkl",
  "max",
  "min",
  "nr",
  "prof",
  "sog",
  "str",
  "tel",
  "usw",
  "vgl",
  "z",
  "d",
  "u",
  "o",
  "v",
  "etc",
  "ff",
]);

const SENTENCE_END = new Set([".", "!", "?", "…"]);
const CLOSERS = new Set(['"', "”", "“", "»", "«", ")", "'"]);
const HAS_CONTENT = /[\p{L}\p{N}]/u;

/** Entfernt Markdown- und Linkzeichen aus einer einzelnen Zeile bzw. einem Satz. */
export function cleanForSpeech(line: string): string {
  return line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/<[^>\n]+>/g, "")
    .replace(/\b(?:https?:\/\/|www\.)\S+/gi, "")
    .replace(/^\s{0,3}#{1,6}\s+/, "")
    .replace(/^\s*>+\s?/, "")
    .replace(/^\s*[-*+•]\s+/, "")
    .replace(/^\s*\d+[.)]\s+/, "")
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/__(.+?)__/g, "$1")
    .replace(/~~(.+?)~~/g, "$1")
    .replace(/\*(\S(?:[^*]*?\S)?)\*/g, "$1")
    .replace(/(^|[^\p{L}\p{N}])_(\S(?:[^_]*?\S)?)_(?![\p{L}\p{N}])/gu, "$1$2")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/[`*⟦⟧]/g, "")
    .replace(/\p{Extended_Pictographic}/gu, "")
    .replace(/\s+/g, " ")
    .replace(/\s+([,.;:!?])/g, "$1")
    .trim();
}

function endsAbbreviation(text: string, dotIndex: number): boolean {
  const before = text.slice(0, dotIndex);
  const token = before.slice(before.search(/[^\s(„"“»]*$/)).toLowerCase();
  if (token === "") return false;
  if (/^\d+$/.test(token)) return true;
  return token.length === 1 || ABBREVIATIONS.has(token);
}

/** Index hinter dem ersten sicheren Satzende (inkl. Schlusszeichen) oder -1. */
function findBoundary(buf: string): number {
  for (let i = 0; i < buf.length; i++) {
    const c = buf.charAt(i);
    if (!SENTENCE_END.has(c)) continue;
    let j = i + 1;
    while (j < buf.length && SENTENCE_END.has(buf.charAt(j))) j++;
    const single = j === i + 1 && c === ".";
    while (j < buf.length && CLOSERS.has(buf.charAt(j))) j++;
    if (j >= buf.length) return -1;
    if (!/\s/.test(buf.charAt(j))) {
      i = j - 1;
      continue;
    }
    if (single && endsAbbreviation(buf, i)) {
      i = j - 1;
      continue;
    }
    return j;
  }
  return -1;
}

export interface SpeakableStreamOptions {
  /** Höchstzahl gelieferter Sätze; danach wird alles weitere verworfen. */
  maxSentences?: number;
  onSentence(sentence: string): void;
  /** Wird einmal gerufen, sobald die Höchstzahl erreicht ist. */
  onLimit?: () => void;
}

export interface SpeakableStream {
  push(delta: string): void;
  /** Eingabe zu Ende: den Rest als letzten Satz liefern. */
  end(): void;
  /** Anzahl bisher gelieferter Sätze. */
  readonly count: number;
  /** Höchstzahl erreicht. */
  readonly done: boolean;
}

export function createSpeakableStream(opts: SpeakableStreamOptions): SpeakableStream {
  const max = opts.maxSentences ?? Number.POSITIVE_INFINITY;
  let buffer = "";
  let inCode = false;
  let count = 0;
  let done = max <= 0;

  const emit = (text: string): void => {
    if (done) return;
    count++;
    opts.onSentence(text);
    if (count >= max) {
      done = true;
      opts.onLimit?.();
    }
  };

  const unit = (raw: string): void => {
    const line = raw.trim();
    if (line.startsWith("```")) {
      inCode = !inCode;
      return;
    }
    if (inCode || line === "" || line.startsWith("|") || /^[-*_=]{3,}$/.test(line)) return;
    const cleaned = cleanForSpeech(line);
    if (HAS_CONTENT.test(cleaned)) emit(cleaned);
  };

  const drain = (): void => {
    while (!done) {
      const nl = buffer.indexOf("\n");
      const b = findBoundary(buffer);
      if (nl !== -1 && (b === -1 || nl < b)) {
        unit(buffer.slice(0, nl));
        buffer = buffer.slice(nl + 1);
      } else if (b !== -1) {
        unit(buffer.slice(0, b));
        buffer = buffer.slice(b);
      } else {
        return;
      }
    }
  };

  return {
    push(delta) {
      if (done) return;
      buffer += delta;
      drain();
    },
    end() {
      if (done) return;
      drain();
      if (done) return;
      const rest = buffer;
      buffer = "";
      unit(rest);
    },
    get count() {
      return count;
    },
    get done() {
      return done;
    },
  };
}

/** Text auf höchstens `maxSentences` sprechbare Sätze kürzen (durch Leerzeichen verbunden). */
export function toSpeakable(text: string, maxSentences = Number.POSITIVE_INFINITY): string {
  const sentences: string[] = [];
  const stream = createSpeakableStream({ maxSentences, onSentence: (s) => sentences.push(s) });
  stream.push(text);
  stream.end();
  return sentences.join(" ");
}
