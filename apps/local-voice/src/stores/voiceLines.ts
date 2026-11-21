import type { ContextHit } from "@/components/voice-call/globalContext";
import type { Denkprozess, VoiceLine } from "@/stores/voiceCallStore";

// Reine Zeilenlogik des Gesprächsspeichers (ohne Zustand und ohne Oberfläche), damit sie prüfbar ist.

/**
 * Hängt die geprüften Belege einer Antwort an ihre Zeile. Gewöhnlich ist das die erste Antwortzeile nach der
 * Frage; das Ergebnis eines Auftrags (`nachgereicht`) steht in der Zeile, die seine Kennung trägt
 * (`auftragVon`), nie in einer Zeile, die nur zufällig die letzte ist; tragen mehrere Zeilen die Kennung, gilt die
 * jüngste. `null`: nichts zu ändern.
 */
export function mitQuellen(
  lines: VoiceLine[],
  turnId: string,
  hits: ContextHit[],
  nachgereicht = false,
): VoiceLine[] | null {
  if (hits.length === 0) return null;
  const i = nachgereicht
    ? zeileDesAuftrags(lines, turnId)
    : zeileNachFrage(lines, turnId);
  if (i < 0) return null;
  const out = [...lines];
  out[i] = {
    ...out[i],
    sources: hits.map((h) => ({
      k: h.k,
      kind: h.kind,
      title: h.title,
      page: h.jump.page,
      jump: h.jump,
    })),
  };
  return out;
}

/** Index der jüngsten Zeile, die das Ergebnis dieses Auftrags trägt (`-1`: keine): der gültige Versuch zählt. */
function zeileDesAuftrags(lines: VoiceLine[], turnId: string): number {
  for (let i = lines.length - 1; i >= 0; i--)
    if (lines[i].role === "assistant" && lines[i].auftragVon === turnId)
      return i;
  return -1;
}

/** Index der ersten Antwortzeile nach der Frage mit dieser Kennung (`-1`: keine). */
function zeileNachFrage(lines: VoiceLine[], turnId: string): number {
  const at = lines.findIndex((l) => l.role === "user" && l.turnId === turnId);
  if (at < 0) return -1;
  return lines.findIndex((l, j) => j > at && l.role === "assistant");
}

/** Die Teile des Zustands, in denen ein Denkprozess angezeigt wird (laufend, im Hintergrund, an einer Antwort). */
export interface DenkAnzeigen {
  denken: Denkprozess | null;
  auftrag: Denkprozess | null;
  lines: VoiceLine[];
}

/**
 * Klappt den Denkprozess auf oder zu, wo immer er gerade angezeigt wird: als laufende Anzeige, als Anzeige eines
 * offenen Auftrags im Hintergrund oder an einer Antwortzeile. `seit` kennzeichnet den Prozess.
 */
export function denkOffenSetzen(
  s: DenkAnzeigen,
  seit: number,
  offen: boolean,
): DenkAnzeigen {
  return {
    denken: s.denken?.seit === seit ? { ...s.denken, offen } : s.denken,
    auftrag: s.auftrag?.seit === seit ? { ...s.auftrag, offen } : s.auftrag,
    lines: s.lines.some((l) => l.denken?.seit === seit)
      ? s.lines.map((l) =>
          l.denken?.seit === seit
            ? { ...l, denken: { ...l.denken, offen } }
            : l,
        )
      : s.lines,
  };
}
