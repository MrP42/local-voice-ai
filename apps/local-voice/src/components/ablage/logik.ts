import type {
  AblageAnbieter,
  AblageCloud,
  AblageDatei,
  AblageOrdner,
  AblageUnterordner,
} from "@/bindings";

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

/**
 * Anker des Markenformats v1 (`seite=3`, `absatz=12`, `folie=2`, `zeile=5`,
 * `blatt=<name>&zeilen=2-21`, `teil=kopf|text|<n>`) in Art und Werte zerlegt.
 * Unbekanntes bleibt `art: "roh"`.
 */
export type AnkerTeile =
  | { art: "seite" | "absatz" | "folie" | "zeile" | "teil"; n: number }
  | { art: "kopf" | "text" }
  | { art: "blatt"; blatt: string; von: number; bis: number }
  | { art: "roh"; text: string };

const prozentDekodieren = (s: string) => {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
};

export function ankerTeile(anker: string): AnkerTeile {
  const einfach = /^(seite|absatz|folie|zeile)=(\d+)$/.exec(anker);
  if (einfach)
    return {
      art: einfach[1] as "seite" | "absatz" | "folie" | "zeile",
      n: Number(einfach[2]),
    };
  const teil = /^teil=(kopf|text|\d+)$/.exec(anker);
  if (teil) {
    if (teil[1] === "kopf" || teil[1] === "text") return { art: teil[1] };
    return { art: "teil", n: Number(teil[1]) };
  }
  const blatt = /^blatt=(.*)&zeilen=(\d+)-(\d+)$/.exec(anker);
  if (blatt)
    return {
      art: "blatt",
      blatt: prozentDekodieren(blatt[1]),
      von: Number(blatt[2]),
      bis: Number(blatt[3]),
    };
  return { art: "roh", text: anker };
}

/** Suchwörter für die Hervorhebung (mindestens zwei Zeichen, ohne Dubletten). */
export function suchwoerter(anfrage: string): string[] {
  const w = anfrage
    .toLocaleLowerCase("de")
    .split(/[^\p{L}\p{N}]+/u)
    .filter((x) => x.length >= 2);
  return [...new Set(w)];
}

/** Zerlegt `text` in Stücke; `treffer` markiert die Vorkommen der Suchwörter. */
export function hervorheben(
  text: string,
  woerter: string[],
): { text: string; treffer: boolean }[] {
  if (woerter.length === 0) return [{ text, treffer: false }];
  const muster = new RegExp(
    `(${woerter.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
    "giu",
  );
  return text
    .split(muster)
    .filter((s) => s.length > 0)
    .map((s) => ({
      text: s,
      treffer: woerter.includes(s.toLocaleLowerCase("de")),
    }));
}

// ---------------------------------------------------------------------------
// D7-Rest: Unterordnerbaum verknüpfter Ordner (nur lesen)
// ---------------------------------------------------------------------------

/** Trenner zwischen Kennung des verknüpften Ordners und Pfad im Ansichts-Schlüssel. */
const UNTER_TRENNER = "|";

/** Ansichts-Schlüssel: die Kennung allein (Wurzel) oder `<Kennung>|<Pfad>` (Unterordner). */
export function unterAnsicht(id: string, pfad: string): string {
  return pfad ? `${id}${UNTER_TRENNER}${pfad}` : id;
}

export function ansichtZerlegen(ansicht: string | null): {
  ordner: string | null;
  pfad: string;
} {
  if (ansicht === null) return { ordner: null, pfad: "" };
  const i = ansicht.indexOf(UNTER_TRENNER);
  return i < 0
    ? { ordner: ansicht, pfad: "" }
    : { ordner: ansicht.slice(0, i), pfad: ansicht.slice(i + 1) };
}

/** Knoten im Unterordnerbaum eines verknüpften Ordners. */
export interface UnterKnoten {
  /** Relativer Pfad mit `/`. */
  pfad: string;
  name: string;
  tiefe: number;
  /** Der Ordner oder ein Vorfahr liegt nur online: wird nie betreten, nicht aufklappbar. */
  nurOnline: boolean;
  /** Dateien darunter (rekursiv); bei „nur online“ nur die zuletzt bekannten. */
  dateien: number;
  kinder: UnterKnoten[];
}

/** Letztes Glied eines relativen Pfads: `A/B/c.txt` → `c.txt`. */
export const letztesGlied = (pfad: string) =>
  pfad.slice(pfad.lastIndexOf("/") + 1);
/** Pfad ohne das letzte Glied: `A/B/c.txt` → `A/B`; oberste Ebene → leer. */
export const elternPfad = (pfad: string) =>
  pfad.includes("/") ? pfad.slice(0, pfad.lastIndexOf("/")) : "";

/** Alle Vorfahren eines Pfads, von oben (`A/B/C` → `A`, `A/B`). */
export function vorfahrenPfade(pfad: string): string[] {
  const teile = pfad.split("/").filter(Boolean);
  return teile.slice(0, -1).map((_, i) => teile.slice(0, i + 1).join("/"));
}

/**
 * Baut den Baum aus den vom Backend gemeldeten Unterordnern und den Pfaden der Dateien (die Eltern jeder
 * Datei kommen dazu: ein Bestand ohne Ordnerliste zeigt so wenigstens die Ordner mit Dateien). Je Ebene
 * nach Namen sortiert.
 */
export function unterbaumBauen(
  unterordner: AblageUnterordner[],
  dateiNamen: string[],
): UnterKnoten[] {
  const online = new Map<string, boolean>();
  for (const u of unterordner) online.set(u.pfad, u.nur_online);
  const pfade = new Set<string>(online.keys());
  const dateienJe = new Map<string, number>();
  for (const name of dateiNamen) {
    for (const v of vorfahrenPfade(name)) {
      pfade.add(v);
      dateienJe.set(v, (dateienJe.get(v) ?? 0) + 1);
    }
  }
  for (const p of [...pfade]) for (const v of vorfahrenPfade(p)) pfade.add(v);
  const kinderVon = new Map<string, string[]>();
  for (const p of pfade) {
    const e = elternPfad(p);
    const liste = kinderVon.get(e) ?? [];
    liste.push(p);
    kinderVon.set(e, liste);
  }
  const knoten = (p: string, tiefe: number, vonOben: boolean): UnterKnoten => {
    const nurOnline = vonOben || online.get(p) === true;
    return {
      pfad: p,
      name: letztesGlied(p),
      tiefe,
      nurOnline,
      dateien: dateienJe.get(p) ?? 0,
      kinder: (kinderVon.get(p) ?? [])
        .sort((a, b) =>
          letztesGlied(a).localeCompare(letztesGlied(b), "de", {
            sensitivity: "base",
          }),
        )
        .map((k) => knoten(k, tiefe + 1, nurOnline)),
    };
  };
  return (kinderVon.get("") ?? [])
    .sort((a, b) => a.localeCompare(b, "de", { sensitivity: "base" }))
    .map((p) => knoten(p, 0, false));
}

/** Knoten zu einem Pfad (leer = Wurzel, dafür gibt es keinen Knoten). */
export function knotenZu(
  baum: UnterKnoten[],
  pfad: string,
): UnterKnoten | null {
  if (!pfad) return null;
  for (const k of baum) {
    if (k.pfad === pfad) return k;
    if (pfad.startsWith(`${k.pfad}/`)) {
      const treffer = knotenZu(k.kinder, pfad);
      if (treffer) return treffer;
    }
  }
  return null;
}

/** Dateien, die direkt in `pfad` liegen (leer = oberste Ebene); `name` ist der relative Pfad. */
export function dateienImPfad<T extends { name: string }>(
  dateien: T[],
  pfad: string,
): T[] {
  return dateien.filter((d) => elternPfad(d.name) === pfad);
}

// ---------------------------------------------------------------------------
// D7-Rest: Cloud-Ausnahme im Ordnerdialog
// ---------------------------------------------------------------------------

/** Wirksame Klasse eines Ordners nach der Auswahl im Dialog (wie im Backend: nie unter der geerbten). */
export function wirksameKlasseNeu(
  ordner: AblageOrdner[],
  id: string,
  gewaehlt: string,
): string {
  const o = ordner.find((x) => x.id === id);
  const eltern = o?.eltern ? ordner.find((x) => x.id === o.eltern) : undefined;
  const geerbt = eltern?.klasse ?? "internal";
  if (gewaehlt === "erbt") return geerbt;
  return klasseRang(gewaehlt) >= klasseRang(geerbt) ? gewaehlt : geerbt;
}

/** Was die Bestätigung der Cloud-Ausnahme zählt: Dateien mit wirksamer Klasse „Vertraulich“ im Unterbaum. */
export interface VertraulichZaehlung {
  dateien: number;
  /** Unterordner (ohne den Ordner selbst). */
  unterordner: number;
  /** Oberste Unterordner mit „Personenbezogen“: ausgenommen, gehen nie an einen Cloud-Anbieter. */
  personenbezogen: string[];
}

export function vertraulicheImUnterbaum(
  ordner: AblageOrdner[],
  dateien: AblageDatei[],
  id: string,
): VertraulichZaehlung {
  const je = new Map(ordner.map((o) => [o.id, o]));
  if (!je.has(id)) return { dateien: 0, unterordner: 0, personenbezogen: [] };
  const kinderVon = new Map<string, AblageOrdner[]>();
  for (const o of ordner) {
    if (!o.eltern) continue;
    const liste = kinderVon.get(o.eltern) ?? [];
    liste.push(o);
    kinderVon.set(o.eltern, liste);
  }
  const im = new Set<string>([id]);
  const personenbezogen: string[] = [];
  const stapel = [id];
  while (stapel.length > 0) {
    const aktuell = stapel.pop() as string;
    for (const k of kinderVon.get(aktuell) ?? []) {
      if (im.has(k.id)) continue;
      im.add(k.id);
      stapel.push(k.id);
      if (k.klasse === "personal" && je.get(aktuell)?.klasse !== "personal")
        personenbezogen.push(k.name);
    }
  }
  personenbezogen.sort((a, b) =>
    a.localeCompare(b, "de", { sensitivity: "base" }),
  );
  return {
    dateien: dateien.filter(
      (d) => im.has(d.ordner) && d.klasse === "confidential",
    ).length,
    unterordner: im.size - 1,
    personenbezogen,
  };
}

/** Welche Anbieter beim Speichern neu erteilt und welche entzogen werden (sortiert). */
export function cloudAenderung(
  bestand: string[],
  auswahl: string[],
): { erteilen: string[]; entziehen: string[] } {
  const alt = new Set(bestand);
  const neu = new Set(auswahl);
  return {
    erteilen: [...neu].filter((a) => !alt.has(a)).sort(),
    entziehen: [...alt].filter((a) => !neu.has(a)).sort(),
  };
}

/** Eigene und geerbte Cloud-Ausnahmen eines Eintrags, ohne Doppelte, sortiert (fehlt die Angabe: keine). */
export function cloudAlle(c: AblageCloud | null | undefined): string[] {
  if (!c) return [];
  return [...new Set([...(c.anbieter ?? []), ...(c.geerbt ?? [])])].sort();
}

/**
 * Zu jedem Anbieter den Namen des nächsten Ordners (der Ordner selbst oder ein übergeordneter), an dem die
 * Ausnahme hängt. Anbieter ohne Quelle fehlen in der Antwort. Zyklen brechen ab.
 */
export function cloudQuellen(
  ordner: AblageOrdner[],
  startId: string,
  ids: string[],
): Map<string, string> {
  const je = new Map(ordner.map((o) => [o.id, o]));
  const offen = new Set(ids);
  const quellen = new Map<string, string>();
  const gesehen = new Set<string>();
  let aktuell = je.get(startId);
  while (aktuell && offen.size > 0 && !gesehen.has(aktuell.id)) {
    gesehen.add(aktuell.id);
    for (const a of aktuell.cloud?.anbieter ?? []) {
      if (offen.delete(a)) quellen.set(a, aktuell.name);
    }
    aktuell = aktuell.eltern ? je.get(aktuell.eltern) : undefined;
  }
  return quellen;
}

/** Namen der Anbieter für die Anzeige: aus der Liste, sonst die Kennung; sortiert, ohne Doppelte. */
export function anbieterNamen(
  ids: string[],
  liste: AblageAnbieter[],
): string[] {
  const name = new Map(liste.map((a) => [a.id, a.name]));
  return [...new Set(ids.map((id) => name.get(id) ?? id))].sort((a, b) =>
    a.localeCompare(b, "de", { sensitivity: "base" }),
  );
}
