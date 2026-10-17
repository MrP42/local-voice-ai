/**
 * @wai/ui erscheinungsbild v1.1 — AkzentAuswahl.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, einstellungen/bausteine.tsx Farbfelder).
 * Selbsttragend: importiert nur react, @fluentui/react-components und Nachbardateien.
 *
 * Runde Farbfelder wie in den Windows-11-Einstellungen (Personalisierung > Farben). Jedes Feld zeigt die
 * Farbe so, wie sie im aktuellen Design und Schema wirkt (bereits auf 4,5:1 angepasst). Der gewaehlte
 * Name steht zusaetzlich als Text darunter — der Zustand haengt nie nur an der Farbe.
 */
import { makeStyles, mergeClasses, tokens } from "@fluentui/react-components";
import { AKZENT_STANDARD, AKZENTE, type AkzentDef, type AppDesign, type Schema } from "./designs";
import { textAuf } from "./farbe";
import { Haken, useRadiogruppe } from "./radiogruppe";
import { wirksameMarke } from "./theme";

const useStyles = makeStyles({
  felder: { display: "flex", flexWrap: "wrap", gap: tokens.spacingHorizontalS },
  feld: {
    display: "grid",
    placeItems: "center",
    width: "32px",
    height: "32px",
    borderRadius: tokens.borderRadiusCircular,
    cursor: "pointer",
    outlineStyle: "none",
    boxShadow: `inset 0 0 0 1px rgba(0, 0, 0, 0.12)`,
    ":hover": { transform: "scale(1.06)" },
    ":focus-visible": {
      outline: `${tokens.strokeWidthThick} solid ${tokens.colorStrokeFocus2}`,
      outlineOffset: "2px",
    },
  },
  an: {
    boxShadow: `0 0 0 2px ${tokens.colorNeutralBackground1}, 0 0 0 4px ${tokens.colorNeutralForeground1}`,
  },
  name: {
    margin: `${tokens.spacingVerticalS} 0 0`,
    fontSize: tokens.fontSizeBase200,
    lineHeight: tokens.lineHeightBase200,
    color: tokens.colorNeutralForeground3,
  },
});

/** Farbe eines Akzents, wie sie im Design und Schema erscheint. */
export function akzentVorschau(design: AppDesign, akzentId: string, schema: Schema): string {
  return wirksameMarke(design, schema, akzentId === AKZENT_STANDARD ? undefined : akzentId);
}

export interface AkzentAuswahlProps {
  design: AppDesign;
  schema: Schema;
  wert: string;
  beiWechsel: (akzentId: string) => void;
  akzente?: readonly AkzentDef[];
  label?: string;
  /** Vorsatz vor dem gewaehlten Namen. */
  gewaehltText?: string;
  /** Gesperrt (Kontrast hoch, Design-Vorgabe): nicht waehlbar und nicht fokussierbar. */
  deaktiviert?: boolean;
  /** Uebersetzte Akzentnamen je Kennung; fehlende kommen aus `akzente`. */
  akzentNamen?: Readonly<Partial<Record<string, string>>>;
  className?: string;
}

export function AkzentAuswahl({
  design,
  schema,
  wert,
  beiWechsel,
  akzente = AKZENTE,
  label = "Akzentfarbe",
  gewaehltText = "Gewählt:",
  deaktiviert = false,
  akzentNamen = {},
  className,
}: AkzentAuswahlProps) {
  const s = useStyles();
  const radio = useRadiogruppe(
    akzente.map((a) => a.id),
    wert,
    beiWechsel,
    deaktiviert,
  );
  const name = (a: AkzentDef) => akzentNamen[a.id] ?? a.name;
  const gewaehlt = akzente.find((a) => a.id === wert) ?? akzente[0];
  return (
    <div className={className}>
      <div
        role="radiogroup"
        aria-label={label}
        aria-disabled={deaktiviert || undefined}
        className={s.felder}
      >
        {akzente.map((a) => {
          const farbe = akzentVorschau(design, a.id, schema);
          const an = a.id === gewaehlt.id;
          const r = radio(a.id);
          return (
            // biome-ignore lint/a11y/useSemanticElements: Farbfeld-Radio mit eigener Tastaturfuehrung (useRadiogruppe)
            <span
              key={a.id}
              {...r}
              role="radio"
              aria-checked={r["aria-checked"]}
              tabIndex={r.tabIndex}
              aria-disabled={r["aria-disabled"]}
              aria-label={name(a)}
              title={name(a)}
              data-akzent={a.id}
              className={mergeClasses(s.feld, an && s.an)}
              style={{ background: farbe, color: textAuf(farbe) }}
            >
              {an ? <Haken groesse={16} /> : null}
            </span>
          );
        })}
      </div>
      <p className={s.name} aria-live="polite">
        {gewaehltText} {name(gewaehlt)}
      </p>
    </div>
  );
}
