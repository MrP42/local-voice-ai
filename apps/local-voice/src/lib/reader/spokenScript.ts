/**
 * Der Text, den die Synthese bekommt: Seitentext ohne Markdown-Zeichen.
 *
 * Sprecher-Marker und Tags bleiben stehen — sie steuern Stimme und Ausdruck.
 * Weg fallen nur Zeichen, die eine Engine sonst vorlesen oder als Pause
 * deuten würde (`#`, `**`, `> `, `- `). Eine Überschrift ohne Satzzeichen
 * bekommt einen Punkt: so endet sie als eigener Satz mit natürlicher Pause,
 * statt mit dem ersten Satz des Kapitels zu verschmelzen. Ein Trenner wird
 * zur Leerzeile. Seitenmarken `{{S. 12}}` aus dem PDF-Import fallen ganz weg
 * (samt einem Leerzeichen davor), damit die Engine keine Zahl vorliest.
 *
 * Ohne Markdown im Text ist das Ergebnis zeichengleich mit der Eingabe —
 * bestehende Seiten, Satz-Cache und Satzzähler ändern sich dadurch nicht.
 */
import { parseInline } from "./readerDoc";

const RULE = /^\s*(?:-{3,}|\*{3,}|_{3,}|⁂|\* \* \*)\s*$/;
const HAS_MARKDOWN =
  /(^|\n)\s*(#{1,3}\s|>\s?|[-*•]\s|-{3,}|\*{3,}|⁂)|\*\*|(^|[\s(„"])[*_][^\s*_]|\{\{S\./;
const PAGE_MARKS = / ?\{\{S\. ?\d{1,5}\}\} ?/g;

export function toSpokenScript(text: string): string {
  if (!HAS_MARKDOWN.test(text)) return text;
  return text
    .split("\n")
    .map((raw) => {
      // Eine Marke zwischen zwei Wörtern lässt genau ein Leerzeichen stehen.
      const line = raw.replace(PAGE_MARKS, (m, at: number) =>
        at > 0 && m.startsWith(" ") && m.endsWith(" ") ? " " : "",
      );
      if (RULE.test(line)) return "";
      const lead = line.length - line.trimStart().length;
      let rest = line.slice(lead);
      // Ein Sprecher-Marker darf vor der Auszeichnung stehen: `<Mara> > Zitat`.
      const marker = /^(<[^<>\n]{1,60}>\s*)/.exec(rest);
      const prefix = marker ? marker[1] : "";
      rest = rest.slice(prefix.length);
      const heading = /^#{1,3}\s+/.exec(rest);
      const block = heading ?? /^>\s?/.exec(rest) ?? /^[-*•]\s+/.exec(rest);
      if (block) rest = rest.slice(block[0].length);
      let body = parseInline(rest, 0)
        .map((r) => r.text)
        .join("");
      if (heading && body.trim() && !/[.!?…:;]["“”»«']?\s*$/.test(body)) {
        body = body.trimEnd() + ".";
      }
      return line.slice(0, lead) + prefix + body;
    })
    .join("\n");
}

/** Seitenmarken entfernen, z. B. bevor ein Sprachmodell den Text übersetzt,
 *  glättet oder zusammenfasst: es würde sie sonst mit übersetzen oder verlieren. */
export const stripPageMarks = (text: string): string =>
  text.replace(PAGE_MARKS, (m, at: number) =>
    at > 0 && m.startsWith(" ") && m.endsWith(" ") ? " " : "",
  );
