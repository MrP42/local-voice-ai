import React from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { FileText, Search } from "lucide-react";
import type {
  AblageDatei,
  AblageFundstelle,
  AblageSuchtreffer,
} from "@/bindings";
import { ankerTeile, hervorheben, suchwoerter } from "./logik";
import { SemantikZeile } from "./Semantik";

/** Lesbare Beschriftung eines Ankers (`seite=3` → „Seite 3“). */
export function ankerText(t: TFunction, anker: string): string {
  const a = ankerTeile(anker);
  switch (a.art) {
    case "seite":
    case "absatz":
    case "folie":
    case "zeile":
    case "teil":
      return t(`ablage.anker.${a.art}`, { n: a.n });
    case "kopf":
    case "text":
      return t(`ablage.anker.${a.art}`);
    case "blatt":
      return t("ablage.anker.blatt", {
        blatt: a.blatt,
        von: a.von,
        bis: a.bis,
      });
    default:
      return a.text;
  }
}

/** Beschriftung samt Überschriftenpfad: „Absatz 4 · Vertrag › § 4 Laufzeit“. */
export function fundstelleText(t: TFunction, f: AblageFundstelle): string {
  const a = ankerTeile(f.anker);
  // Beim Blatt steht der Name schon im Anker.
  const pfad = a.art === "blatt" ? [] : f.pfad;
  return [ankerText(t, f.anker), pfad.join(" › ")].filter(Boolean).join(" · ");
}

export const Auszug: React.FC<{ text: string; anfrage: string }> = ({
  text,
  anfrage,
}) => (
  <>
    {hervorheben(text, suchwoerter(anfrage)).map((s, i) =>
      s.treffer ? <mark key={i}>{s.text}</mark> : <span key={i}>{s.text}</span>,
    )}
  </>
);

/**
 * Suchergebnisse über die ganze Ablage (ohne KI): Treffer im Namen und im Inhalt
 * (mit Auszug und Fundstelle). Der Papierkorb ist ausgenommen.
 */
export const SuchErgebnisse: React.FC<{
  anfrage: string;
  namen: AblageDatei[];
  inhalt: AblageSuchtreffer[] | null;
  fehler: boolean;
  pfadText(ordner: string): string;
  /** D6: Suchart geändert (semantisch an/aus, Vektoren fertig) – Inhaltssuche erneut ausführen. */
  onSuchartGeaendert(): void;
  onOeffnen(
    dat: string,
    ordner: string,
    fundstelle: AblageFundstelle | null,
    ausloeser: HTMLElement,
  ): void;
}> = ({
  anfrage,
  namen,
  inhalt,
  fehler,
  pfadText,
  onSuchartGeaendert,
  onOeffnen,
}) => {
  const { t } = useTranslation();
  return (
    <section
      className="ablage-suche"
      aria-label={t("ablage.suchergebnis.titel")}
      data-testid="ablage-suchergebnisse"
    >
      <div className="ablage-suche__kopf">
        <Search size={16} aria-hidden="true" />
        <span>{t("ablage.suchergebnis.titel")}</span>
        <span className="ablage-mut">{t("ablage.suchergebnis.hinweis")}</span>
      </div>
      <SemantikZeile onGeaendert={onSuchartGeaendert} />
      {namen.length > 0 && (
        <>
          <h3 className="ablage-suche__titel">
            {t("ablage.suchergebnis.imNamen", { anzahl: namen.length })}
          </h3>
          <ul className="ablage-suche__liste">
            {namen.map((d) => (
              <li key={d.id}>
                <button
                  type="button"
                  className="ablage-suche__treffer"
                  data-testid={`ablage-namenstreffer-${d.id}`}
                  onClick={(e) =>
                    onOeffnen(d.id, d.ordner, null, e.currentTarget)
                  }
                >
                  <FileText size={18} aria-hidden="true" />
                  <span className="ablage-suche__name">
                    <Auszug text={d.name} anfrage={anfrage} />
                  </span>
                  <span className="ablage-mut">{pfadText(d.ordner)}</span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      <h3 className="ablage-suche__titel">
        {inhalt
          ? t("ablage.suchergebnis.imInhalt", { anzahl: inhalt.length })
          : t("ablage.suchergebnis.imInhaltOhneZahl")}
      </h3>
      {fehler ? (
        <div className="ablage-mut" role="status">
          {t("ablage.suchergebnis.fehler")}
        </div>
      ) : inhalt === null ? (
        <div className="ablage-mut" role="status">
          {t("ablage.suchergebnis.laedt")}
        </div>
      ) : inhalt.length === 0 ? (
        <div
          className="ablage-mut"
          role="status"
          data-testid="ablage-inhalt-leer"
        >
          {t("ablage.suchergebnis.keine")}
        </div>
      ) : (
        <ul className="ablage-suche__liste">
          {inhalt.map((tr) => (
            <li key={tr.dat} className="ablage-suche__datei">
              <div className="ablage-suche__dateikopf">
                <FileText size={18} aria-hidden="true" />
                <span className="ablage-suche__name">{tr.name}</span>
                <span className="ablage-mut">{pfadText(tr.ordner)}</span>
              </div>
              {tr.stellen.map((f) => (
                <button
                  type="button"
                  key={f.anker}
                  className="ablage-suche__stelle"
                  data-testid={`ablage-inhaltstreffer-${tr.dat}`}
                  aria-label={t("ablage.suchergebnis.oeffnen", {
                    name: tr.name,
                    anker: fundstelleText(t, f),
                  })}
                  onClick={(e) =>
                    onOeffnen(tr.dat, tr.ordner, f, e.currentTarget)
                  }
                >
                  <span className="ablage-suche__anker">
                    {fundstelleText(t, f)}
                  </span>
                  <span className="ablage-suche__auszug">
                    <Auszug text={f.auszug} anfrage={anfrage} />
                  </span>
                </button>
              ))}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
};
