/**
 * Der Text eines Sprechhäppchens für `voice_call_synthesize` (lva#556): mit den Ausspracheregeln des Assistenten. Reine Logik
 * ohne Tauri, damit Node-Tests sie laden können.
 *
 * Ersetzt wird pro vollständigem Häppchen (nicht pro Token des Modells), mit `zerlegeAussprache` aus dem Kern der Assistenten.
 * Der Kern der Sprachausgabe bekommt die Stücke mit (`teile`) und lässt die eingesetzten bei der Aufbereitung unangetastet
 * (kein Ausschreiben von Abkürzungen, Datum, Lexikon). Anzeige, Verlauf und Prompt behalten den Originaltext; hier entsteht nur,
 * was gesprochen wird.
 *
 * Reihenfolge:
 *  1. `speakable`: die Belegmarken `[Qk]` fallen aus dem ORIGINAL weg, bevor eine Regel den Text sieht (eine Regel darf keine
 *     Marke verändern und keine Marke eine Regel verdecken).
 *  2. `zerlegeAussprache` zerlegt den Rest in unveränderte und eingesetzte Stücke.
 *  3. Eine zweite, globale Filterung über den zusammengesetzten Text (`speakableMitHerkunft`): eine Marke, die erst durch
 *     eine Ersetzung entsteht, wird ebenfalls entfernt, und jedes überlebende Zeichen bleibt in seinem Stück (Herkunft).
 */
import { zerlegeAussprache } from "./assistenten";
import type { AusspracheEintrag } from "./assistenten";
import type { AusspracheTeil } from "@/bindings";
import { speakable, speakableMitHerkunft } from "./globalContext";

/**
 * Was an `voice_call_synthesize` geht. `teile` nur, wenn mindestens ein Stück eingesetzt ist; sonst fehlt es (alter Textpfad).
 * `AusspracheTeil` ist der aus Rust erzeugte Typ der Schnittstelle: weicht die Form von der des Assistenten-Kerns ab, fällt es
 * hier beim Übersetzen auf.
 */
export interface Sprechtext {
  text: string;
  teile: AusspracheTeil[] | null;
}

/** Hat die Zeichenkette einen einzelnen Surrogat (halbes Paar)? Ein solcher Text ginge nicht als gültiges JSON über die Schnittstelle. */
export function hatEinzelnenSurrogat(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff) {
      const n = s.charCodeAt(i + 1);
      if (!(n >= 0xdc00 && n <= 0xdfff)) return true;
      i++;
    } else if (c >= 0xdc00 && c <= 0xdfff) {
      return true;
    }
  }
  return false;
}

/**
 * Text und Stücke eines Häppchens für die Stimme. Ohne Regeln (fehlen oder leer) und ohne Treffer: genau `speakable(piece)` und
 * keine `teile`, also der Weg wie vor den Ausspracheregeln. Die Stücke ergeben zusammengesetzt immer genau `text`.
 *
 * Läge ein Stück an einer Grenze, an der ein Zeichen (Surrogatpaar) zerschnitten wäre, geht das Häppchen ohne Ersetzung an die
 * Stimme: ein Ausspracheregel-Fehler darf nie ein Häppchen stumm machen.
 */
export function sprechtext(
  piece: string,
  regeln: readonly AusspracheEintrag[] | undefined,
): Sprechtext {
  const roh = speakable(piece);
  if (!regeln || regeln.length === 0) return { text: roh, teile: null };
  const zerlegt = zerlegeAussprache(roh, regeln);
  if (!zerlegt.some((t) => t.ersetzt)) return { text: roh, teile: null };
  const teile = speakableMitHerkunft(zerlegt);
  if (teile.some((t) => hatEinzelnenSurrogat(t.text)))
    return { text: roh, teile: null };
  const text = teile.map((t) => t.text).join("");
  return { text, teile: teile.some((t) => t.ersetzt) ? teile : null };
}
