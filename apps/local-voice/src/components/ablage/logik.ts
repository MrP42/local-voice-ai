import type { AblageDatei, AblageOrdner } from "@/bindings";

/** Knoten des Ordnerbaums (nur lebende Ordner; Papierkorb steht separat). */
export interface BaumKnoten {
  ordner: AblageOrdner;
  kinder: BaumKnoten[];
  tiefe: number;
}

const nachName = (a: AblageOrdner, b: AblageOrdner) =>
  a.name.localeCompare(b.name, "de", { sensitivity: "base" });

/**
 * Baut den Baum aus den wirksamen Elternzuordnungen des Backends. Ordner mit
 * Zyklus oder verschwundenem Elternordner (`wiederhergestellt`) stehen
 * getrennt unter „Wiederhergestellt“. Ein Ordner, dessen Eltern nicht in der
 * Liste steht (z. B. im Papierkorb), erscheint nie als Wurzel.
 */
export function baumBauen(ordner: AblageOrdner[]): {
  wurzeln: BaumKnoten[];
  wiederhergestellt: BaumKnoten[];
} {
  const ids = new Set(ordner.map((o) => o.id));
  const kinderVon = new Map<string | null, AblageOrdner[]>();
  for (const o of ordner) {
    const schluessel = o.eltern && ids.has(o.eltern) ? o.eltern : null;
    if (o.eltern && !ids.has(o.eltern)) continue;
    const liste = kinderVon.get(schluessel) ?? [];
    liste.push(o);
    kinderVon.set(schluessel, liste);
  }
  const knoten = (
    o: AblageOrdner,
    tiefe: number,
    gesehen: Set<string>,
  ): BaumKnoten => {
    const folge = new Set(gesehen).add(o.id);
    return {
      ordner: o,
      tiefe,
      kinder: (kinderVon.get(o.id) ?? [])
        .filter((k) => !folge.has(k.id))
        .sort(nachName)
        .map((k) => knoten(k, tiefe + 1, folge)),
    };
  };
  const oben = (kinderVon.get(null) ?? []).sort(nachName);
  return {
    wurzeln: oben
      .filter((o) => !o.wiederhergestellt)
      .map((o) => knoten(o, 0, new Set())),
    wiederhergestellt: oben
      .filter((o) => o.wiederhergestellt)
      .map((o) => knoten(o, 0, new Set())),
  };
}

/** Pfad von der obersten Ebene bis zu `id` (leer, wenn unbekannt). */
export function pfadZu(
  ordner: AblageOrdner[],
  id: string | null,
): AblageOrdner[] {
  const je = new Map(ordner.map((o) => [o.id, o]));
  const pfad: AblageOrdner[] = [];
  let aktuell = id ? je.get(id) : undefined;
  while (aktuell && !pfad.includes(aktuell)) {
    pfad.unshift(aktuell);
    aktuell = aktuell.eltern ? je.get(aktuell.eltern) : undefined;
  }
  return pfad;
}

export function unterordner(
  ordner: AblageOrdner[],
  id: string,
): AblageOrdner[] {
  return ordner.filter((o) => o.eltern === id).sort(nachName);
}

/** Filtert nach Namen (ohne Groß-/Kleinschreibung); leer = alles. */
export function nachNameFiltern<T extends { name: string }>(
  liste: T[],
  suche: string,
): T[] {
  const q = suche.trim().toLocaleLowerCase("de");
  return q
    ? liste.filter((x) => x.name.toLocaleLowerCase("de").includes(q))
    : liste;
}

export function formatBytes(n: number, sprache: string): string {
  const zahl = (x: number, stellen: number) =>
    new Intl.NumberFormat(sprache, { maximumFractionDigits: stellen }).format(
      x,
    );
  if (n < 1024) return `${zahl(n, 0)} B`;
  if (n < 1024 * 1024) return `${zahl(n / 1024, 0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${zahl(n / (1024 * 1024), 1)} MB`;
  return `${zahl(n / (1024 * 1024 * 1024), 1)} GB`;
}

export type DateiArt =
  "pdf" | "doc" | "ppt" | "xls" | "md" | "eml" | "img" | "txt";

const ARTEN: Record<string, DateiArt> = {
  pdf: "pdf",
  doc: "doc",
  docx: "doc",
  odt: "doc",
  rtf: "doc",
  ppt: "ppt",
  pptx: "ppt",
  odp: "ppt",
  xls: "xls",
  xlsx: "xls",
  ods: "xls",
  csv: "xls",
  md: "md",
  eml: "eml",
  msg: "eml",
  jpg: "img",
  jpeg: "img",
  png: "img",
  gif: "img",
  webp: "img",
  heic: "img",
};

export function dateiArt(typ: string): DateiArt {
  return ARTEN[typ.toLowerCase()] ?? "txt";
}

/** Kürzel im Dateisymbol (`PDF`, `DOCX` …), höchstens vier Zeichen. */
export function dateiKuerzel(typ: string): string {
  return (typ || "·").slice(0, 4).toUpperCase();
}

/** Letztes Glied eines Pfads (Windows und Unix). */
export function dateiname(pfad: string): string {
  const teile = pfad.split(/[\\/]/).filter(Boolean);
  return teile[teile.length - 1] ?? pfad;
}

/** Reihenfolge der Klassen: niedrigere als die geerbte sind nicht wählbar. */
export const KLASSEN = [
  "public",
  "internal",
  "confidential",
  "personal",
] as const;
export type KlassenWert = (typeof KLASSEN)[number];

export function klasseRang(k: string): number {
  const i = KLASSEN.indexOf(k as KlassenWert);
  return i < 0 ? KLASSEN.length : i;
}

/** Klasse, die ein Ordner von oben erbt (Wirksame Klasse des Elternordners). */
export function geerbteKlasse(ordner: AblageOrdner[], id: string): string {
  const o = ordner.find((x) => x.id === id);
  const eltern = o?.eltern ? ordner.find((x) => x.id === o.eltern) : undefined;
  return eltern?.klasse ?? "internal";
}

/** Aus `<code>: <text>` des Backends: Code und Text. */
export function fehlerTeile(meldung: string): {
  code: string | null;
  text: string;
} {
  const m = /^([a-z]+(?:\.[a-z_]+)+): ([\s\S]*)$/.exec(meldung);
  return m ? { code: m[1], text: m[2] } : { code: null, text: meldung };
}

/** Eintrag der Dateiliste: Ordner oder Datei. */
export type ListenZeile =
  | { art: "ordner"; ordner: AblageOrdner }
  | { art: "datei"; datei: AblageDatei };
