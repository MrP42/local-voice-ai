/**
 * Skript-Pruefung fuer den Vorlesen-Editor: findet im Text, was beim
 * Vorlesen anders klaenge als gemeint. Reine Funktion ohne UI, damit jede
 * Regel ohne Oberflaeche pruefbar ist.
 *
 * Zwei Befundarten sind Pflicht:
 * - `unknown-speaker`: ein `<Name>`/`<Name:Stil>`-Marker, dessen Name keiner
 *   Stimme gehoert. Das Backend streicht ihn und faellt auf die
 *   Standardstimme zurueck (`split_speaker_segments` in protocol.rs) --
 *   gemeint war aber offensichtlich jemand anderes.
 * - `unknown-tag`: ein `[…]`-Tag, das die aktive Engine nicht kennt. Fish
 *   Speech kennt genau die Registry; Piper kennt gar keine Stil-Tags, nur
 *   die Pausen, die die App selbst als Stille einsetzt.
 *
 * Die Erkennungsregeln (welche Spans ein Marker bzw. ein Tag sind) sind
 * dieselben wie in den Chip-Providern: `scanMarkerCandidates` und
 * `scanTagMatches` -- was dort kein Fund ist, ist hier kein Befund.
 */
import { TAG_REGISTRY } from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";
import { scanTagMatches } from "@/components/settings/tts/tags/tagProvider";
import {
  resolveSpeaker,
  scanMarkerCandidates,
  type SpeakerRef,
} from "./speakerMarkers";

export type ScriptFindingKind = "unknown-speaker" | "unknown-tag";

export interface ScriptFinding {
  kind: ScriptFindingKind;
  /** UTF-16-Offsets der betroffenen Stelle inklusive Klammern. */
  start: number;
  end: number;
  /** Der exakte Ausschnitt, z. B. `<Bob>` oder `[mysterious]`. */
  raw: string;
  /** Sprechername bzw. Tag-Inhalt ohne Klammern, getrimmt. */
  name: string;
  /** Stil aus `<Name:Stil>`, falls vorhanden -- bleibt beim Ersetzen. */
  style?: string;
  /** 1-basierte Zeile, damit die Befundliste die Stelle benennen kann. */
  line: number;
}

export type ScriptEngine = "fish" | "piper";

/** Tags, die die Engine wirklich versteht (oder die App selbst umsetzt). */
export function knownTagsFor(engine: ScriptEngine): TagDef[] {
  return engine === "piper"
    ? TAG_REGISTRY.filter((tag) => tag.category === "pauses")
    : TAG_REGISTRY;
}

const lineOf = (text: string, offset: number): number => {
  let line = 1;
  for (let i = 0; i < offset && i < text.length; i++) {
    if (text[i] === "\n") line++;
  }
  return line;
};

/** Alle Befunde, aufsteigend nach Position. */
export function checkScript(
  text: string,
  speakers: SpeakerRef[],
  engine: ScriptEngine,
): ScriptFinding[] {
  const out: ScriptFinding[] = [];

  for (const candidate of scanMarkerCandidates(text)) {
    const inner = text.slice(candidate.start + 1, candidate.end - 1);
    // Wie im Backend: "a < b und b > c" ist ein Vergleich, kein Marker.
    if (/^\s/.test(inner) || /\s$/.test(inner)) continue;
    if (resolveSpeaker(candidate.name, speakers)) continue;
    out.push({
      kind: "unknown-speaker",
      start: candidate.start,
      end: candidate.end,
      raw: text.slice(candidate.start, candidate.end),
      name: candidate.name,
      style: candidate.style,
      line: lineOf(text, candidate.start),
    });
  }

  const known = new Set(
    knownTagsFor(engine).map((tag) => tag.insert.toLowerCase()),
  );
  for (const span of scanTagMatches(text)) {
    const inner = text.slice(span.start + 1, span.end - 1).trim();
    if (known.has(inner.toLowerCase())) continue;
    out.push({
      kind: "unknown-tag",
      start: span.start,
      end: span.end,
      raw: text.slice(span.start, span.end),
      name: inner,
      line: lineOf(text, span.start),
    });
  }

  out.sort((a, b) => a.start - b.start);
  return out;
}

/**
 * Ersetzt JEDES Vorkommen desselben Befunds im ganzen Text -- gleiche Art und
 * gleicher Name (Gross-/Kleinschreibung egal), also `<Bob>` UND `<Bob:leise>`
 * bei einem Sprecher-Befund. Suchen-und-Ersetzen laeuft ueber die
 * Befundliste, nicht ueber rohe Zeichenketten, damit exakt die Stellen
 * getroffen werden, die auch angezeigt sind. `replacement` liefert je
 * Stelle den fertigen Text (so bleibt der Stil erhalten) oder `` zum
 * Entfernen.
 */
export function replaceFindingEverywhere(
  text: string,
  findings: ScriptFinding[],
  target: ScriptFinding,
  replacement: (finding: ScriptFinding) => string,
): string {
  const needle = target.name.toLowerCase();
  let out = "";
  let last = 0;
  for (const finding of findings) {
    if (finding.kind !== target.kind) continue;
    if (finding.name.toLowerCase() !== needle) continue;
    out += text.slice(last, finding.start) + replacement(finding);
    last = finding.end;
  }
  out += text.slice(last);
  return out;
}
