// Vendort aus https://github.com/MrP42/wai-sdk (packages/voice/src, Commit 6e6c8cf917c4) durch scripts/vendor-wai-voice.py.
// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.
/**
 * Textprotokoll für Modelle ohne native Werkzeugaufrufe (ADR-0021, 4.).
 *
 * Das Modell schreibt am Antwortanfang oder an einer Satzgrenze `⟦werkzeug name {json}⟧`.
 * Der Marker wird nie gesprochen: Ab dem ersten `⟦` geht nichts mehr an den Sprecher-Strom,
 * der Rest wird zurückgehalten und erst am Ende ausgewertet.
 */

import type { ToolSpec } from "./model.ts";

export const MARKER_OPEN = "⟦";
export const MARKER_CLOSE = "⟧";
const MARKER_WORD = "werkzeug";
/** Längster zulässiger Marker (Zeichen); darüber gilt er als ungültig. */
export const MAX_MARKER_LENGTH = 16 * 1024;

/** Ein im Text gefundener Aufruf; bei `error` war der Marker unbrauchbar. */
export interface ParsedMarker {
  name: string;
  args: unknown;
  error?: string;
}

/** Index hinter dem schließenden `}` des JSON-Objekts, das bei `start` beginnt; -1 wenn unvollständig. */
function scanJsonObject(s: string, start: number): number {
  let depth = 0;
  let inString = false;
  for (let i = start; i < s.length; i++) {
    const c = s.charAt(i);
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"') inString = false;
    } else if (c === '"') {
      inString = true;
    } else if (c === "{") {
      depth++;
    } else if (c === "}" && --depth === 0) {
      return i + 1;
    }
  }
  return -1;
}

function skipSpace(s: string, i: number): number {
  let j = i;
  while (j < s.length && /\s/.test(s.charAt(j))) j++;
  return j;
}

function invalid(name: string, error: string): ParsedMarker {
  return { name, args: undefined, error };
}

/** Wertet einen Marker ab `start` (Index von `⟦`) aus; liefert Treffer und Index dahinter. */
function parseOne(text: string, start: number): { marker: ParsedMarker; next: number } {
  let i = start + MARKER_OPEN.length;
  if (!text.startsWith(MARKER_WORD, i)) {
    return { marker: invalid("", "Marker beginnt nicht mit „werkzeug“"), next: i };
  }
  i = skipSpace(text, i + MARKER_WORD.length);
  const nameEnd = text.slice(i).search(/[\s{⟧]|$/);
  const name = text.slice(i, i + nameEnd);
  i = skipSpace(text, i + nameEnd);
  if (name === "") return { marker: invalid("", "Werkzeugname fehlt"), next: i };
  if (i >= text.length) return { marker: invalid(name, "Marker nicht abgeschlossen"), next: i };
  if (text.startsWith(MARKER_CLOSE, i)) {
    return { marker: { name, args: {} }, next: i + MARKER_CLOSE.length };
  }
  if (text.charAt(i) !== "{") {
    return { marker: invalid(name, "Argumente müssen ein JSON-Objekt sein"), next: i };
  }
  const end = scanJsonObject(text, i);
  if (end === -1) return { marker: invalid(name, "Marker nicht abgeschlossen"), next: text.length };
  let args: unknown;
  try {
    args = JSON.parse(text.slice(i, end));
  } catch {
    return { marker: invalid(name, "Argumente sind kein gültiges JSON"), next: end };
  }
  const close = skipSpace(text, end);
  if (!text.startsWith(MARKER_CLOSE, close)) {
    return { marker: invalid(name, "Marker nicht abgeschlossen"), next: close };
  }
  return { marker: { name, args }, next: close + MARKER_CLOSE.length };
}

/** Alle Marker in einem Text auswerten (z. B. dem zurückgehaltenen Rest). */
export function parseToolMarkers(text: string): ParsedMarker[] {
  const out: ParsedMarker[] = [];
  let at = text.indexOf(MARKER_OPEN);
  while (at !== -1) {
    const { marker, next } = parseOne(text, at);
    out.push(
      next - at > MAX_MARKER_LENGTH
        ? invalid("", `Marker länger als ${MAX_MARKER_LENGTH / 1024} KB`)
        : marker,
    );
    at = text.indexOf(MARKER_OPEN, Math.max(next, at + MARKER_OPEN.length));
  }
  return out;
}

/** Text ohne Marker (alles ab dem ersten `⟦` fällt weg). */
export function stripToolMarkers(text: string): string {
  const at = text.indexOf(MARKER_OPEN);
  return (at === -1 ? text : text.slice(0, at)).trim();
}

export interface MarkerFilter {
  push(delta: string): void;
  /** Stromende: die zurückgehaltenen Marker auswerten. */
  end(): ParsedMarker[];
  /** Ein Marker hat begonnen; ab jetzt geht nichts mehr an `onText`. */
  readonly markerSeen: boolean;
}

/**
 * Streaming-Filter: reicht Text bis zum Markerbeginn an `onText` weiter und hält alles ab `⟦`
 * zurück, auch wenn der Marker über viele Stücke verteilt ankommt.
 */
export function createMarkerFilter(onText: (text: string) => void): MarkerFilter {
  let held = "";
  let seen = false;
  let overflow = false;
  const hold = (text: string): void => {
    if (overflow) return;
    held += text;
    if (held.length <= MAX_MARKER_LENGTH) return;
    overflow = true;
    held = "";
  };
  return {
    push(delta) {
      if (seen) {
        hold(delta);
        return;
      }
      const at = delta.indexOf(MARKER_OPEN);
      if (at === -1) {
        if (delta) onText(delta);
        return;
      }
      seen = true;
      hold(delta.slice(at));
      if (at > 0) onText(delta.slice(0, at));
    },
    end: () =>
      overflow
        ? [invalid("", `Marker länger als ${MAX_MARKER_LENGTH / 1024} KB`)]
        : parseToolMarkers(held),
    get markerSeen() {
      return seen;
    },
  };
}

/** Anweisung für Modelle ohne native Werkzeuge: Format des Markers und die Liste der Werkzeuge. */
export function toolProtocolPrompt(tools: readonly ToolSpec[]): string {
  const list = tools
    .map(
      (t) =>
        `- ${t.name}: ${t.description} Argumente (JSON-Schema): ${JSON.stringify(t.parameters)}`,
    )
    .join("\n");
  return [
    "Du kannst Werkzeuge nutzen. Dafür schreibst du am Anfang deiner Antwort oder nach einem ganzen Satz",
    `genau eine Zeile im Format ${MARKER_OPEN}${MARKER_WORD} name {"argument":"wert"}${MARKER_CLOSE}`,
    "und hörst danach sofort auf zu schreiben. Das System führt das Werkzeug aus und gibt dir das Ergebnis.",
    "Den Marker liest niemand vor. Ohne Marker antwortest du dem Nutzer direkt.",
    "Verfügbare Werkzeuge:",
    list,
  ].join("\n");
}
