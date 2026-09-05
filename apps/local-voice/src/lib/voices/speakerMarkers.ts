/**
 * TS-Spiegel der Sprecher-Erkennung aus
 * `src-tauri/src/managers/tts/protocol.rs` (`scan_marker_candidates`,
 * `resolve_speaker`, `alt_speaker_marker`, `split_speaker_segments`).
 *
 * Der Editor darf NIE mehr einfärben als die Pipeline später schaltet:
 * Was hier kein Treffer ist, bleibt gewöhnlicher Text — und was hier ein
 * Chip ist, wechselt beim Vorlesen wirklich die Stimme. Deshalb sind beide
 * Seiten Zeile für Zeile dieselbe Regel:
 *
 * - `<Name>` / `<Name:Stil>` — überall in der Zeile, 1–60 Zeichen zwischen
 *   den Klammern, kein `<`, `>` oder Zeilenumbruch dazwischen; Stil wird am
 *   ERSTEN Doppelpunkt abgetrennt.
 * - `Name:` am Zeilenanfang (Alt-Format) — nur wenn `Name` eine bekannte
 *   Stimme ist. Ohne diesen Abgleich würde „Achtung: nicht vergessen" zur
 *   Sprecherzeile.
 *
 * Rust rechnet in Byte-Offsets, hier sind es UTF-16-Offsets — die Grenzen
 * liegen an denselben Zeichen, weil `<`, `>`, `:` und `\n` ASCII sind.
 */

/** Eine bekannte Stimme, so wie der Marker-Abgleich sie braucht. */
export interface SpeakerRef {
  /** voice_id — der Wert, den das Backend zum Schalten benutzt. */
  id: string;
  displayName: string;
  /** CSS-Farbe (bereits aus dem Palette-Key aufgelöst). */
  color: string;
}

export interface MarkerCandidate {
  start: number;
  end: number;
  name: string;
  style?: string;
}

/** Alle `<…>`-Kandidaten — noch OHNE Abgleich gegen bekannte Sprecher.
 *  `<div>` aus eingefügtem HTML ist hier ein Kandidat und wird erst beim
 *  Abgleich verworfen; genau so macht es die Rust-Seite. */
export function scanMarkerCandidates(text: string): MarkerCandidate[] {
  const out: MarkerCandidate[] = [];
  let i = 0;
  while (i < text.length) {
    if (text[i] === "<") {
      let closed = -1;
      for (let j = i + 1; j < text.length; j++) {
        const c = text[j];
        if (c === ">") {
          closed = j;
          break;
        }
        if (c === "<" || c === "\n") break;
      }
      if (closed !== -1) {
        const inner = text.slice(i + 1, closed);
        const charLen = [...inner].length;
        if (charLen >= 1 && charLen <= 60) {
          const colon = inner.indexOf(":");
          const name = (colon === -1 ? inner : inner.slice(0, colon)).trim();
          const rawStyle = colon === -1 ? "" : inner.slice(colon + 1).trim();
          out.push({
            start: i,
            end: closed + 1,
            name,
            // Leerer Stil nach dem Trimmen (`<Anna: >`) zählt als „kein Stil".
            style: rawStyle === "" ? undefined : rawStyle,
          });
          i = closed + 1;
          continue;
        }
      }
    }
    i++;
  }
  return out;
}

/** Namensabgleich gegen voice_id UND Anzeigename, Unicode-kleingeschrieben
 *  (`toLowerCase`, nicht ASCII-only) — „MÜLLER" trifft „müller". */
export function resolveSpeaker(
  name: string,
  speakers: SpeakerRef[],
): SpeakerRef | undefined {
  const needle = name.trim().toLowerCase();
  if (!needle) return undefined;
  return speakers.find(
    (s) =>
      s.id.toLowerCase() === needle || s.displayName.toLowerCase() === needle,
  );
}

/** Ein Fund im Text — Offsets plus das, was der Marker bedeutet. */
export interface SpeakerMarker {
  start: number;
  end: number;
  raw: string;
  speaker: SpeakerRef;
  style?: string;
  /** `true` für das Alt-Format `Name:`, `false` für `<Name>`. */
  legacy: boolean;
}

/** Alle Sprecher-Marker im Text, aufsteigend nach Position. */
export function scanSpeakerMarkers(
  text: string,
  speakers: SpeakerRef[],
): SpeakerMarker[] {
  if (speakers.length === 0) return [];
  const out: SpeakerMarker[] = [];
  let lineStart = 0;
  while (lineStart <= text.length) {
    const nl = text.indexOf("\n", lineStart);
    const lineEnd = nl === -1 ? text.length : nl;
    const line = text.slice(lineStart, lineEnd);
    let restFrom = lineStart;

    const lead = line.length - line.trimStart().length;
    const colon = line.indexOf(":", lead);
    if (colon !== -1) {
      const name = line.slice(lead, colon).trim();
      const speaker = resolveSpeaker(name, speakers);
      if (speaker) {
        out.push({
          start: lineStart + lead,
          end: lineStart + colon + 1,
          raw: line.slice(lead, colon + 1),
          speaker,
          legacy: true,
        });
        restFrom = lineStart + colon + 1;
      }
    }

    const rest = text.slice(restFrom, lineEnd);
    for (const candidate of scanMarkerCandidates(rest)) {
      const speaker = resolveSpeaker(candidate.name, speakers);
      if (!speaker) continue;
      out.push({
        start: restFrom + candidate.start,
        end: restFrom + candidate.end,
        raw: rest.slice(candidate.start, candidate.end),
        speaker,
        style: candidate.style,
        legacy: false,
      });
    }

    if (nl === -1) break;
    lineStart = nl + 1;
  }
  return out;
}

/** Der Markertext, den ein Wechsel schreibt. Immer die Spitzklammer-Form:
 *  sie funktioniert an jeder Stelle der Zeile und kann — anders als
 *  `Name:` — nicht mit gewöhnlichem Text verwechselt werden. Ein
 *  vorhandener Stil bleibt beim Wechsel erhalten. */
export const speakerMarkerText = (
  speaker: SpeakerRef,
  style?: string,
): string =>
  style ? `<${speaker.displayName}:${style}>` : `<${speaker.displayName}>`;
