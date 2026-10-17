/**
 * @wai/ui erscheinungsbild v1 — DesignVorschau.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, einstellungen/Vorschau.tsx).
 * Selbsttragend: importiert nur react, @fluentui/react-components und Nachbardateien.
 *
 * Miniatur einer App-Oberflaeche, gezeichnet aus den Tokens des Themes (kein Bild): Titelleiste,
 * Seitenleiste mit Auswahl-Pill, Ueberschrift, Karte mit Balkendiagramm, primaerer Knopf und Link.
 * Fuer Hilfstechnik ausgeblendet; Name und Beschreibung stehen als Text an der Karte.
 */
import { makeStyles, mergeClasses, type Theme } from "@fluentui/react-components";
import { useMemo } from "react";
import type { AppDesign } from "./designs";
import { themeFuerDesign } from "./theme";

const useStyles = makeStyles({
  paar: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: "6px",
  },
  mini: {
    display: "grid",
    gridTemplateColumns: "28% 1fr",
    gridTemplateRows: "12px 1fr",
    aspectRatio: "16 / 10",
    overflow: "hidden",
    borderRadius: "4px",
    boxShadow: "0 0 0 1px rgba(0, 0, 0, 0.12)",
    fontSize: 0,
    lineHeight: 0,
  },
  titel: {
    gridColumn: "1 / span 2",
    display: "flex",
    alignItems: "center",
    gap: "4px",
    padding: "0 6px",
  },
  logo: { width: "6px", height: "6px", borderRadius: "1px" },
  titeltext: { width: "28%", height: "3px", borderRadius: "1px" },
  kaption: { marginLeft: "auto", display: "flex", gap: "4px" },
  kpunkt: { width: "5px", height: "1px" },
  nav: {
    display: "grid",
    alignContent: "start",
    gap: "5px",
    padding: "6px 5px",
  },
  navzeile: { height: "3px", borderRadius: "1px", opacity: 0.6 },
  navaktiv: {
    position: "relative",
    height: "8px",
    borderRadius: "2px",
  },
  pill: {
    position: "absolute",
    left: "0",
    top: "2px",
    width: "2px",
    height: "4px",
    borderRadius: "1px",
  },
  haupt: {
    display: "grid",
    alignContent: "start",
    gap: "4px",
    padding: "6px 7px",
    minWidth: 0,
  },
  h: { height: "5px", width: "55%", borderRadius: "1px" },
  text: { height: "2px", width: "80%", borderRadius: "1px" },
  karte: {
    display: "flex",
    alignItems: "flex-end",
    gap: "4px",
    height: "24px",
    padding: "4px 5px",
  },
  balken: { width: "6px", borderRadius: "1px 1px 0 0" },
  aktionen: { display: "flex", alignItems: "center", gap: "6px", marginTop: "1px" },
  knopf: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: "26px",
    height: "9px",
  },
  knopftext: { width: "12px", height: "2px", borderRadius: "1px" },
  link: { width: "18px", height: "2px", borderRadius: "1px" },
});

const halb = (r: string) => (r.endsWith("px") ? `${Number.parseFloat(r) / 2}px` : r);

/** Eine Oberflaeche in Miniatur, gezeichnet mit den Farben von `theme`. */
export function MiniOberflaeche({ theme }: { theme: Theme }) {
  const s = useStyles();
  const t = theme;
  return (
    <span className={s.mini} style={{ background: t.colorNeutralBackground2 }} aria-hidden="true">
      <span className={s.titel} style={{ background: t.colorNeutralBackground3 }}>
        <i className={s.logo} style={{ background: t.colorBrandBackground }} />
        <i className={s.titeltext} style={{ background: t.colorNeutralForeground3 }} />
        <span className={s.kaption}>
          <i className={s.kpunkt} style={{ background: t.colorNeutralForeground2 }} />
          <i className={s.kpunkt} style={{ background: t.colorNeutralForeground2 }} />
          <i className={s.kpunkt} style={{ background: t.colorNeutralForeground2 }} />
        </span>
      </span>
      <span className={s.nav} style={{ background: t.colorNeutralBackground4 }}>
        <span className={s.navaktiv} style={{ background: t.colorNeutralBackground1Selected }}>
          <i className={s.pill} style={{ background: t.colorCompoundBrandStroke }} />
        </span>
        <i className={s.navzeile} style={{ background: t.colorNeutralForeground3 }} />
        <i className={s.navzeile} style={{ background: t.colorNeutralForeground3 }} />
      </span>
      <span className={s.haupt}>
        <i className={s.h} style={{ background: t.colorNeutralForeground1 }} />
        <i className={s.text} style={{ background: t.colorNeutralForeground3 }} />
        <span
          className={s.karte}
          style={{
            background: t.colorNeutralBackground1,
            boxShadow: `0 0 0 1px ${t.colorNeutralStroke2}`,
            borderRadius: halb(t.borderRadiusMedium),
          }}
        >
          {[45, 75, 58, 90].map((h, i) => (
            <i
              key={h}
              className={s.balken}
              style={{
                height: `${h}%`,
                background: i % 2 ? t.colorBrandForegroundLink : t.colorBrandBackground,
                opacity: i % 2 ? 0.75 : 1,
              }}
            />
          ))}
        </span>
        <span className={s.aktionen}>
          <span
            className={s.knopf}
            style={{ background: t.colorBrandBackground, borderRadius: halb(t.borderRadiusMedium) }}
          >
            <i className={s.knopftext} style={{ background: t.colorNeutralForegroundOnBrand }} />
          </span>
          <i className={s.link} style={{ background: t.colorBrandForegroundLink }} />
        </span>
      </span>
    </span>
  );
}

export interface DesignVorschauProps {
  design: AppDesign;
  /** Akzent-Kennung oder Farbe; leer: Markenfarbe des Designs. */
  akzent?: string;
  className?: string;
}

/** Hell (links) und Dunkel (rechts) nebeneinander: beide Modi eines Designs ohne Umschalten. */
export function DesignVorschau({ design, akzent, className }: DesignVorschauProps) {
  const s = useStyles();
  const hell = useMemo(() => themeFuerDesign(design, "hell", { akzent }), [design, akzent]);
  const dunkel = useMemo(() => themeFuerDesign(design, "dunkel", { akzent }), [design, akzent]);
  return (
    <span className={mergeClasses(s.paar, className)} aria-hidden="true">
      <MiniOberflaeche theme={hell} />
      <MiniOberflaeche theme={dunkel} />
    </span>
  );
}
