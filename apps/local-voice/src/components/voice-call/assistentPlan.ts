import type { EinstufungDto, MessungDto, StufeDto, WegDto } from "@/bindings";

// ADR-0028 A2: was der Sprachanruf aus der lokalen Einstufung macht. Reine Logik ohne Tauri-Aufrufe
// (Test: scripts/assistent-plan.test.mjs); die Aufrufe selbst stehen in `assistent.ts`.
//
// Das Tor vor dem Modell steht in Rust (`assistent::tor`): Rust stuft selbst ein und verweigert das
// Modell bei S0, Fehler, leerem oder zu langem Text. Die Einstufung hier steuert nur, WAS die
// Oberfläche daraus macht (Vorlage, Ansage) und nie, OB das Modell gefragt werden darf.

export type Sprache = "de" | "en";

/** Sprache der Vorlagen aus dem Sprachcode der App (alles außer Englisch: Deutsch). */
export function sprachwahl(code: string | null | undefined): Sprache {
  return /^en([-_]|$)/i.test(code ?? "") ? "en" : "de";
}

export type VorlageName = "fehler" | "wiederhole" | "nichts-zu-wiederholen";

const VORLAGEN: Record<VorlageName, Record<Sprache, string>> = {
  fehler: {
    de: "Das kann ich gerade nicht sicher beantworten.",
    en: "I can't answer that reliably right now.",
  },
  wiederhole: {
    de: "Ich wiederhole.",
    en: "I'll repeat that.",
  },
  "nichts-zu-wiederholen": {
    de: "Ich habe noch nichts gesagt, das ich wiederholen kann.",
    en: "I haven't said anything yet that I could repeat.",
  },
};

/** Feste Vorlage; sie kommt nie vom Modell und gehört nie in den Verlauf. */
export function vorlage(name: VorlageName, sprache: Sprache): string {
  return VORLAGEN[name][sprache];
}

/** Ergebnis der Einstufung: `einstufung` ist `null`, wenn Ansagen und S0-Vorlagen aus sind. */
export type EinstufungErgebnis =
  { art: "ok"; einstufung: EinstufungDto | null } | { art: "fehler" };

/**
 * S0-Absichten ohne S0-Vorlage: „Ja“ antwortet meist auf eine Frage des Modells, Bedienabsichten kann
 * der Sprachanruf nicht ausführen (ihre Vorlage „Lauter.“ würde etwas behaupten, das nicht geschieht).
 * Das Tor in Rust lässt sie wie S1 zum Modell durch; die Antwort läuft durch das Ausgabetor.
 * Spiegel von `FORTSETZUNG` in `src-tauri/src/assistent/tor.rs`; `scripts/assistent-plan.test.mjs`
 * prüft die Gleichheit.
 */
export const FORTSETZUNG: ReadonlySet<string> = new Set([
  "bestaetigung",
  "lauter",
  "leiser",
  "langsamer",
  "schneller",
]);

export type AssistentPlan =
  /** Kein Eingriff: Modell (Ansagen und Vorlagen aus, oder Fortsetzung). Das Tor in Rust gilt trotzdem. */
  | { kind: "weiter" }
  /** S0: Vorlagenantwort sprechen/anzeigen, KEIN Modellaufruf. */
  | { kind: "antwort"; text: string; absicht: string }
  /** S0 „wiederholen“: nur die letzte vom Tor freigegebene Ausgabe, nicht ins Modell, nicht in den Verlauf. */
  | { kind: "wiederholen" }
  /** Einstufung fehlgeschlagen oder S0 ohne Vorlage und ohne Fortsetzung: feste Meldung, nie ans Modell. */
  | { kind: "fehler"; text: string }
  /** S1–S3: Ansage sofort, danach das Modell. */
  | { kind: "ansage"; text: string; stufe: StufeDto; absicht: string };

export function planAssistent(
  r: EinstufungErgebnis,
  sprache: Sprache,
): AssistentPlan {
  const fehler: AssistentPlan = {
    kind: "fehler",
    text: vorlage("fehler", sprache),
  };
  if (r.art === "fehler") return fehler;
  const e = r.einstufung;
  if (!e) return { kind: "weiter" };
  if (e.stufe === "S0") {
    if (e.absicht === "wiederholen") return { kind: "wiederholen" };
    if (FORTSETZUNG.has(e.absicht)) return { kind: "weiter" };
    if (!e.antwort) return fehler;
    return { kind: "antwort", text: e.antwort, absicht: e.absicht };
  }
  if (!e.ansage) return { kind: "weiter" };
  return { kind: "ansage", text: e.ansage, stufe: e.stufe, absicht: e.absicht };
}

/** Stufe der Einstufung, falls es eine gibt (für die Messung). */
export function stufeVon(r: EinstufungErgebnis): StufeDto | null {
  return r.art === "ok" && r.einstufung ? r.einstufung.stufe : null;
}

/** Zeitmarken eines Schritts auf einer Uhr (ms, `performance.now()`-Basis). */
export interface Marken {
  start: number;
  ansage?: number;
  inhalt?: number;
  fertig?: number;
}

/** Merkmale der Messung, die die Engine kennt: Modell schon warm, Weg der Antwort. */
export interface MessArt {
  warm: boolean;
  weg: WegDto;
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
  art: MessArt,
): MessungDto {
  return {
    beitrag_id: beitragId,
    erste_ansage_ms: relativ(m.start, m.ansage),
    erster_inhalt_ms: relativ(m.start, m.inhalt),
    fertig_ms: relativ(m.start, m.fertig),
    warm: art.warm,
    weg: art.weg,
    stufe,
  };
}
