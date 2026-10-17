/**
 * @wai/ui erscheinungsbild v1 — zustand.ts
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, erscheinungsbild/einstellungen.ts).
 * Selbsttragend: importiert nur react, @fluentui/react-components (Typen) und Nachbardateien.
 *
 * Gespeicherter Stand des Erscheinungsbilds je App (localStorage, eigener Schluessel), Pruefung beliebiger
 * Eingaben, Abgleich zwischen Fenstern ueber das `storage`-Ereignis und das wirksame Theme:
 * Schema aus `prefers-color-scheme` bei „System“, Animationen aus bei `prefers-reduced-motion`.
 */
import type { Theme } from "@fluentui/react-components";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  AKZENT_STANDARD,
  type AppDesign,
  designListe,
  istAkzentId,
  type Schema,
  STANDARD_DESIGN_ID,
} from "./designs";
import { type Kontrast, ohneBewegung, themeFuerDesign } from "./theme";

export type Modus = "system" | "hell" | "dunkel";

export interface ErscheinungsbildStand {
  modus: Modus;
  kontrast: Kontrast;
  animationen: boolean;
  designId: string;
  /** Gewaehlter Akzent je Design (Kennung aus `AKZENTE`); fehlend heisst „Wie das Design“. */
  akzent: Record<string, string>;
}

export const STANDARD_SCHLUESSEL = "wai.erscheinungsbild.v1";

export function standardStand(standardDesign = STANDARD_DESIGN_ID): ErscheinungsbildStand {
  return {
    modus: "system",
    kontrast: "standard",
    animationen: true,
    designId: standardDesign,
    akzent: {},
  };
}

const objekt = (w: unknown): Record<string, unknown> =>
  typeof w === "object" && w !== null && !Array.isArray(w) ? (w as Record<string, unknown>) : {};

/** Macht aus beliebiger Eingabe einen gueltigen Stand; Unbekanntes faellt auf den Standard zurueck. */
export function normalisiere(
  roh: unknown,
  designs: readonly AppDesign[],
  standardDesign = STANDARD_DESIGN_ID,
): ErscheinungsbildStand {
  const s = standardStand(standardDesign);
  const w = objekt(roh);
  const ids = designs.map((d) => d.id);
  const akzent: Record<string, string> = {};
  for (const [id, a] of Object.entries(objekt(w.akzent))) {
    if (ids.includes(id) && istAkzentId(a) && a !== AKZENT_STANDARD) akzent[id] = a;
  }
  return {
    modus: w.modus === "hell" || w.modus === "dunkel" || w.modus === "system" ? w.modus : s.modus,
    kontrast: w.kontrast === "hoch" || w.kontrast === "standard" ? w.kontrast : s.kontrast,
    animationen: typeof w.animationen === "boolean" ? w.animationen : s.animationen,
    designId: typeof w.designId === "string" && ids.includes(w.designId) ? w.designId : s.designId,
    akzent,
  };
}

function speicher(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined; // gesperrt (Datenschutzmodus, Sandbox)
  }
}

export function ladeStand(
  schluessel: string,
  designs: readonly AppDesign[],
  standardDesign = STANDARD_DESIGN_ID,
): ErscheinungsbildStand {
  try {
    const roh = speicher()?.getItem(schluessel);
    if (roh != null) return normalisiere(JSON.parse(roh), designs, standardDesign);
  } catch {
    /* beschaedigt oder gesperrt: Standard */
  }
  return standardStand(standardDesign);
}

export function speichereStand(schluessel: string, stand: ErscheinungsbildStand): boolean {
  try {
    const sp = speicher();
    if (!sp) return false;
    sp.setItem(schluessel, JSON.stringify(stand));
    return true;
  } catch {
    return false; // Speicher voll oder gesperrt: gilt nur fuer diese Sitzung
  }
}

function useMedienabfrage(abfrage: string): boolean {
  const abonnieren = useCallback(
    (cb: () => void) => {
      const mq = typeof window === "undefined" ? undefined : window.matchMedia?.(abfrage);
      mq?.addEventListener?.("change", cb);
      return () => mq?.removeEventListener?.("change", cb);
    },
    [abfrage],
  );
  return useSyncExternalStore(
    abonnieren,
    () => (typeof window === "undefined" ? false : (window.matchMedia?.(abfrage).matches ?? false)),
    () => false,
  );
}

export interface ErscheinungsbildOptionen {
  /** localStorage-Schluessel der App, z. B. „poststelle.erscheinungsbild.v1“. */
  speicherSchluessel?: string;
  /** Weitere Designs der App (zusaetzlich zu Microsoft und WAI; gleiche Kennung ersetzt). */
  designs?: readonly AppDesign[];
  /** Design beim ersten Start; Standard „microsoft“. */
  standardDesign?: string;
}

export interface Erscheinungsbild extends ErscheinungsbildStand {
  /** Alle waehlbaren Designs in Anzeigereihenfolge. */
  designs: readonly AppDesign[];
  design: AppDesign;
  /** Wirksames Schema (bei „System“ aus prefers-color-scheme). */
  schema: Schema;
  /** Betriebssystem verlangt reduzierte Bewegung. */
  bewegungReduziert: boolean;
  /** Animationen wirken tatsaechlich (Schalter an und keine reduzierte Bewegung). */
  animationenWirksam: boolean;
  /** Akzent des aktuellen Designs. */
  akzentAktuell: string;
  /** Wirksames Fluent-Theme fuer den FluentProvider. */
  theme: Theme;
  setzeModus: (modus: Modus) => void;
  setzeKontrast: (kontrast: Kontrast) => void;
  setzeAnimationen: (an: boolean) => void;
  setzeDesign: (designId: string) => void;
  /** Akzent fuer ein Design (Standard: das aktuelle). */
  setzeAkzent: (akzentId: string, designId?: string) => void;
  zuruecksetzen: () => void;
}

/** Zustand des Erscheinungsbilds einer App. Aenderungen wirken sofort und gelten fuer alle Fenster. */
export function useErscheinungsbild(optionen: ErscheinungsbildOptionen = {}): Erscheinungsbild {
  const schluessel = optionen.speicherSchluessel ?? STANDARD_SCHLUESSEL;
  const zusatz = optionen.designs;
  const designs = useMemo(() => designListe(zusatz), [zusatz]);
  const standardDesign =
    optionen.standardDesign && designs.some((d) => d.id === optionen.standardDesign)
      ? optionen.standardDesign
      : STANDARD_DESIGN_ID;
  const [stand, setStand] = useState(() => ladeStand(schluessel, designs, standardDesign));

  // Schlug das Speichern fehl, ist die Sitzung fuehrend: Fokus liest dann nicht nach, sonst ginge die
  // Auswahl verloren. Ein echtes storage-Ereignis (anderes Fenster hat gespeichert) gilt weiterhin.
  const sitzungFuehrend = useRef(false);

  // Andere Fenster derselben App: storage-Ereignis; beim Zurueckkehren ins Fenster zusaetzlich nachlesen.
  useEffect(() => {
    const neu = () => setStand(ladeStand(schluessel, designs, standardDesign));
    const beiSpeicher = (ev: StorageEvent) => {
      if (ev.key !== null && ev.key !== schluessel) return;
      sitzungFuehrend.current = false;
      neu();
    };
    const beiFokus = () => {
      if (!sitzungFuehrend.current) neu();
    };
    window.addEventListener("storage", beiSpeicher);
    window.addEventListener("focus", beiFokus);
    return () => {
      window.removeEventListener("storage", beiSpeicher);
      window.removeEventListener("focus", beiFokus);
    };
  }, [schluessel, designs, standardDesign]);

  const aendere = useCallback(
    (f: (s: ErscheinungsbildStand) => ErscheinungsbildStand) => {
      setStand((alt) => {
        const neu = normalisiere(f(alt), designs, standardDesign);
        sitzungFuehrend.current = !speichereStand(schluessel, neu);
        return neu;
      });
    },
    [designs, schluessel, standardDesign],
  );

  const systemDunkel = useMedienabfrage("(prefers-color-scheme: dark)");
  const bewegungReduziert = useMedienabfrage("(prefers-reduced-motion: reduce)");
  const schema: Schema =
    stand.modus === "system" ? (systemDunkel ? "dunkel" : "hell") : stand.modus;
  const design = designs.find((d) => d.id === stand.designId) ?? designs[0];
  const akzentAktuell = stand.akzent[design.id] ?? AKZENT_STANDARD;
  const animationenWirksam = stand.animationen && !bewegungReduziert;
  const theme = useMemo(() => {
    const t = themeFuerDesign(design, schema, { akzent: akzentAktuell, kontrast: stand.kontrast });
    return animationenWirksam ? t : ohneBewegung(t);
  }, [design, schema, akzentAktuell, stand.kontrast, animationenWirksam]);

  return {
    ...stand,
    designs,
    design,
    schema,
    bewegungReduziert,
    animationenWirksam,
    akzentAktuell,
    theme,
    setzeModus: (modus) => aendere((s) => ({ ...s, modus })),
    setzeKontrast: (kontrast) => aendere((s) => ({ ...s, kontrast })),
    setzeAnimationen: (animationen) => aendere((s) => ({ ...s, animationen })),
    setzeDesign: (designId) => aendere((s) => ({ ...s, designId })),
    setzeAkzent: (akzentId, designId) =>
      aendere((s) => ({ ...s, akzent: { ...s.akzent, [designId ?? s.designId]: akzentId } })),
    zuruecksetzen: () => aendere(() => standardStand(standardDesign)),
  };
}
