export interface Vorschlag {
  art: { regel: string } | "sprache" | "glaetten";
  alt: string;
  neu: string;
  gruppe: string | null;
  status: "offen" | "angenommen" | "abgelehnt";
}

export interface Segment {
  art: "gleich" | "aenderung";
  text: string;
  alt?: string;
  neu?: string;
  gruppe?: string | null;
  status?: Vorschlag["status"];
}

type Bereich = readonly [number, number];

/** Ungültige Einträge dürfen weder Text verschlucken noch Bereiche verschieben. */
function gueltigeBereiche(
  text: string,
  aenderungen: readonly Vorschlag[],
  bereiche: readonly Bereich[],
) {
  let ende = 0;
  const result: { aenderung: Vorschlag; start: number; ende: number }[] = [];
  aenderungen.forEach((aenderung, index) => {
    const bereich = bereiche[index];
    if (!bereich) return;
    const [start, bis] = bereich;
    if (
      !Number.isInteger(start) ||
      !Number.isInteger(bis) ||
      start < ende ||
      bis < start ||
      bis > text.length ||
      start > 2 ** 31 ||
      bis > 2 ** 31 ||
      text.slice(start, bis) !== aenderung.alt
    ) {
      return;
    }
    result.push({ aenderung, start, ende: bis });
    ende = bis;
  });
  return result;
}

export function gruppenUebersicht(
  aenderungen: readonly Vorschlag[],
  basisText: string,
  utf16Bereiche: readonly Bereich[],
) {
  const regeln = new Map<
    string,
    {
      regel: string;
      anzahl: number;
      gruppen: Set<string>;
      status: Vorschlag["status"] | "gemischt";
      beispiel: string;
    }
  >();
  const gueltig = gueltigeBereiche(basisText, aenderungen, utf16Bereiche);
  for (const aenderung of aenderungen) {
    if (typeof aenderung.art !== "object") continue;
    const regel = aenderung.art.regel;
    let eintrag = regeln.get(regel);
    if (!eintrag) {
      const bereich = gueltig.find((b) => b.aenderung === aenderung);
      const links = bereich
        ? basisText.slice(Math.max(0, bereich.start - 24), bereich.start)
        : "";
      const rechts = bereich
        ? basisText.slice(bereich.ende, bereich.ende + 24)
        : "";
      eintrag = {
        regel,
        anzahl: 0,
        gruppen: new Set<string>(),
        status: aenderung.status,
        beispiel: `${links}${aenderung.alt} → ${aenderung.neu}${rechts}`
          .replace(/[\r\n]/g, " "),
      };
      regeln.set(regel, eintrag);
    }
    eintrag.anzahl += 1;
    if (aenderung.gruppe !== null) eintrag.gruppen.add(aenderung.gruppe);
    if (eintrag.status !== aenderung.status) eintrag.status = "gemischt";
  }
  return [...regeln.values()].map((eintrag) => ({
    ...eintrag,
    gruppen: eintrag.gruppen.size,
  }));
}

export function vergleichSegmente(
  basisText: string,
  aenderungen: readonly Vorschlag[],
  utf16Bereiche: readonly Bereich[],
): Segment[] {
  const segmente: Segment[] = [];
  let ende = 0;
  for (const b of gueltigeBereiche(basisText, aenderungen, utf16Bereiche)) {
    if (b.start > ende) {
      segmente.push({ art: "gleich", text: basisText.slice(ende, b.start) });
    }
    segmente.push({
      art: "aenderung",
      text: b.aenderung.alt,
      alt: b.aenderung.alt,
      neu: b.aenderung.neu,
      gruppe: b.aenderung.gruppe,
      status: b.aenderung.status,
    });
    ende = b.ende;
  }
  if (ende < basisText.length) {
    segmente.push({ art: "gleich", text: basisText.slice(ende) });
  }
  return segmente;
}

export function ergebnisText(
  basisText: string,
  aenderungen: readonly Vorschlag[],
  utf16Bereiche: readonly Bereich[],
  modus: "angenommen" | "alle",
): string {
  return vergleichSegmente(basisText, aenderungen, utf16Bereiche)
    .map((s) =>
      s.art === "aenderung" &&
      (modus === "alle" || s.status === "angenommen")
        ? s.neu ?? ""
        : s.text,
    )
    .join("");
}

export function strukturVergleich(vorher: string, nachher: string) {
  const zaehlen = (text: string) => ({
    absaetze: text.split(/\r?\n\s*\r?\n/).filter((s) => s.trim()).length,
    ueberschriften: (text.match(/^\s*#+[^\r\n]*/gm) ?? []).length,
    seitenmarken: (text.match(/\{\{S\.\s*\d+\}\}/g) ?? []).length,
  });
  const a = zaehlen(vorher);
  const b = zaehlen(nachher);
  return {
    absaetze: { vorher: a.absaetze, nachher: b.absaetze },
    ueberschriften: { vorher: a.ueberschriften, nachher: b.ueberschriften },
    seitenmarken: { vorher: a.seitenmarken, nachher: b.seitenmarken },
  };
}

/** Seite ist nullbasiert; überlappender Kontext wird nur einmal geliefert. */
export function fenster(
  segmente: readonly Segment[],
  seite: number,
  proSeite: number,
): Segment[] {
  if (
    !Number.isInteger(seite) ||
    seite < 0 ||
    !Number.isInteger(proSeite) ||
    proSeite < 1
  ) {
    return [];
  }
  const start = seite * proSeite;
  let nummer = 0;
  const indices = new Set<number>();
  segmente.forEach((segment, index) => {
    if (segment.art !== "aenderung") return;
    if (nummer >= start && nummer < start + proSeite) {
      for (
        let i = Math.max(0, index - 1);
        i <= Math.min(segmente.length - 1, index + 1);
        i += 1
      ) {
        indices.add(i);
      }
    }
    nummer += 1;
  });
  return [...indices].sort((a, b) => a - b).map((i) => segmente[i]);
}
