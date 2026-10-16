/**
 * Sprecher einer Übersetzung gegen Stimmen der Zielsprache tauschen.
 *
 * Die Übersetzung behält die Sprechermarken des Originals (`<Erzählerin>`):
 * das Backend schützt sie vor dem Modell. Gelesen werden sollen sie aber von
 * Stimmen der Zielsprache — eine deutsche Erzählerin liest kein Englisch.
 * Hier steht die reine Logik: welche Sprecher der Text hat, welche Stimme der
 * Zielsprache jeder bekommt, und wie die Marken im Text ersetzt werden.
 */
import {
  resolveSpeaker,
  scanMarkerCandidates,
  type SpeakerRef,
} from "./speakerMarkers";

/** Eine Stimme mit Sprache (aus `ttsListVoiceInfos`). */
export interface LangVoice extends SpeakerRef {
  /** BCP-47, z. B. `en-US`; leer, wenn unbekannt. */
  language: string;
}

/** Die bekannten Sprecher eines Textes in der Reihenfolge ihres Auftretens. */
export function speakersInText(text: string, voices: SpeakerRef[]): SpeakerRef[] {
  const seen = new Set<string>();
  const out: SpeakerRef[] = [];
  for (const candidate of scanMarkerCandidates(text)) {
    const voice = resolveSpeaker(candidate.name, voices);
    if (voice && !seen.has(voice.id)) {
      seen.add(voice.id);
      out.push(voice);
    }
  }
  return out;
}

/** Spricht die Stimme die Sprache mit diesem Kürzel (`en`, `de`)? */
export const speaksLanguage = (voice: LangVoice, code: string): boolean =>
  voice.language.toLowerCase().split(/[-_]/)[0] === code.toLowerCase();

/**
 * Vorschlag: jeder Sprecher bekommt eine Stimme der Zielsprache, verschiedene
 * Sprecher verschiedene Stimmen, solange es genug gibt; eine Stimme, die schon
 * die Zielsprache spricht, bleibt. Ohne Stimme der Zielsprache bleibt der
 * Sprecher, wie er ist. Ergebnis: Sprecher-id → Stimmen-id.
 */
export function suggestSwap(
  sources: SpeakerRef[],
  voices: LangVoice[],
  code: string,
): Record<string, string> {
  const target = voices.filter((v) => speaksLanguage(v, code));
  const used = new Set<string>();
  const mapping: Record<string, string> = {};
  for (const source of sources) {
    const self = target.find((v) => v.id === source.id);
    if (self) {
      mapping[source.id] = self.id;
      used.add(self.id);
    }
  }
  let next = 0;
  for (const source of sources) {
    if (mapping[source.id] || target.length === 0) continue;
    const free = target.find((v) => !used.has(v.id));
    const pick = free ?? target[next++ % target.length];
    mapping[source.id] = pick.id;
    used.add(pick.id);
  }
  for (const source of sources) mapping[source.id] ??= source.id;
  return mapping;
}

/** Marken ersetzen: `<Alt>` → `<Neu>`, ein Stil (`<Alt:ruhig>`) bleibt. */
export function applySwap(
  text: string,
  mapping: Record<string, string>,
  voices: SpeakerRef[],
): string {
  const byId = new Map(voices.map((v) => [v.id, v]));
  let out = "";
  let last = 0;
  for (const candidate of scanMarkerCandidates(text)) {
    const from = resolveSpeaker(candidate.name, voices);
    const to = from ? byId.get(mapping[from.id] ?? "") : undefined;
    if (!from || !to || to.id === from.id) continue;
    out += text.slice(last, candidate.start);
    out += candidate.style
      ? `<${to.displayName}:${candidate.style}>`
      : `<${to.displayName}>`;
    last = candidate.end;
  }
  return out + text.slice(last);
}
