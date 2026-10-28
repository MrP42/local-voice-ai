import {
  resolveSpeaker,
  scanMarkerCandidates,
  speakerMarkerText,
  type SpeakerRef,
} from "./speakerMarkers";

/** Die Teile eines Seitenstands, in denen Sprecher stehen koennen. */
export interface SpeakerFields {
  text: string;
  summary: string;
  /** Stimme je Reiter (Werte wie im Stimmen-Dropdown). */
  voices: Record<string, string>;
}

export interface SpeakerReplacement {
  fields: SpeakerFields;
  /** Ersetzte `<Name>`-Marker in Text und Zusammenfassung. */
  markers: number;
  /** Umgestellte gespeicherte Reiter-Stimmen. */
  voices: number;
}

/** Jeden Marker, der `from` meint, auf `to` umhaengen; der Stil jeder Stelle bleibt. */
export function replaceSpeakerInText(
  text: string,
  from: SpeakerRef,
  to: SpeakerRef,
  speakers: SpeakerRef[],
): { text: string; count: number } {
  const hits = scanMarkerCandidates(text).filter(
    (candidate) => resolveSpeaker(candidate.name, speakers)?.id === from.id,
  );
  let out = text;
  for (const candidate of [...hits].reverse()) {
    out =
      out.slice(0, candidate.start) +
      speakerMarkerText(to, candidate.style) +
      out.slice(candidate.end);
  }
  return { text: out, count: hits.length };
}

/**
 * "Stimme X ueberall durch Y ersetzen" fuer eine Seite: Marker in Text und
 * Zusammenfassung sowie die gemerkten Reiter-Stimmen (`fromValue` -> `toValue`).
 */
export function replaceSpeakerInFields(
  fields: SpeakerFields,
  from: SpeakerRef,
  to: SpeakerRef,
  speakers: SpeakerRef[],
  fromValue: string,
  toValue: string,
): SpeakerReplacement {
  const text = replaceSpeakerInText(fields.text, from, to, speakers);
  const summary = replaceSpeakerInText(fields.summary, from, to, speakers);
  let voices = 0;
  const nextVoices = Object.fromEntries(
    Object.entries(fields.voices).map(([key, value]) => {
      if (value !== fromValue) return [key, value];
      voices += 1;
      return [key, toValue];
    }),
  );
  return {
    fields: { text: text.text, summary: summary.text, voices: nextVoices },
    markers: text.count + summary.count,
    voices,
  };
}

/** Felder aus dem JSON-Seitenstand (`state.json`); fehlerhafte Staende gelten als leer. */
export function fieldsOfState(json: string | null | undefined): {
  state: Record<string, unknown>;
  fields: SpeakerFields;
} | null {
  if (!json) return null;
  try {
    const state = JSON.parse(json) as Record<string, unknown>;
    if (!state || typeof state !== "object") return null;
    const voices = state.voices;
    return {
      state,
      fields: {
        text: typeof state.text === "string" ? state.text : "",
        summary: typeof state.summary === "string" ? state.summary : "",
        voices:
          voices && typeof voices === "object"
            ? (voices as Record<string, string>)
            : {},
      },
    };
  } catch {
    return null;
  }
}
