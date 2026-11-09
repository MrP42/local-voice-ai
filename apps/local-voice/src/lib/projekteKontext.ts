// Projektseite mit „Als Kontext“ (lva#365 P4b): reine Abbildungen zwischen den Antworten des
// Backends (EintragSicht, KontextUebersicht) und der Oberfläche. Ohne React und ohne Tauri.
import type {
  EintragSicht,
  KontextUebersicht,
  KontextZeile,
} from "../bindings";

export type Gruppe =
  "buecher" | "texte" | "aufnahmen" | "dateien" | "recherchen";

export const GRUPPEN: readonly Gruppe[] = [
  "buecher",
  "texte",
  "aufnahmen",
  "dateien",
  "recherchen",
];

const BUCH = "wai://lva/buch/";
const SEITE = "wai://lva/seite/";
const BESPRECHUNG = "wai://lva/besprechung/";
const RECHERCHE = "wai://lva/recherche/";
const DATEI = /^wai:\/\/lva\/projektdatei\/[A-Za-z0-9_-]+\/([A-Za-z0-9_.-]+)$/;

const nach = (
  link: string | null | undefined,
  prefix: string,
): string | null =>
  link?.startsWith(prefix) ? link.slice(prefix.length) || null : null;

export const buchIdVon = (link: string | null | undefined) => nach(link, BUCH);
export const seiteIdVon = (link: string | null | undefined) =>
  nach(link, SEITE);
export const besprechungIdVon = (link: string | null | undefined) =>
  nach(link, BESPRECHUNG);
export const rechercheIdVon = (link: string | null | undefined) =>
  nach(link, RECHERCHE);
export const dateiIdVon = (link: string | null | undefined): string | null =>
  DATEI.exec(link ?? "")?.[1] ?? null;

/** In welche Gruppe der Projektseite ein Eintrag gehört. */
export function gruppeVon(e: Pick<EintragSicht, "kind" | "link">): Gruppe {
  if (dateiIdVon(e.link)) return "dateien";
  if (rechercheIdVon(e.link) || e.kind === "research") return "recherchen";
  if (buchIdVon(e.link) || e.kind === "book") return "buecher";
  if (
    besprechungIdVon(e.link) ||
    e.kind === "meeting" ||
    e.kind === "transcript"
  )
    return "aufnahmen";
  return "texte";
}

/** Eintraege je Gruppe, in der Reihenfolge der Seite; leere Gruppen fehlen. */
export function gruppiere(
  eintraege: readonly EintragSicht[],
): { gruppe: Gruppe; eintraege: EintragSicht[] }[] {
  const je = new Map<Gruppe, EintragSicht[]>();
  for (const e of eintraege) {
    const g = gruppeVon(e);
    je.set(g, [...(je.get(g) ?? []), e]);
  }
  return GRUPPEN.filter((g) => je.has(g)).map((g) => ({
    gruppe: g,
    eintraege: je.get(g) ?? [],
  }));
}

/** Zahl mit geschütztem Leerzeichen als Tausendertrenner (`182 000`). */
export const zahl = (n: number): string =>
  Math.round(n)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, " ");

/** Hinweis-Codes, bei denen der Schalter gesperrt ist: der Eintrag kann nie Kontext sein. */
const GESPERRT = new Set([
  "kontext.format",
  "kontext.nicht_lesbar",
  "kontext.fehlt",
]);
export const schalterGesperrt = (z: KontextZeile | undefined): boolean =>
  !!z?.hint && GESPERRT.has(z.hint);

/** Schlüsselteil eines Hinweis-Codes (`kontext.kein_kurzbuch` -> `kein_kurzbuch`). */
export const hinweisSchluessel = (code: string): string =>
  code.replace(/^kontext\./, "");

export type FehltArt =
  | "ohneKurzbuch"
  | "ohneProtokoll"
  | "formatNichtLesbar"
  | "ausgeschaltet"
  | "ohneText";

/** „Nicht im Kontext“: wie viele Einträge aus welchem Grund nichts beitragen. */
export function nichtImKontext(
  uebersicht: KontextUebersicht | null,
): { art: FehltArt; count: number }[] {
  const zaehler = new Map<FehltArt, number>();
  const plus = (a: FehltArt) => zaehler.set(a, (zaehler.get(a) ?? 0) + 1);
  for (const z of uebersicht?.rows ?? []) {
    if (z.hint === "kontext.kein_kurzbuch") plus("ohneKurzbuch");
    else if (z.hint === "kontext.kein_protokoll") plus("ohneProtokoll");
    else if (z.hint === "kontext.format") plus("formatNichtLesbar");
    // Ablage-Eintrag: keine festen Inhalte, Auszüge je Frage (Spec Ablage §5.3).
    else if (z.hint === "kontext.auszuege") {
      if (!z.on) plus("ausgeschaltet");
    } else if (z.hint) plus("ohneText");
    else if (!z.on) plus("ausgeschaltet");
  }
  const ordnung: FehltArt[] = [
    "ohneKurzbuch",
    "ohneProtokoll",
    "formatNichtLesbar",
    "ohneText",
    "ausgeschaltet",
  ];
  return ordnung
    .filter((a) => zaehler.has(a))
    .map((a) => ({ art: a, count: zaehler.get(a) ?? 0 }));
}

/** Größe in Worten für Chips („3 KB“). */
export const groesse = (bytes: number): string =>
  bytes < 1024
    ? `${bytes} B`
    : bytes < 1024 * 1024
      ? `${Math.max(1, Math.round(bytes / 1024))} KB`
      : `${(bytes / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;

/** Fenster des Modells, das in diesem Projekt antworten würde (Rückfall 128 000). */
export const STANDARD_FENSTER = 128_000;
