/**
 * Markdown (Protokoll, Dokumente, Notizen) in Klartext fürs Vorlesen wandeln.
 *
 * Die Vorlese-Engine liest, was sie bekommt: `**fett**`, `#`-Überschriften, Tabellenstriche
 * und Folienbelege wie `[F7]` würden mitgesprochen (oder als Stil-Tag missverstanden). Hier
 * bleibt nur der Text. Überschriften und Listenpunkte enden mit einem Punkt, damit die
 * Engine dort eine Pause macht; Tabellenzeilen werden zu "Zelle, Zelle, Zelle.".
 */

const SENTENCE_END = /[.!?…:;]$/;

const endSentence = (line: string): string => {
  const t = line.trim();
  if (!t) return "";
  return SENTENCE_END.test(t) ? t : `${t}.`;
};

/** Inline-Auszeichnung einer Zeile entfernen; der sichtbare Text bleibt. */
const stripInline = (line: string): string =>
  line
    // Bilder ganz weg, Links auf ihren Text
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    // Beleg-Marken: Folien `[F7]`, Zeitmarken `[12:34]`
    .replace(/\[F\d+\]/g, "")
    .replace(/\[\d{1,2}:\d{2}(?::\d{2})?\]/g, "")
    // Code, Fett, Kursiv, Durchgestrichen (Unterstrich nur an Wortgrenzen)
    .replace(/`+([^`]*)`+/g, "$1")
    .replace(/(\*\*|__)(.+?)\1/g, "$2")
    .replace(/(^|[\s(])\*([^*\s][^*]*?)\*(?=[\s).,;:!?]|$)/g, "$1$2")
    .replace(/(^|[\s(])_([^_\s][^_]*?)_(?=[\s).,;:!?]|$)/g, "$1$2")
    .replace(/~~(.+?)~~/g, "$1")
    // Reste von HTML
    .replace(/<[^>]+>/g, "")
    .replace(/\s{2,}/g, " ")
    .trim();

const isTableSeparator = (line: string): boolean =>
  /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line);

export function markdownToSpeech(markdown: string): string {
  const out: string[] = [];
  let inFence = false;
  for (const raw of markdown.replace(/\r\n?/g, "\n").split("\n")) {
    if (/^\s*(```|~~~)/.test(raw)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      const code = raw.trim();
      if (code) out.push(code);
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(raw)) continue; // Trennlinie
    if (isTableSeparator(raw) && raw.includes("-")) continue;

    const heading = /^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$/.exec(raw);
    if (heading) {
      const text = endSentence(stripInline(heading[1]));
      if (text) out.push("", text);
      continue;
    }
    if (/^\s*\|.*\|\s*$/.test(raw)) {
      const cells = raw
        .trim()
        .slice(1, -1)
        .split("|")
        .map((c) => stripInline(c))
        .filter(Boolean);
      if (cells.length) out.push(endSentence(cells.join(", ")));
      continue;
    }
    const item = /^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?(.*)$/.exec(raw);
    if (item) {
      const text = endSentence(stripInline(item[1]));
      if (text) out.push(text);
      continue;
    }
    const quote = /^\s*>+\s?(.*)$/.exec(raw);
    const text = stripInline(quote ? quote[1] : raw);
    // Leerzeile = Absatz; mehrere hintereinander zählen einmal.
    if (!text) {
      if (out.length > 0 && out[out.length - 1] !== "") out.push("");
      continue;
    }
    out.push(text);
  }
  return out
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
