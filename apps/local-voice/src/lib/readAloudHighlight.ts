/**
 * Den gerade vorgelesenen Satz im angezeigten Dokument finden (Protokoll, Dokumente,
 * Projektprotokoll).
 *
 * Vorgelesen wird nicht das Markdown, sondern Klartext (`markdownToSpeech`): ohne
 * Auszeichnung, Folienbelege und Tabellenstriche, mit einem Punkt hinter Überschriften und
 * Listenpunkten, Tabellenzellen mit Komma verbunden. Ein Offset im Sprechtext passt deshalb
 * nicht auf das DOM. Gesucht wird über die Buchstaben und Ziffern allein: Satzzeichen,
 * Leerraum und Groß-/Kleinschreibung zählen nicht, und das DOM darf zwischen zwei
 * Buchstaben des Satzes etwas haben, das nicht gesprochen wird (Beleg `F3`, Metaangaben).
 *
 * Gemalt wird über die CSS Custom Highlight API wie im Vorlesen-Modul (ReaderView): keine
 * Spans im Dokument, das Layout bleibt stehen, Kopieren nach Word liefert dasselbe HTML.
 */

const WORD_CHAR = /[\p{L}\p{N}]/u;

/** Höchstens so viele ungesprochene Zeichen dürfen zwischen zwei Satzzeichen liegen. */
const MAX_GAP = 40;
/** So viele Zeichen am Satzanfang müssen genau passen, bevor ein Treffer geprüft wird. */
const HEAD = 6;
/** Satzzeichen direkt hinter dem letzten Buchstaben gehören zur Markierung. */
const TRAILING = /[.!?…:;,"'»«“”)\]]/;

/** Buchstaben und Ziffern eines Texts, klein geschrieben, je UTF-16-Einheit. */
export function normalizeForMatch(text: string): string {
  let out = "";
  for (const ch of text) {
    if (!WORD_CHAR.test(ch)) continue;
    out += ch.toLowerCase();
  }
  return out;
}

/** Die Buchstabenfolge eines Teilbaums und zu jedem Zeichen sein Textknoten + Offset. */
export interface CharStream {
  chars: string;
  nodes: Text[];
  /** Je Zeichen in `chars`: Index in `nodes`. */
  nodeOf: number[];
  /** Je Zeichen in `chars`: Offset im Textknoten (UTF-16) und Länge des Zeichens. */
  offsetOf: number[];
  lengthOf: number[];
  /** Je Zeichen: beginnt hier ein Wort (Knotenanfang oder Nicht-Buchstabe davor)? */
  wordStart: boolean[];
}

export function buildCharStream(root: Node): CharStream {
  const stream: CharStream = {
    chars: "",
    nodes: [],
    nodeOf: [],
    offsetOf: [],
    lengthOf: [],
    wordStart: [],
  };
  const parts: string[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const node = n as Text;
    const text = node.data;
    const nodeIndex = stream.nodes.length;
    let used = false;
    let offset = 0;
    let inWord = false;
    for (const ch of text) {
      const isWord = WORD_CHAR.test(ch);
      if (isWord) {
        const lower = ch.toLowerCase();
        // Ein Zeichen kann klein geschrieben aus mehreren Einheiten bestehen ("İ"):
        // jede Einheit zeigt auf dasselbe Zeichen im Knoten.
        for (let k = 0; k < lower.length; k++) {
          parts.push(lower[k]);
          stream.nodeOf.push(nodeIndex);
          stream.offsetOf.push(offset);
          stream.lengthOf.push(ch.length);
          stream.wordStart.push(k === 0 && !inWord);
        }
        used = true;
      }
      inWord = isWord;
      offset += ch.length;
    }
    if (used) stream.nodes.push(node);
  }
  stream.chars = parts.join("");
  return stream;
}

/** Endet ein Treffer, der genau bei `p` beginnt? Dann das Ende (exklusiv), sonst null. */
function matchAt(chars: string, sentence: string, p: number): number | null {
  let i = p;
  for (let k = 0; k < sentence.length; k++) {
    const limit = k === 0 ? i : i + MAX_GAP;
    while (i < chars.length && i <= limit && chars[i] !== sentence[k]) i++;
    if (i >= chars.length || i > limit) return null;
    i++;
  }
  return i;
}

export interface MatchSpan {
  /** Erstes Zeichen in `CharStream.chars`. */
  start: number;
  /** Hinter dem letzten Zeichen. */
  end: number;
}

/**
 * Den Satz (bereits `normalizeForMatch`) ab `from` suchen; findet sich dort nichts, von
 * vorn. `from` kommt vom zuletzt gefundenen Satz: gleiche Sätze ("Keine.") werden so in
 * Lesereihenfolge getroffen und nicht immer am ersten Vorkommen.
 */
export function locateSentence(
  stream: Pick<CharStream, "chars" | "wordStart">,
  sentence: string,
  from = 0,
): MatchSpan | null {
  const { chars, wordStart } = stream;
  if (!sentence || !chars) return null;
  // Nur an Wortgrenzen: "Ja." trifft nicht das "Ja" in "Januar".
  const bounded = (start: number, end: number) =>
    wordStart[start] && (end >= chars.length || wordStart[end]);
  const starts = from > 0 ? [from, 0] : [0];
  // Zuerst wörtlich: der Normalfall, und schnell.
  for (const begin of starts) {
    for (
      let p = chars.indexOf(sentence, begin);
      p >= 0;
      p = chars.indexOf(sentence, p + 1)
    )
      if (bounded(p, p + sentence.length))
        return { start: p, end: p + sentence.length };
  }
  // Sonst mit Lücken (ungesprochene Zeichen im Dokument). Ein Treffer mit wenigen
  // Lückenzeichen gilt sofort; sonst der mit den wenigsten -- so greift "Punkt 70" nicht
  // schon bei "Punkt 69" und überspringt den Rest bis zur 7.
  const tolerance = Math.max(6, Math.floor(sentence.length / 4));
  const head = sentence.slice(0, Math.min(HEAD, sentence.length));
  let best: MatchSpan | null = null;
  for (const begin of starts) {
    for (
      let p = chars.indexOf(head, begin);
      p >= 0;
      p = chars.indexOf(head, p + 1)
    ) {
      if (!wordStart[p]) continue;
      const end = matchAt(chars, sentence, p);
      if (end === null || !bounded(p, end)) continue;
      const skipped = end - p - sentence.length;
      if (skipped <= tolerance) return { start: p, end };
      if (!best || skipped < best.end - best.start - sentence.length)
        best = { start: p, end };
    }
  }
  return best;
}

/** DOM-Bereiche über einen Treffer, je Textknoten eine (Zwischenraum im Knoten inklusive). */
export function rangesOfSpan(
  stream: CharStream,
  span: MatchSpan,
): StaticRange[] {
  const out: StaticRange[] = [];
  let i = span.start;
  while (i < span.end) {
    const node = stream.nodeOf[i];
    let last = i;
    while (last + 1 < span.end && stream.nodeOf[last + 1] === node) last++;
    const text = stream.nodes[node];
    // Ab dem zweiten Stück den Leerraum davor mitnehmen ("**Angebot** bis"): sonst
    // klafft zwischen zwei Wörtern eine Lücke in der Markierung.
    let from = stream.offsetOf[i];
    if (out.length > 0)
      while (from > 0 && /\s/.test(text.data[from - 1])) from--;
    let to = stream.offsetOf[last] + stream.lengthOf[last];
    // Am Satzende das Satzzeichen mitnehmen ("eintragen." statt "eintragen"), sonst
    // den Leerraum bis zum nächsten Stück ("im [Kalender]").
    const tail = last + 1 === span.end ? TRAILING : /\s/;
    while (to < text.length && tail.test(text.data[to])) to++;
    // StaticRange statt Range: eine lebende Range klappt zusammen, sobald die Ansicht
    // das Dokument im DOM umhängt (schmales Fenster), die Textknoten aber bleiben.
    out.push(
      new StaticRange({
        startContainer: text,
        startOffset: from,
        endContainer: text,
        endOffset: to,
      }),
    );
    i = last + 1;
  }
  return out;
}

/** Kann die Ansicht über die Highlight-API malen? (WebView2, WebKit ab 17.2.) */
export function canPaintHighlights(): boolean {
  if (typeof window === "undefined" || typeof CSS === "undefined") return false;
  return (
    "highlights" in CSS &&
    typeof Highlight !== "undefined" &&
    typeof StaticRange !== "undefined"
  );
}
