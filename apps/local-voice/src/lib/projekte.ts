// Suite-Projekte und Recherche (ADR-0026 PR-7, Epic wai-sdk#1047): reine Abbildungen zwischen den
// Antworten des Backends (bindings.ts) und den Bausteinen aus @wai/ui (src/vendor/wai-ui/projekt).
// Ohne React und ohne Tauri, damit `node --experimental-strip-types --test` sie pruefen kann.
import type {
  EintragSicht,
  FreigabeInfo,
  RechercheSicht,
  SuiteProjekt,
} from "../bindings";
import type {
  Datenklasse,
  EintragArt,
  FreigabeAnfrage,
  ProjektEintrag,
  ProjektKurz,
  QuellenArt,
  RechercheErgebnis,
} from "../vendor/wai-ui/projekt";

const KLASSEN: readonly Datenklasse[] = [
  "PUBLIC",
  "INTERNAL",
  "PERSONAL",
  "SPECIAL",
  "RESTRICTED",
];

export const alsKlasse = (s: string | null | undefined): Datenklasse =>
  KLASSEN.find((k) => k === s) ?? "PERSONAL";

/** Ergebnis des Backends auspacken; ein Fehler wird zum Text (kein Inhalt der Frage darin). */
export function auspacken<T>(
  r: { status: "ok"; data: T } | { status: "error"; error: string },
): T {
  if (r.status === "ok") return r.data;
  throw new Error(r.error);
}

export const fehlerText = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

export function projektKurz(p: SuiteProjekt): ProjektKurz {
  return {
    id: p.id,
    title: p.title,
    dataClass: alsKlasse(p.dataClass),
    specialCategory: p.specialCategory,
    archived: p.archived,
    webResearchEnabled: p.webResearchEnabled,
    entryCount: p.entryCount,
  };
}

const RECHERCHE_PRAEFIX = "wai://lva/recherche/";

/** Art fuer die Anzeige: Recherchen liegen als `document` mit `wai://lva/recherche/<id>`. */
export function eintragArt(e: Pick<EintragSicht, "kind" | "link">): EintragArt {
  if (e.link?.startsWith(RECHERCHE_PRAEFIX)) return "research";
  switch (e.kind) {
    case "meeting":
    case "transcript":
    case "conversation":
    case "book":
    case "book_document":
    case "note":
    case "research":
    case "document":
    case "plan":
    case "talk":
      return e.kind;
    default:
      return "document";
  }
}

/** "30.10." aus einem RFC-3339-Zeitpunkt; unlesbare Werte ergeben `undefined`. */
export function datumKurz(iso: string | null | undefined): string | undefined {
  if (!iso) return undefined;
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return m ? `${m[3]}.${m[2]}.` : undefined;
}

export function projektEintrag(e: EintragSicht): ProjektEintrag {
  return {
    id: e.id,
    kind: eintragArt(e),
    title: e.title ?? undefined,
    subtitle: e.subtitle ?? undefined,
    originApp: e.originApp ?? "lva",
    dataClass: alsKlasse(e.dataClass),
    available: e.available,
    dateLabel: datumKurz(e.assignedAt),
  };
}

/** Kennung der Besprechung oder des Buches aus dem Verweis `wai://lva/<art>/<id>`. */
export function verweisZiel(
  link: string | null | undefined,
): { art: "besprechung" | "buch" | "recherche"; id: string } | null {
  const m =
    /^wai:\/\/lva\/(besprechung|buch|recherche)\/([A-Za-z0-9_-]+)$/.exec(
      link ?? "",
    );
  return m
    ? { art: m[1] as "besprechung" | "buch" | "recherche", id: m[2] }
    : null;
}

const QUELLENART: Record<string, QuellenArt> = {
  amtlich: "official",
  wissenschaftlich: "scholarly",
  Presse: "press",
  Forum: "forum",
  unbekannt: "unknown",
};

export function hostVon(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function zeitKurz(iso: string): string {
  const m = /T(\d{2}:\d{2})/.exec(iso);
  return m ? m[1] : "";
}

export function rechercheErgebnis(d: RechercheSicht): RechercheErgebnis {
  const quellen = d.sources.length;
  return {
    title: d.question,
    answer: d.answer.map((a) => ({
      text: a.text,
      citations: a.citations,
      unsupported: a.unsupported,
    })),
    sources: d.sources.map((q) => ({
      index: q.index,
      title: q.title,
      host: hostVon(q.url),
      sourceType: QUELLENART[q.sourceType] ?? "unknown",
      retrievedLabel: `abgerufen ${zeitKurz(q.retrievedAt)}`.trim(),
      excerpt: q.excerpt,
    })),
    contradictions: d.contradictions,
    followUps: d.followUps,
    provenance: `Schnell · ${quellen === 1 ? "1 Quelle" : `${quellen} Quellen`} · ${
      d.stoppedReason === "budget"
        ? "Budget erreicht"
        : d.stoppedReason === "cancelled"
          ? "abgebrochen"
          : "geprüft"
    } · ${d.model}`,
  };
}

/** Angezeigter Kontext des Freigabedialogs: die Frage der Recherche und die Klasse in Worten. */
export function freigabeKontext(info: FreigabeInfo): string {
  return `Recherche „${info.question}“ (${info.dataClassLabel})`;
}

export function freigabeAnfragen(info: FreigabeInfo): FreigabeAnfrage[] {
  return info.queries.map((q) => ({
    id: q.id,
    text: q.text,
    hint: q.hint ?? undefined,
    // Ab PERSONAL ist nichts vorab gewaehlt, wenn ein Hinweis dabei steht; die Person entscheidet.
    selected: false,
  }));
}
