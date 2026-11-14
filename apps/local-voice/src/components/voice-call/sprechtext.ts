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
 *  4. Die Grenzen der Schnittstelle gelten (`MAX_TEILE`, `MAX_TEILE_BYTES`). Sind es zu viele Stücke, werden zuerst
 *     aneinandergrenzende gleicher Art zusammengefasst; reicht das nicht oder ist der Text zu lang, geht das Häppchen ohne
 *     `teile` und ohne Ersetzung hinaus, nie stumm.
 */
import { zerlegeAussprache } from "./assistenten";
import type { AusspracheEintrag } from "./assistenten";
import type { AusspracheTeil } from "@/bindings";
import { speakable, speakableMitHerkunft } from "./globalContext";

/**
 * Obergrenzen für `teile` an `voice_call_synthesize`. Dieselben Zahlen wie `MAX_TEILE` und `MAX_TEILE_BYTES` in
 * `managers/tts/aussprache.rs`: Rust weist mehr als 4096 Stücke und mehr als 64 KiB Text (UTF-8) mit `teile` ab, bevor es am
 * Text arbeitet. Ein Test vergleicht beide Seiten. Ein Häppchen ist ein Satz oder Absatz und bleibt weit darunter.
 */
export const MAX_TEILE = 4096;
export const MAX_TEILE_BYTES = 65536;

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

/** Länge einer Zeichenkette in UTF-8-Bytes (wie `String::len` in Rust), ohne sie zu kopieren. */
export function utf8Laenge(s: string): number {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length) {
      const d = s.charCodeAt(i + 1);
      if (d >= 0xdc00 && d <= 0xdfff) {
        n += 4;
        i++;
      } else n += 3;
    } else n += 3;
  }
  return n;
}

/**
 * Fasst aneinandergrenzende Stücke gleicher Art (beide eingesetzt oder beide nicht) zu einem zusammen und lässt leere weg. Der
 * Text und die Herkunft jedes Zeichens bleiben, wie sie sind: Rust vereinigt aneinandergrenzende eingesetzte Stücke ohnehin zu
 * einem geschützten Bereich. Es bleiben nur die Stücke, an denen sich die Art wechselt.
 */
export function fasseZusammen(
  teile: readonly AusspracheTeil[],
): AusspracheTeil[] {
  const aus: AusspracheTeil[] = [];
  for (const t of teile) {
    if (t.text === "") continue;
    const letztes = aus[aus.length - 1];
    if (letztes && letztes.ersetzt === t.ersetzt) letztes.text += t.text;
    else aus.push({ text: t.text, ersetzt: t.ersetzt });
  }
  return aus;
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
  const gefiltert = speakableMitHerkunft(zerlegt);
  const teile =
    gefiltert.length > MAX_TEILE ? fasseZusammen(gefiltert) : gefiltert;
  if (teile.some((t) => hatEinzelnenSurrogat(t.text)))
    return { text: roh, teile: null };
  const text = teile.map((t) => t.text).join("");
  if (!teile.some((t) => t.ersetzt)) return { text, teile: null };
  if (teile.length > MAX_TEILE || utf8Laenge(text) > MAX_TEILE_BYTES)
    return { text: roh, teile: null };
  return { text, teile };
}
