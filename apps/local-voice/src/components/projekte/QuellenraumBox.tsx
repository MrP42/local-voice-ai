import "./quellenraum.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type QuellenraumSicht } from "@/bindings";
import { auspacken } from "@/lib/projekte";
import { useProjekteMeldung } from "@/lib/projekteBus";
import {
  artSchluessel,
  auswahlText,
  sortiert,
  versionLabel,
  zustandSchluessel,
} from "@/lib/quellenraumLogik";

export function QuellenraumBox({
  projektId,
  stand,
}: {
  projektId: string;
  stand: string | null;
}) {
  const { t } = useTranslation();
  const [sicht, setSicht] = useState<{
    projektId: string;
    daten: QuellenraumSicht;
  } | null>(null);
  const [fehler, setFehler] = useState<string | null>(null);
  const lauf = useRef(0);

  const neuLaden = useCallback(async () => {
    const mein = ++lauf.current;
    try {
      const daten = auspacken(await commands.projektQuellenraum(projektId));
      if (mein !== lauf.current) return;
      setSicht({ projektId, daten });
      setFehler(null);
    } catch {
      if (mein !== lauf.current) return;
      setSicht(null);
      setFehler(projektId);
    }
  }, [projektId]);

  useEffect(() => {
    void neuLaden();
    // Auch beim Unmount und Projektwechsel laufende Antworten entwerten.
    return () => {
      lauf.current += 1;
    };
  }, [neuLaden, stand]);
  useProjekteMeldung("projekte", () => void neuLaden());

  const daten = sicht?.projektId === projektId ? sicht.daten : null;
  const auswahl = daten ? auswahlText(daten) : { count: 0, total: 0 };

  return (
    <section className="pb-box" data-testid="quellenraum">
      <h3>{t("projektBereich.quellenraum.titel")}</h3>
      <p role="status" data-testid="quellenraum-auswahl">
        {t("projektBereich.quellenraum.auswahl", auswahl)}
      </p>
      {fehler === projektId && (
        <p role="alert" className="pb-note">
          {t("projektBereich.quellenraum.fehler")}
        </p>
      )}
      {daten?.reset && (
        <p className="pb-note">
          {t("projektBereich.quellenraum.reset")}
        </p>
      )}
      {daten?.sources.length === 0 && (
        <p className="pb-note">
          {t("projektBereich.quellenraum.leer")}
        </p>
      )}
      <ul
        className="pb-qr-liste"
        aria-label={t("projektBereich.quellenraum.liste")}
      >
        {sortiert(daten?.sources ?? []).map((z) => {
          const version = versionLabel(z);
          return (
            <li className="pb-qr-zeile" key={z.id}>
              <span>
                {t(`projektBereich.quellenraum.art.${artSchluessel(z)}`)}
              </span>
              <span className="pb-qr-titel">
                {z.title ?? t("projektBereich.quellenraum.ohneTitel")}
              </span>
              <span className="pb-qr-badges">
                <span className="pb-qr-badge">
                  {t(
                    `projektBereich.quellenraum.zustand.${zustandSchluessel(z)}`,
                  )}
                </span>
                {version && (
                  <span className="pb-qr-badge">
                    {t(
                      version.anzahl > 1
                        ? "projektBereich.quellenraum.versionVon"
                        : "projektBereich.quellenraum.version",
                      version,
                    )}
                  </span>
                )}
                <span className="pb-qr-badge">
                  {t(
                    z.selected
                      ? "projektBereich.quellenraum.imKontext"
                      : "projektBereich.quellenraum.nichtImKontext",
                  )}
                </span>
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
