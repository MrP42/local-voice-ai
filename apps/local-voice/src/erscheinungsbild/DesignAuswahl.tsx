/**
 * @wai/ui erscheinungsbild v1.1 — DesignAuswahl.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, einstellungen/bausteine.tsx Kartenauswahl).
 * Selbsttragend: importiert nur react, @fluentui/react-components und Nachbardateien.
 *
 * Kartenauswahl der Designs: je Karte die Mini-Oberflaeche hell und dunkel, darunter Name und
 * Beschreibung, rechts Radiokreis mit Haken. Radiogruppe mit Pfeiltasten; zwei Spalten, schmal eine.
 */
import { makeStyles, mergeClasses, tokens } from "@fluentui/react-components";
import { useId } from "react";
import { DesignVorschau } from "./DesignVorschau";
import type { AppDesign } from "./designs";
import { Haken, useRadiogruppe } from "./radiogruppe";

const useStyles = makeStyles({
  gitter: {
    display: "grid",
    gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    gap: tokens.spacingHorizontalM,
    "@media (max-width: 640px)": { gridTemplateColumns: "minmax(0, 1fr)" },
  },
  karte: {
    display: "grid",
    gap: tokens.spacingVerticalS,
    padding: tokens.spacingHorizontalS,
    borderRadius: tokens.borderRadiusLarge,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    backgroundColor: tokens.colorNeutralBackground1,
    color: tokens.colorNeutralForeground1,
    cursor: "pointer",
    textAlign: "start",
    outlineStyle: "none",
    transitionProperty: "background-color, border-color, box-shadow",
    transitionDuration: tokens.durationFaster,
    ":hover": { backgroundColor: tokens.colorNeutralBackground1Hover },
    ":active": { backgroundColor: tokens.colorNeutralBackground1Pressed },
    ":focus-visible": {
      outline: `${tokens.strokeWidthThick} solid ${tokens.colorStrokeFocus2}`,
      outlineOffset: "2px",
    },
  },
  gesperrt: {
    cursor: "not-allowed",
    opacity: 0.55,
    ":hover": { backgroundColor: tokens.colorNeutralBackground1 },
    ":active": { backgroundColor: tokens.colorNeutralBackground1 },
  },
  gewaehlt: {
    border: `${tokens.strokeWidthThin} solid ${tokens.colorCompoundBrandStroke}`,
    boxShadow: `inset 0 0 0 1px ${tokens.colorCompoundBrandStroke}`,
  },
  fuss: {
    display: "flex",
    alignItems: "flex-start",
    gap: tokens.spacingHorizontalS,
    padding: `0 ${tokens.spacingHorizontalXS} ${tokens.spacingVerticalXS}`,
  },
  text: { display: "grid", gap: "2px", flex: "1 1 auto", minWidth: 0 },
  name: {
    fontSize: tokens.fontSizeBase300,
    lineHeight: tokens.lineHeightBase300,
    fontWeight: tokens.fontWeightSemibold,
  },
  beschreibung: {
    fontSize: tokens.fontSizeBase200,
    lineHeight: tokens.lineHeightBase200,
    color: tokens.colorNeutralForeground3,
  },
  kreis: {
    flex: "none",
    display: "grid",
    placeItems: "center",
    width: "20px",
    height: "20px",
    marginTop: "1px",
    borderRadius: tokens.borderRadiusCircular,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStrokeAccessible}`,
  },
  kreisAn: {
    border: `${tokens.strokeWidthThin} solid ${tokens.colorCompoundBrandBackground}`,
    backgroundColor: tokens.colorCompoundBrandBackground,
    color: tokens.colorNeutralForegroundOnBrand,
  },
});

export interface DesignAuswahlProps {
  designs: readonly AppDesign[];
  wert: string;
  beiWechsel: (designId: string) => void;
  /** Akzent je Design fuer die Vorschau (Kennung aus `AKZENTE`). */
  akzent?: Readonly<Record<string, string>>;
  /** Zugaenglicher Name der Gruppe. */
  label?: string;
  /** Alternativ: Id eines sichtbaren Titels. */
  labelledBy?: string;
  /** Id eines Hinweises zur Gruppe (z. B. Grund der Sperre). */
  describedBy?: string;
  /** Gesperrt: Karten nicht waehlbar und nicht fokussierbar (aria-disabled). */
  gesperrt?: boolean;
  /** Uebersetzte Namen/Beschreibungen je Design-Kennung; fehlende Werte kommen aus dem Design. */
  designTexte?: DesignTexte;
  className?: string;
}

/** Texte je Design-Kennung, z. B. `{ microsoft: { name: "Fluent", beschreibung: "…" } }`. */
export type DesignTexte = Readonly<
  Partial<Record<string, { name?: string; beschreibung?: string }>>
>;

/** Name und Beschreibung eines Designs mit Uebersetzung. */
export function designText(d: AppDesign, texte: DesignTexte = {}) {
  return {
    name: texte[d.id]?.name ?? d.name,
    beschreibung: texte[d.id]?.beschreibung ?? d.beschreibung,
  };
}

export function DesignAuswahl({
  designs,
  wert,
  beiWechsel,
  akzent = {},
  label = "Design",
  labelledBy,
  describedBy,
  gesperrt = false,
  designTexte,
  className,
}: DesignAuswahlProps) {
  const s = useStyles();
  const basis = useId();
  const radio = useRadiogruppe(
    designs.map((d) => d.id),
    wert,
    beiWechsel,
    gesperrt,
  );
  return (
    <div
      role="radiogroup"
      aria-label={labelledBy ? undefined : label}
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      aria-disabled={gesperrt || undefined}
      className={mergeClasses(s.gitter, className)}
    >
      {designs.map((d) => {
        const an = d.id === wert;
        const nameId = `${basis}-${d.id}-n`;
        const textId = `${basis}-${d.id}-b`;
        const r = radio(d.id);
        const text = designText(d, designTexte);
        return (
          // biome-ignore lint/a11y/useSemanticElements: Karten-Radio mit Vorschau; natives input kann keinen Karteninhalt tragen
          <div
            key={d.id}
            {...r}
            role="radio"
            aria-checked={r["aria-checked"]}
            tabIndex={r.tabIndex}
            aria-disabled={r["aria-disabled"]}
            aria-labelledby={nameId}
            aria-describedby={textId}
            data-design-karte={d.id}
            className={mergeClasses(s.karte, an && s.gewaehlt, gesperrt && s.gesperrt)}
          >
            <DesignVorschau design={d} akzent={akzent[d.id]} />
            <span className={s.fuss}>
              <span className={s.text}>
                <span id={nameId} className={s.name}>
                  {text.name}
                </span>
                <span id={textId} className={s.beschreibung}>
                  {text.beschreibung}
                </span>
              </span>
              <span className={mergeClasses(s.kreis, an && s.kreisAn)} aria-hidden="true">
                {an ? <Haken /> : null}
              </span>
            </span>
          </div>
        );
      })}
    </div>
  );
}
