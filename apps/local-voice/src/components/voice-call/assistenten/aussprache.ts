/**
 * Ausspracheregeln eines Assistenten-Profils anwenden (lva#556): wörtliche Ersetzung nur für die Sprachausgabe. Die Regeln
 * stehen in `apps/shared/assistenten/README.md` („Aussprache“), die gemeinsamen Fälle in `aussprache-pruefstaende.json`
 * (Abschnitt `ersetzung`); dieselben Fälle liest der Rust-Kern.
 *
 * Reine Logik ohne Abhängigkeiten. Verglichen wird genau (Folge der Unicode-Skalare, keine Normalisierung, Groß- und
 * Kleinschreibung zählt), gesucht wird ohne Muster, Platzhalter oder Wortgrenzen.
 */
import type { AusspracheEintrag } from "./profil";

/** Ein Stück des Ergebnisses: unveränderter Text oder die Aussprache eines Treffers. */
export type AusspracheTeil = { text: string; ersetzt: boolean };

/**
 * Zerlegt `text` von links nach rechts. An jeder Stelle gilt der Eintrag mit dem längsten `wort`, das dort beginnt; bei
 * gleicher Länge der frühere in der Liste. Der Treffer wird ein eigenes Stück (`ersetzt: true`) mit der `aussprache`, danach
 * geht es **hinter** dem Treffer weiter: Was eine Regel eingesetzt hat, wird nicht noch einmal durchsucht, es gibt also keine
 * Ketten und keine Schleifen. Text ohne Treffer bleibt unverändert und wird zu einem Stück zusammengefasst.
 *
 * Die Stücke sind für den Aufrufer da, der danach ein weiteres Lexikon anwendet: Es darf nur die Stücke mit `ersetzt: false`
 * anfassen (die Regel des Profils hat Vorrang). Ein Eintrag mit leerem `wort` wird übergangen.
 */
export function zerlegeAussprache(
  text: string,
  eintraege: readonly AusspracheEintrag[],
): AusspracheTeil[] {
  // Nach der ersten Codeeinheit gruppiert, je Gruppe längstes `wort` zuerst; die stabile Sortierung hält bei gleicher
  // Länge die Reihenfolge der Liste. Ein längeres `wort`, das an derselben Stelle passt, ist in jeder Zählweise länger
  // (das kürzere ist sein Anfang), darum genügt hier die Länge in Codeeinheiten.
  const nachAnfang = new Map<number, AusspracheEintrag[]>();
  for (const eintrag of eintraege) {
    if (eintrag.wort === "") continue;
    const anfang = eintrag.wort.charCodeAt(0);
    const gruppe = nachAnfang.get(anfang);
    if (gruppe) gruppe.push(eintrag);
    else nachAnfang.set(anfang, [eintrag]);
  }
  for (const gruppe of nachAnfang.values())
    gruppe.sort((a, b) => b.wort.length - a.wort.length);

  const teile: AusspracheTeil[] = [];
  let offen = 0;
  let stelle = 0;
  while (stelle < text.length) {
    const treffer = nachAnfang
      .get(text.charCodeAt(stelle))
      ?.find((e) => text.startsWith(e.wort, stelle));
    if (!treffer) {
      stelle += 1;
      continue;
    }
    if (offen < stelle)
      teile.push({ text: text.slice(offen, stelle), ersetzt: false });
    teile.push({ text: treffer.aussprache, ersetzt: true });
    stelle += treffer.wort.length;
    offen = stelle;
  }
  if (offen < text.length)
    teile.push({ text: text.slice(offen), ersetzt: false });
  return teile;
}

/**
 * Der Text für die Sprachausgabe: `text` mit den Ausspracheregeln des Profils. Nur dafür gedacht; Anzeige, Verlauf, Prompt
 * und Kontext behalten den Originaltext. Ohne Regeln (`undefined` oder leer) kommt `text` unverändert zurück.
 */
export function wendeAusspracheAn(
  text: string,
  eintraege: readonly AusspracheEintrag[] | undefined,
): string {
  if (!eintraege || eintraege.length === 0) return text;
  let aus = "";
  for (const teil of zerlegeAussprache(text, eintraege)) aus += teil.text;
  return aus;
}
