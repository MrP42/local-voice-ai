/**
 * „Text aufbereiten“ in einem Schritt (lva#783).
 *
 * Die Bausteine gibt es schon einzeln, versteckt im Editor-Menü:
 * - Regeln ohne KI (`textCleanup.ts`): Zeitstempel, abgerissene Zeilen,
 *   Leerzeichen, Absätze – kostet kein Token.
 * - KI-Aufbereitung (`tts_tidy_text`): Seitenzahlen, Kopf-/Fußzeilen,
 *   Trennungen und harte Umbrüche raus, nichts wird gekürzt.
 * - KI-Korrektur (`tts_text_assist` „grammar“): Rechtschreibung und
 *   Grammatik, nur geänderte Zeilen kommen zurück.
 *
 * Hier laufen sie in fester Reihenfolge hintereinander: erst die Regeln (das
 * Modell sieht weniger Müll), dann die KI-Aufbereitung, zuletzt die Korrektur
 * auf dem fertigen Fließtext. Die Aufrufe der App kommen als `deps` herein,
 * damit sich die Reihenfolge ohne Tauri prüfen lässt.
 */
import { CLEANUP_RULES, applyCleanup } from "./textCleanup";
import { applyGrammar, buildAssistInput } from "./textAssist";

export type PrepStep = "rules" | "layout" | "spelling";

/** Reihenfolge der Schritte; die Auswahl ändert sie nicht. */
export const PREP_STEPS: readonly PrepStep[] = ["rules", "layout", "spelling"];

export interface PrepDeps {
  /** KI-Aufbereitung des ganzen Texts (`commands.ttsTidyText`). */
  tidy(text: string): Promise<string>;
  /** KI-Korrektur, Antwort im Zeilenformat (`commands.ttsTextAssist`). */
  grammar(prompt: string): Promise<string>;
  /** Fortschritt: gerade begonnener Schritt. */
  onStep?(step: PrepStep): void;
}

export interface PrepResult {
  text: string;
  /** Schritte, die den Text geändert haben. */
  changed: PrepStep[];
  /** Geänderte Stellen der Regeln und Korrektur (die KI-Aufbereitung zählt
   *  nicht in Stellen, sie ersetzt den Text als Ganzes). */
  changes: number;
}

export async function prepareText(
  text: string,
  steps: readonly PrepStep[],
  deps: PrepDeps,
): Promise<PrepResult> {
  let current = text;
  const changed: PrepStep[] = [];
  let changes = 0;
  for (const step of PREP_STEPS) {
    if (!steps.includes(step) || !current.trim()) continue;
    deps.onStep?.(step);
    if (step === "rules") {
      const result = applyCleanup(current, CLEANUP_RULES);
      if (result.text !== current) {
        current = result.text;
        changes += result.changes;
        changed.push(step);
      }
    } else if (step === "layout") {
      const next = (await deps.tidy(current)).trim();
      if (next && next !== current.trim()) {
        current = next;
        changed.push(step);
      }
    } else {
      const input = buildAssistInput(current, 0, 0);
      if (input.sent.length === 0) continue;
      const applied = applyGrammar(
        current,
        input,
        await deps.grammar(input.prompt),
      );
      if (applied.text !== current) {
        current = applied.text;
        changes += applied.changes;
        changed.push(step);
      }
    }
  }
  return { text: current, changed, changes };
}
