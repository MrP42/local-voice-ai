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
import { TAG_REGISTRY, searchTags } from "@/lib/tags/registry";
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

// ---- Gruppen und Empfehlungen ------------------------------------------

/** Alle Stellen desselben Befunds (gleiche Art, gleicher Name). */
export interface FindingGroup {
  key: string;
  kind: ScriptFindingKind;
  name: string;
  findings: ScriptFinding[];
}

/**
 * Bündelt Befunde je (Art, Name), in der Reihenfolge des ersten Vorkommens.
 * Siebzehn Zeilen "[calm]" sind EIN Problem mit siebzehn Stellen — und die
 * Massnahme gilt fuer alle zugleich.
 */
export function groupFindings(findings: ScriptFinding[]): FindingGroup[] {
  const groups = new Map<string, FindingGroup>();
  for (const finding of findings) {
    const key = `${finding.kind}:${finding.name.toLowerCase()}`;
    const group = groups.get(key);
    if (group) group.findings.push(finding);
    else
      groups.set(key, {
        key,
        kind: finding.kind,
        name: finding.name,
        findings: [finding],
      });
  }
  return [...groups.values()];
}

const fold = (s: string): string =>
  s
    .toLowerCase()
    .replace(/ä/g, "ae")
    .replace(/ö/g, "oe")
    .replace(/ü/g, "ue")
    .replace(/ß/g, "ss")
    .replace(/[^a-z0-9]/g, "");

/** Levenshtein-Distanz — klein genug, um sie hier zu halten. */
const editDistance = (a: string, b: string): number => {
  const prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    let last = prev[0];
    prev[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const tmp = prev[j];
      prev[j] = Math.min(
        prev[j] + 1,
        prev[j - 1] + 1,
        last + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
      last = tmp;
    }
  }
  return prev[b.length];
};

/**
 * Sprecher, die dem unbekannten Namen aehneln — Tippfehler, Umlaut-Varianten
 * ("Erzähler" vs. "Erzaehler"), Teilstrings. Hoechstens `max`, beste zuerst;
 * leer, wenn nichts naeher als zwei Aenderungen liegt.
 */
export function suggestSpeakers(
  name: string,
  speakers: SpeakerRef[],
  max = 3,
): SpeakerRef[] {
  const q = fold(name);
  if (!q) return [];
  const scored = speakers
    .map((speaker) => {
      const candidates = [fold(speaker.displayName), fold(speaker.id)];
      let score = Infinity;
      for (const c of candidates) {
        if (!c) continue;
        if (c === q) score = Math.min(score, 0);
        else if (c.startsWith(q) || q.startsWith(c)) score = Math.min(score, 1);
        else if (c.includes(q) || q.includes(c)) score = Math.min(score, 2);
        else {
          const d = editDistance(c, q);
          if (d <= Math.max(2, Math.floor(q.length / 4)))
            score = Math.min(score, 2 + d);
        }
      }
      return { speaker, score };
    })
    .filter((entry) => entry.score !== Infinity)
    .sort((a, b) => a.score - b.score);
  return scored.slice(0, max).map((entry) => entry.speaker);
}

/**
 * Tags, die zum unbekannten Tag passen: Synonyme aus der Registry
 * ("calm" ist ein Alias von "relaxed"), Label-Treffer in beiden Sprachen,
 * sonst Tippfehler-Naehe. Dokumentierte Tags zuerst — die wirken
 * verlaesslich, Freitext liest das Modell womoeglich vor.
 */
export function suggestTags(
  name: string,
  engine: ScriptEngine,
  uiLang: string,
  max = 3,
): TagDef[] {
  const known = knownTagsFor(engine);
  const allowed = new Set(known.map((tag) => tag.id));
  const q = fold(name);
  const bySearch = searchTags(name, uiLang).filter((tag) =>
    allowed.has(tag.id),
  );
  const byDistance = known.filter((tag) => {
    const c = fold(tag.insert);
    return c && editDistance(c, q) <= Math.max(1, Math.floor(q.length / 4));
  });
  const merged: TagDef[] = [];
  for (const tag of [...bySearch, ...byDistance]) {
    if (!merged.includes(tag)) merged.push(tag);
  }
  merged.sort(
    (a, b) => Number(b.verified === true) - Number(a.verified === true),
  );
  return merged.slice(0, max);
}
