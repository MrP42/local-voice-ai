import type { EinstufungDto, MessungDto, StufeDto } from "@/bindings";

// ADR-0028 A2: was der Sprachanruf aus der lokalen Einstufung macht. Reine Logik ohne Tauri-Aufrufe
// (Test: scripts/assistent-plan.test.mjs); der Aufruf selbst steht in `assistent.ts`.

/**
 * S0-Absichten, die nicht per Vorlage enden dürfen und wie bisher über das Modell laufen:
 * Bedienabsichten, die der Sprachanruf nicht ausführen kann (ihre Vorlage „Lauter.“ würde etwas
 * behaupten, das nicht geschieht), und „bestaetigung“, weil ein „Ja“ meist eine Frage des Modells
 * beantwortet und sonst nie beim Modell ankäme.
 */
export const OHNE_WIRKUNG: ReadonlySet<string> = new Set([
  "bestaetigung",
  "lauter",
  "leiser",
  "langsamer",
  "schneller",
]);

export type AssistentPlan =
  /** Kein Eingriff: bisheriger Ablauf (Einstufung aus, nicht erreichbar, S0 ohne Wirkung). */
  | { kind: "weiter" }
  /** S0: Vorlagenantwort sprechen/anzeigen, KEIN Modellaufruf. */
  | { kind: "antwort"; text: string; absicht: string; wiederholen: boolean }
  /** S1–S3: Ansage sofort, danach der bisherige Ablauf. */
  | { kind: "ansage"; text: string; stufe: StufeDto; absicht: string };

const WEITER: AssistentPlan = { kind: "weiter" };

export function planAssistent(
  e: EinstufungDto | null | undefined,
): AssistentPlan {
  if (!e) return WEITER;
  if (e.stufe === "S0") {
    if (!e.antwort || OHNE_WIRKUNG.has(e.absicht)) return WEITER;
    return {
      kind: "antwort",
      text: e.antwort,
      absicht: e.absicht,
      wiederholen: e.absicht === "wiederholen",
    };
  }
  if (!e.ansage) return WEITER;
  return { kind: "ansage", text: e.ansage, stufe: e.stufe, absicht: e.absicht };
}

/** Zeitmarken eines Schritts auf einer Uhr (ms, `performance.now()`-Basis). */
export interface Marken {
  start: number;
  ansage?: number;
  inhalt?: number;
  fertig?: number;
}

function relativ(start: number, t: number | undefined): number | null {
  if (t === undefined) return null;
  return Math.max(0, Math.round(t - start));
}

/** Messpunkte nach ADR-0028 E9 (nur Zeiten und Merkmale, keine Inhalte). */
export function baueMessung(
  beitragId: string,
  m: Marken,
  stufe: StufeDto,
): MessungDto {
  return {
    beitrag_id: beitragId,
    erste_ansage_ms: relativ(m.start, m.ansage),
    erster_inhalt_ms: relativ(m.start, m.inhalt),
    fertig_ms: relativ(m.start, m.fertig),
    warm: true,
    weg: "lokal",
    stufe,
  };
}
