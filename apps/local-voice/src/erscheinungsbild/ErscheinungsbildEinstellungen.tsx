/**
 * @wai/ui erscheinungsbild v1.1 — ErscheinungsbildEinstellungen.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild (Vorbild: WAI Presenter, einstellungen/Erscheinungsbild.tsx).
 * Selbsttragend: importiert nur react, @fluentui/react-components und Nachbardateien.
 *
 * Fertiger Abschnitt „Erscheinungsbild“ im Stil der Windows-11-Einstellungen: Karten-Zeilen mit Titel und
 * Hilfetext links, Bedienelement rechts (Farbschema, Kontrast, Animationen) und aufgeklappte Karten fuer
 * Design und Akzentfarbe. Wirkt sofort, ohne „Speichern“. Texte deutsch, je Schluessel ueberschreibbar.
 */
import {
  Dropdown,
  makeStyles,
  mergeClasses,
  Option,
  Switch,
  tokens,
} from "@fluentui/react-components";
import { type ReactNode, useId } from "react";
import { AkzentAuswahl } from "./AkzentAuswahl";
import { DesignAuswahl, type DesignTexte, designText } from "./DesignAuswahl";
import { AKZENTE } from "./designs";
import { useErscheinungsbildKontext } from "./ErscheinungsbildProvider";
import type { Kontrast } from "./theme";
import type { Erscheinungsbild, Modus } from "./zustand";

export interface ErscheinungsbildTexte {
  titel: string;
  farbschema: string;
  farbschemaHilfe: string;
  modusSystem: string;
  modusHell: string;
  modusDunkel: string;
  kontrast: string;
  kontrastHilfe: string;
  kontrastStandard: string;
  kontrastHoch: string;
  animationen: string;
  animationenHilfe: string;
  animationenSystem: string;
  ein: string;
  aus: string;
  design: string;
  designHilfe: string;
  akzent: string;
  akzentHilfe: string;
  akzentHochKontrast: string;
  /** Hilfetext der Akzentzeile, solange das Design gesperrt ist. */
  akzentGesperrt: string;
  /** Vorsatz vor dem gewaehlten Akzentnamen („Gewählt:“). */
  akzentGewaehlt: string;
  /** Zugaenglicher Name der Farbfelder; `{akzent}` = Zeilentitel, `{design}` = Designname. */
  akzentGruppe: string;
  /** Akzentnamen je Kennung (blau, petrol, …; „standard“ = „Wie das Design“). */
  akzentNamen: Readonly<Record<string, string>>;
  /** Namen/Beschreibungen je Design-Kennung (eingebaute: microsoft, wai); fehlend = Werte des Designs. */
  designs: DesignTexte;
}

export const STANDARD_TEXTE: ErscheinungsbildTexte = {
  titel: "Erscheinungsbild",
  farbschema: "Farbschema",
  farbschemaHilfe: "Hell, Dunkel oder wie in Windows eingestellt.",
  modusSystem: "Wie System",
  modusHell: "Hell",
  modusDunkel: "Dunkel",
  kontrast: "Kontrast",
  kontrastHilfe:
    "Hoch nutzt die Fluent-Kontrastpalette mit kräftigen Rändern. Design und Akzent ruhen dann.",
  kontrastStandard: "Standard",
  kontrastHoch: "Hoch",
  animationen: "Animationen",
  animationenHilfe: "Weiche Übergänge in der Oberfläche.",
  animationenSystem:
    "Im Betriebssystem ist „Animationen reduzieren“ eingestellt – Animationen bleiben aus.",
  ein: "Ein",
  aus: "Aus",
  design: "Design",
  designHilfe: "Jede Karte zeigt links den hellen und rechts den dunklen Modus.",
  akzent: "Akzentfarbe",
  akzentHilfe:
    "Für Knöpfe, Links und Auswahl. Jede Farbe wird so angepasst, dass Text darauf gut lesbar bleibt.",
  akzentHochKontrast: "Bei hohem Kontrast ruhen Akzentfarben.",
  akzentGesperrt: "Die Akzentfarbe folgt der Design-Vorgabe.",
  akzentGewaehlt: "Gewählt:",
  akzentGruppe: "{akzent} für {design}",
  akzentNamen: Object.fromEntries(AKZENTE.map((a) => [a.id, a.name])),
  designs: {},
};

/** Texte mit Vorgaben zusammenfuehren; Akzentnamen und Designtexte werden je Schluessel ergaenzt. */
export function erscheinungsbildTexte(
  teil: Partial<ErscheinungsbildTexte> = {},
): ErscheinungsbildTexte {
  return {
    ...STANDARD_TEXTE,
    ...teil,
    akzentNamen: { ...STANDARD_TEXTE.akzentNamen, ...teil.akzentNamen },
    designs: { ...STANDARD_TEXTE.designs, ...teil.designs },
  };
}

const useStyles = makeStyles({
  abschnitt: { display: "grid", gap: tokens.spacingVerticalXS, maxWidth: "1000px" },
  titel: {
    margin: `0 0 ${tokens.spacingVerticalS}`,
    fontSize: tokens.fontSizeBase300,
    lineHeight: tokens.lineHeightBase300,
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
  },
  karte: {
    display: "grid",
    backgroundColor: tokens.colorNeutralBackground1,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    color: tokens.colorNeutralForeground1,
  },
  zeile: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalL,
    minHeight: "68px",
    boxSizing: "border-box",
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalL}`,
  },
  text: { display: "grid", gap: "2px", flex: "1 1 260px", minWidth: 0 },
  kopf: { fontSize: tokens.fontSizeBase300, lineHeight: tokens.lineHeightBase300 },
  hilfe: {
    fontSize: tokens.fontSizeBase200,
    lineHeight: tokens.lineHeightBase200,
    color: tokens.colorNeutralForeground3,
  },
  steuerung: { flex: "none", display: "flex", alignItems: "center" },
  dropdown: { minWidth: "180px" },
  inhalt: {
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalL} ${tokens.spacingVerticalL}`,
    borderTop: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
  },
  ruht: { opacity: 0.5, cursor: "not-allowed" },
  hinweis: {
    margin: `0 0 ${tokens.spacingVerticalM}`,
    fontSize: tokens.fontSizeBase200,
    lineHeight: tokens.lineHeightBase200,
    color: tokens.colorNeutralForeground2,
  },
});

export interface EinstellungsKarteProps {
  kopf: string;
  hilfe?: ReactNode;
  kopfId?: string;
  hilfeId?: string;
  /** Bedienelement rechts in der Zeile. */
  steuerung?: ReactNode;
  /** Aufgeklappter Inhalt unter der Zeile (z. B. Kartenauswahl). */
  children?: ReactNode;
}

/** Eine Karten-Zeile im Stil der Windows-11-Einstellungen; auch fuer App-Zusaetze im `zusatz`-Slot. */
export function EinstellungsKarte({
  kopf,
  hilfe,
  kopfId: kopfIdProp,
  hilfeId: hilfeIdProp,
  steuerung,
  children,
}: EinstellungsKarteProps) {
  const s = useStyles();
  const eigen = useId();
  const kopfId = kopfIdProp ?? `${eigen}-k`;
  const hilfeId = hilfeIdProp ?? `${eigen}-h`;
  return (
    <div className={s.karte}>
      <div className={s.zeile}>
        <div className={s.text}>
          <span id={kopfId} className={s.kopf}>
            {kopf}
          </span>
          {hilfe ? (
            <span id={hilfeId} className={s.hilfe}>
              {hilfe}
            </span>
          ) : null}
        </div>
        {steuerung ? <div className={s.steuerung}>{steuerung}</div> : null}
      </div>
      {children ? <div className={s.inhalt}>{children}</div> : null}
    </div>
  );
}

export interface ErscheinungsbildEinstellungenProps {
  /** Zustand; ohne Angabe aus dem umgebenden `ErscheinungsbildProvider`. */
  zustand?: Erscheinungsbild;
  texte?: Partial<ErscheinungsbildTexte>;
  /** App-Zusaetze (weitere Karten-Zeilen) am Ende des Abschnitts. */
  zusatz?: ReactNode;
  /** App-Zeilen direkt vor der Design-Karte (z. B. „Design der App kommt aus …“). */
  vorDesign?: ReactNode;
  /**
   * Design-Wahl gesperrt (z. B. Design folgt einer Vorlage): Karten und Akzent sind nicht waehlbar und nicht
   * fokussierbar; `hinweis` steht ueber den Karten. Unabhaengig von der Sperre durch Kontrast „Hoch“.
   */
  designGesperrt?: { hinweis: ReactNode };
  /** Ueberschrift ausblenden, wenn die Seite selbst schon „Erscheinungsbild“ heisst. */
  ohneTitel?: boolean;
  className?: string;
}

export function ErscheinungsbildEinstellungen({
  zustand,
  texte,
  zusatz,
  vorDesign,
  designGesperrt,
  ohneTitel = false,
  className,
}: ErscheinungsbildEinstellungenProps) {
  const s = useStyles();
  const ausKontext = useErscheinungsbildKontext();
  const z = zustand ?? ausKontext;
  const id = useId();
  if (!z)
    throw new Error(
      "ErscheinungsbildEinstellungen braucht einen ErscheinungsbildProvider oder `zustand`.",
    );
  const t = erscheinungsbildTexte(texte);
  const ids = (k: string) => ({ kopfId: `${id}-${k}`, hilfeId: `${id}-${k}-h` });
  const modusText: Record<Modus, string> = {
    system: t.modusSystem,
    hell: t.modusHell,
    dunkel: t.modusDunkel,
  };
  const kontrastText: Record<Kontrast, string> = {
    standard: t.kontrastStandard,
    hoch: t.kontrastHoch,
  };
  const hoch = z.kontrast === "hoch";
  const gesperrt = designGesperrt !== undefined;
  // Beide Gruende sperren den Akzent; keiner hebt den anderen auf.
  const akzentGesperrt = hoch || gesperrt;
  const akzentHilfe =
    [hoch ? t.akzentHochKontrast : "", gesperrt ? t.akzentGesperrt : ""]
      .filter(Boolean)
      .join(" ") || t.akzentHilfe;
  const designName = designText(z.design, t.designs).name;
  const titelId = `${id}-titel`;

  return (
    <section
      className={mergeClasses(s.abschnitt, className)}
      aria-labelledby={ohneTitel ? undefined : titelId}
      aria-label={ohneTitel ? t.titel : undefined}
    >
      {ohneTitel ? null : (
        <h2 id={titelId} className={s.titel}>
          {t.titel}
        </h2>
      )}

      <EinstellungsKarte
        kopf={t.farbschema}
        hilfe={t.farbschemaHilfe}
        {...ids("modus")}
        steuerung={
          <Dropdown
            className={s.dropdown}
            aria-labelledby={ids("modus").kopfId}
            aria-describedby={ids("modus").hilfeId}
            value={modusText[z.modus]}
            selectedOptions={[z.modus]}
            onOptionSelect={(_, d) => d.optionValue && z.setzeModus(d.optionValue as Modus)}
          >
            <Option value="system">{t.modusSystem}</Option>
            <Option value="hell">{t.modusHell}</Option>
            <Option value="dunkel">{t.modusDunkel}</Option>
          </Dropdown>
        }
      />

      <EinstellungsKarte
        kopf={t.kontrast}
        hilfe={t.kontrastHilfe}
        {...ids("kontrast")}
        steuerung={
          <Dropdown
            className={s.dropdown}
            aria-labelledby={ids("kontrast").kopfId}
            aria-describedby={ids("kontrast").hilfeId}
            value={kontrastText[z.kontrast]}
            selectedOptions={[z.kontrast]}
            onOptionSelect={(_, d) => d.optionValue && z.setzeKontrast(d.optionValue as Kontrast)}
          >
            <Option value="standard">{t.kontrastStandard}</Option>
            <Option value="hoch">{t.kontrastHoch}</Option>
          </Dropdown>
        }
      />

      <EinstellungsKarte
        kopf={t.animationen}
        hilfe={z.bewegungReduziert ? t.animationenSystem : t.animationenHilfe}
        {...ids("animationen")}
        steuerung={
          <Switch
            checked={z.animationenWirksam}
            disabled={z.bewegungReduziert}
            onChange={(_, d) => z.setzeAnimationen(d.checked)}
            aria-labelledby={ids("animationen").kopfId}
            aria-describedby={ids("animationen").hilfeId}
            label={z.animationenWirksam ? t.ein : t.aus}
            labelPosition="before"
          />
        }
      />

      {vorDesign}

      <EinstellungsKarte kopf={t.design} hilfe={t.designHilfe} {...ids("design")}>
        {gesperrt ? (
          <div id={`${id}-sperre`} className={s.hinweis}>
            {designGesperrt.hinweis}
          </div>
        ) : null}
        <DesignAuswahl
          designs={z.designs}
          wert={z.design.id}
          beiWechsel={z.setzeDesign}
          akzent={z.akzent}
          labelledBy={ids("design").kopfId}
          describedBy={gesperrt ? `${id}-sperre` : undefined}
          gesperrt={gesperrt}
          designTexte={t.designs}
        />
      </EinstellungsKarte>

      <EinstellungsKarte kopf={t.akzent} hilfe={akzentHilfe} {...ids("akzent")}>
        <AkzentAuswahl
          className={akzentGesperrt ? s.ruht : undefined}
          design={z.design}
          schema={z.schema}
          wert={z.akzentAktuell}
          beiWechsel={(a) => z.setzeAkzent(a)}
          label={t.akzentGruppe.replace("{akzent}", t.akzent).replace("{design}", designName)}
          gewaehltText={t.akzentGewaehlt}
          akzentNamen={t.akzentNamen}
          deaktiviert={akzentGesperrt}
        />
      </EinstellungsKarte>

      {zusatz}
    </section>
  );
}
