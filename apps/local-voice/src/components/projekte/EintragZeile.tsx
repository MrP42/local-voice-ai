import React from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, MoreVertical } from "lucide-react";
import type { EintragSicht, KontextForm, KontextZeile } from "@/bindings";
import {
  gruppeVon,
  hinweisSchluessel,
  schalterGesperrt,
  zahl,
  type Gruppe,
} from "@/lib/projekteKontext";
import { Menue, type MenueZeile } from "./Menue";

export interface Fassung {
  id: string;
  label: string;
}

interface Props {
  eintrag: EintragSicht;
  zeile: KontextZeile | undefined;
  /** Fassungen eines Buches (Hauptfassung zuerst); leer bei allem anderen. */
  fassungen: Fassung[];
  /** Schalten läuft gerade: der Schalter wartet, damit keine zwei Aufrufe überholen. */
  beschaeftigt: boolean;
  onSchalten: (on: boolean) => void;
  /** Form- oder Fassungswechsel: ob der Eintrag an ist, entscheidet die Seite aus dem Backend-Stand. */
  onForm: (form: KontextForm | null, fassung: string | null) => void;
  onEntfernen: () => void;
}

const ART_KEY: Record<Gruppe, string> = {
  buecher: "buch",
  texte: "text",
  aufnahmen: "aufnahme",
  dateien: "datei",
  recherchen: "recherche",
};

export const EintragZeile: React.FC<Props> = ({
  eintrag,
  zeile,
  fassungen,
  beschaeftigt,
  onSchalten,
  onForm,
  onEntfernen,
}) => {
  const { t } = useTranslation();
  const gruppe = gruppeVon(eintrag);
  const name = eintrag.title ?? t("projektBereich.ohneName");
  const gesperrt = schalterGesperrt(zeile) || !eintrag.available;
  const an = !!zeile?.on && !gesperrt;
  const formName = (f: KontextForm) => t(`projektBereich.formen.${f}`);

  const hinweisText = zeile?.hint
    ? t(`projektBereich.hinweis.${hinweisSchluessel(zeile.hint)}`, {
        defaultValue: zeile.hintText ?? "",
      })
    : null;
  const formatHinweis = zeile?.hint === "kontext.format";

  const chips: string[] = [];
  if (eintrag.subtitle) chips.push(eintrag.subtitle);
  if (gruppe === "aufnahmen" && eintrag.assignedBy)
    chips.push(
      t(
        eintrag.assignedBy === "person"
          ? "projektBereich.zugeordnetPerson"
          : "projektBereich.zugeordnetAutomatik",
      ),
    );

  const token = () => {
    if (formatHinweis || !eintrag.available)
      return <span className="pb-tok pb-tok--aus">{hinweisText}</span>;
    if (zeile?.tokens == null)
      return (
        <span className="pb-tok pb-tok--aus">
          {t("projektBereich.keinText")}
        </span>
      );
    return an ? (
      <span className="pb-tok">
        {t("projektBereich.token", { n: zahl(zeile.tokens) })}
      </span>
    ) : (
      <span className="pb-tok pb-tok--aus">
        {t("projektBereich.tokenAus", { n: zahl(zeile.tokens) })}
      </span>
    );
  };

  const formZeilen = (z: KontextZeile): MenueZeile[] => {
    const zeilen: MenueZeile[] = [];
    if (z.forms.length > 1) zeilen.push({ kopf: t("projektBereich.form") });
    const formen = z.forms.length > 1 ? z.forms : [];
    formen.forEach((f, i) =>
      zeilen.push({
        id: `form:${f}`,
        label: formName(f),
        gewaehlt: f === z.form,
        rechts:
          f === z.form && z.tokens != null
            ? [
                t("projektBereich.token", { n: zahl(z.tokens) }),
                i === 0 ? t("projektBereich.vorgabe") : null,
              ]
                .filter(Boolean)
                .join(" · ")
            : i === 0
              ? t("projektBereich.vorgabe")
              : undefined,
      }),
    );
    if (fassungen.length > 1) {
      zeilen.push({ kopf: t("projektBereich.fassung") });
      fassungen.forEach((f, i) =>
        zeilen.push({
          id: `ed:${f.id}`,
          label: f.label,
          gewaehlt: z.edition ? z.edition === f.id : i === 0,
        }),
      );
    }
    return zeilen;
  };

  const wahl = (id: string) => {
    if (id.startsWith("form:")) onForm(id.slice(5) as KontextForm, null);
    else if (id.startsWith("ed:")) onForm(null, id.slice(3));
  };

  const alternative: KontextForm | null =
    zeile?.hint === "kontext.kein_kurzbuch" && zeile.forms.includes("volltext")
      ? "volltext"
      : zeile?.hint === "kontext.kein_protokoll" &&
          zeile.forms.includes("transkript")
        ? "transkript"
        : null;

  return (
    <div className="pb-ent" data-testid="projekt-eintrag" data-id={eintrag.id}>
      <div className="pb-ent__ti">
        <b title={name}>{name}</b>
        {chips.length > 0 && (
          <span className="pb-ent__meta">
            {chips.map((c) => (
              <span key={c} className="pb-chip">
                {c}
              </span>
            ))}
          </span>
        )}
      </div>
      <span className="pb-ent__art">
        {t(`projektBereich.art.${ART_KEY[gruppe]}`)}
      </span>
      <span className="pb-ctx">
        <button
          type="button"
          role="switch"
          aria-checked={an}
          aria-disabled={gesperrt || beschaeftigt || undefined}
          aria-label={
            gesperrt
              ? t("projektBereich.schalterGesperrt", { name })
              : t("projektBereich.schalter", { name })
          }
          data-testid="kontext-schalter"
          className="pb-schalter"
          onClick={() => {
            if (gesperrt || beschaeftigt || !zeile) return;
            onSchalten(!zeile.on);
          }}
        >
          <span className="pb-sw" aria-hidden="true" />
        </button>
        {zeile &&
          (zeile.forms.length > 1 || fassungen.length > 1) &&
          !gesperrt && (
            <Menue
              label={
                zeile.forms.length > 1
                  ? t("projektBereich.formWaehlen", {
                      form: formName(zeile.form),
                    })
                  : t("projektBereich.fassungWaehlen", { name })
              }
              menueLabel={t("projektBereich.formUndFassung")}
              className="pb-form"
              testId="form-wahl"
              align="start"
              zeilen={formZeilen(zeile)}
              onWahl={(id) => wahl(id)}
              knopf={
                <>
                  {formName(zeile.form)}
                  <ChevronDown width={14} height={14} aria-hidden="true" />
                </>
              }
            />
          )}
        {token()}
      </span>
      <Menue
        label={t("projektBereich.weitereAktionen", { name })}
        menueLabel={t("projektBereich.weitereAktionenMenue")}
        className="pb-ib"
        testId="eintrag-menue"
        zeilen={[
          {
            id: "entfernen",
            gefahr: true,
            label: t(
              gruppe === "recherchen"
                ? "projektBereich.rechercheLoeschenMenue"
                : "projektBereich.entfernenMenue",
            ),
          },
        ]}
        onWahl={() => onEntfernen()}
        knopf={<MoreVertical width={18} height={18} aria-hidden="true" />}
      />
      {zeile?.hint && !formatHinweis && eintrag.available && (
        <div className="pb-hint" data-testid="kontext-hinweis">
          <span>
            {hinweisText} {t("projektBereich.nichtImKontextPunkt")}
          </span>
          {alternative && zeile && (
            <button
              type="button"
              className="pb-btn pb-btn--link"
              onClick={() => onForm(alternative, null)}
            >
              {t(`projektBereich.waehlen.${alternative}`)}
            </button>
          )}
        </div>
      )}
    </div>
  );
};
