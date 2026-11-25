/**
 * Such- und Sprunglogik der Auswahlfelder (lva#839), ohne React.
 *
 * - Vergleich ohne Groß/Klein; ä/ae, ö/oe, ü/ue und ß/ss gelten als gleich,
 *   andere Akzente fallen weg (é → e).
 * - Ein Treffer beginnt und endet an Grenzen von Originalzeichen: „er“ findet
 *   „Bär“ nicht (das „e“ stammt aus „ä→ae“), „a“ und „ba“ auch nicht; „bae“,
 *   „bä“ und „baer“ schon (Codex-Auflage zu lva#839).
 * - Teilwortsuche an beliebiger Stelle; Treffer am Feldanfang vor Treffern am
 *   Wortanfang vor Treffern mitten im Wort, sonst in der gegebenen Reihenfolge.
 * - Buchstabensprung wie in Windows-Listen: ein Buchstabe springt zum nächsten
 *   Eintrag mit diesem Anfang (wiederholt: weiter, am Ende wieder vorn), mehrere
 *   schnell getippte Buchstaben suchen den Anfang ab dem aktiven Eintrag. Hier
 *   zählt zusätzlich der Grundbuchstabe: „o“, „ö“ und „oe“ finden „öffentlich“.
 * - Alle Stellen sind UTF-16-Einheiten (wie `String#slice`), auch hinter Emoji.
 */

export interface AuswahlEintrag {
  label: string;
  disabled?: boolean;
}

/** Fundstelle im Originaltext: [Anfang, Ende) in UTF-16-Einheiten. */
export type Bereich = [number, number];

export interface Treffer<T> {
  eintrag: T;
  /** `null` ohne Suchtext. */
  bereich: Bereich | null;
}

const UMLAUTE: Record<string, string> = {
  ä: "ae",
  ö: "oe",
  ü: "ue",
  ß: "ss",
};
const KOMBINIERENDES_TREMA = "\u0308";
const KOMBINIEREND = /\p{M}/u;
const WORTZEICHEN = /[\p{L}\p{N}]/u;

interface Normalform {
  text: string;
  /** Je UTF-16-Einheit von `text`: Anfang und Ende des Originalzeichens. */
  von: number[];
  bis: number[];
}

function normalform(original: string): Normalform {
  let text = "";
  const von: number[] = [];
  const bis: number[] = [];
  // Je UTF-16-Einheit ein Eintrag: `text.indexOf` zählt in Einheiten, ein Emoji
  // (zwei Einheiten) darf die Abbildung nicht verschieben.
  const anhaengen = (teil: string, anfang: number, ende: number) => {
    text += teil;
    for (let k = 0; k < teil.length; k++) {
      von.push(anfang);
      bis.push(ende);
    }
  };
  let i = 0;
  for (const zeichen of original) {
    const ende = i + zeichen.length;
    const klein = zeichen.toLocaleLowerCase("de");
    if (zeichen === KOMBINIERENDES_TREMA) {
      // Zerlegter Umlaut: „u“ + Trema wird „ue“ und zählt als ein Originalzeichen.
      if (/[aou]$/.test(text)) {
        const anfang = von[von.length - 1];
        bis[bis.length - 1] = ende;
        anhaengen("e", anfang, ende);
      }
    } else if (KOMBINIEREND.test(zeichen)) {
      // andere Akzentzeichen fallen weg
    } else if (UMLAUTE[klein] !== undefined) {
      anhaengen(UMLAUTE[klein], i, ende);
    } else {
      const ohneAkzent = klein
        .normalize("NFD")
        .replace(/\p{M}/gu, "")
        .replace(/ß/g, "ss");
      anhaengen(ohneAkzent, i, ende);
    }
    i = ende;
  }
  return { text, von, bis };
}

/** Liegt Stelle `k` der Normalform auf einer Grenze zwischen Originalzeichen
 *  (nicht mitten in einer Expansion wie „ä→ae“ oder „ß→ss“)? */
function aufGrenze(form: Normalform, k: number): boolean {
  return k === 0 || k === form.text.length || form.von[k] !== form.von[k - 1];
}

/** Vergleichsform eines Texts: klein, Umlaute ausgeschrieben, ohne Akzente. */
export function normalisiere(text: string): string {
  return normalform(text).text;
}

/** Rang einer Fundstelle: 0 Feldanfang, 1 Wortanfang, 2 mitten im Wort. */
function rang(text: string, stelle: number): number {
  if (stelle === 0) return 0;
  return WORTZEICHEN.test(text[stelle - 1]) ? 2 : 1;
}

/**
 * Filtert nach Teilwort. Ohne Suchtext kommen alle Einträge unverändert zurück.
 * Je Eintrag zählt die beste Fundstelle (früher Rang, dann früheste Stelle).
 */
export function filtere<T extends AuswahlEintrag>(
  eintraege: readonly T[],
  suchtext: string,
): Treffer<T>[] {
  const muster = normalisiere(suchtext.trim());
  if (muster === "")
    return eintraege.map((eintrag) => ({ eintrag, bereich: null }));
  const gefunden: { treffer: Treffer<T>; rang: number; nr: number }[] = [];
  eintraege.forEach((eintrag, nr) => {
    const form = normalform(eintrag.label);
    let beste: { stelle: number; rang: number } | null = null;
    for (
      let stelle = form.text.indexOf(muster);
      stelle !== -1;
      stelle = form.text.indexOf(muster, stelle + 1)
    ) {
      if (!aufGrenze(form, stelle) || !aufGrenze(form, stelle + muster.length))
        continue;
      const r = rang(form.text, stelle);
      if (beste === null || r < beste.rang) beste = { stelle, rang: r };
      if (r === 0) break;
    }
    if (beste === null) return;
    const bereich: Bereich = [
      form.von[beste.stelle],
      form.bis[beste.stelle + muster.length - 1],
    ];
    gefunden.push({ treffer: { eintrag, bereich }, rang: beste.rang, nr });
  });
  gefunden.sort((a, b) => a.rang - b.rang || a.nr - b.nr);
  return gefunden.map((g) => g.treffer);
}

/** Zerlegt einen Text in Stücke mit und ohne Treffer (für `<mark>`). */
export function trefferTeile(
  text: string,
  bereich: Bereich | null,
): { text: string; treffer: boolean }[] {
  if (!bereich) return [{ text, treffer: false }];
  const [anfang, ende] = bereich;
  return [
    { text: text.slice(0, anfang), treffer: false },
    { text: text.slice(anfang, ende), treffer: true },
    { text: text.slice(ende), treffer: false },
  ].filter((teil) => teil.text !== "");
}

/**
 * Ziel des Buchstabensprungs: Index des Eintrags oder -1.
 *
 * @param getippt die seit der letzten Pause getippten Zeichen
 * @param aktiv der aktive Eintrag (-1: keiner)
 */
export function sprungZiel(
  eintraege: readonly AuswahlEintrag[],
  getippt: string,
  aktiv: number,
): number {
  const anzahl = eintraege.length;
  const zeichen = [...getippt];
  if (anzahl === 0 || zeichen.length === 0) return -1;
  const formen = eintraege.map((e) => normalform(e.label));
  const grund = eintraege.map((e) => grundform(e.label));
  // Passt der Anfang in der Normalform (an einer Zeichengrenze) oder in der
  // Grundform? So finden „o“, „ö“ und „oe“ den Eintrag „öffentlich“.
  const passt = (i: number, getippt: string) => {
    const anfang = normalisiere(getippt);
    return (
      (formen[i].text.startsWith(anfang) &&
        aufGrenze(formen[i], anfang.length)) ||
      grund[i].startsWith(grundform(getippt))
    );
  };
  const suche = (getippt: string, ab: number) => {
    for (let schritt = 0; schritt < anzahl; schritt++) {
      const i = (((ab + schritt) % anzahl) + anzahl) % anzahl;
      if (!eintraege[i].disabled && passt(i, getippt)) return i;
    }
    return -1;
  };
  const weiter = aktiv < 0 ? 0 : aktiv + 1;
  if (zeichen.length === 1) return suche(getippt, weiter);
  const ziel = suche(getippt, Math.max(aktiv, 0));
  if (ziel !== -1) return ziel;
  return zeichen.every((z) => z === zeichen[0])
    ? suche(zeichen[0], weiter)
    : -1;
}

/** Grundform für den Anfangssprung: klein, Akzente und Umlautpunkte weg (ö → o). */
function grundform(text: string): string {
  return text.toLocaleLowerCase("de").normalize("NFD").replace(/\p{M}/gu, "");
}
