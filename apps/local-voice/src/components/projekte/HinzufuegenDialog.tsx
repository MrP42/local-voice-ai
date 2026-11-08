import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type EintragSicht, type SuiteProjekt } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { buchIdVon, besprechungIdVon, seiteIdVon } from "@/lib/projekteKontext";
import { Dialog } from "../ui/Dialog";

type Art = "buch" | "text" | "aufnahme";
interface Kandidat {
  key: string;
  art: Art;
  id: string;
  titel: string;
}

interface Props {
  open: boolean;
  projekt: SuiteProjekt | null;
  eintraege: EintragSicht[];
  onClose: () => void;
  /** Etwas wurde zugeordnet oder kopiert: Seite und Leisten neu laden. */
  onGeaendert: () => void;
  /** Eine Dateikopie läuft (Seite hält die Sperre, auch bei geschlossenem Dialog). */
  kopiert: boolean;
  /** Dateiauswahl und Kopieren; liefert eine Fehlermeldung oder null. */
  onDateien: () => Promise<string | null>;
}

const SEITE = 500;
const alsKandidaten = <T extends { id: string; title: string }>(
  art: Art,
  r: { status: string; data?: unknown },
): Kandidat[] => {
  if (r.status !== "ok" || !Array.isArray(r.data)) throw new Error("liste");
  return (r.data as T[]).map((x) => ({
    key: `${art}:${x.id}`,
    art,
    id: x.id,
    titel: x.title,
  }));
};

const ARTEN: Art[] = ["buch", "text", "aufnahme"];
const GRUPPE_KEY: Record<Art, string> = {
  buch: "buecher",
  text: "texte",
  aufnahme: "aufnahmen",
};

/**
 * Hinzufügen zu einem Projekt: Reiter „Aus der App“ (Bücher, Texte, Aufnahmen werden verknüpft,
 * nicht kopiert) und „Dateien kopieren“ (Dateiauswahl des Systems; das Backend kopiert in den
 * Projektordner und prüft die Kopie). Was schon im Projekt ist, wird nicht angeboten.
 */
export const HinzufuegenDialog: React.FC<Props> = ({
  open,
  projekt,
  eintraege,
  onClose,
  onGeaendert,
  kopiert,
  onDateien,
}) => {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"app" | "dateien">("app");
  const [kandidaten, setKandidaten] = useState<Kandidat[]>([]);
  const [gewaehlt, setGewaehlt] = useState<Set<string>>(new Set());
  const [laeuft, setLaeuft] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);

  const schon = useMemo(
    () => new Set(eintraege.map((e) => e.link)),
    [eintraege],
  );

  const [laden, setLaden] = useState<"laedt" | "fehler" | "fertig">("laedt");
  const [weitere, setWeitere] = useState(false);
  const [ladeMehr, setLadeMehr] = useState(false);
  const [versuch, setVersuch] = useState(0);
  const aufnahmen = React.useRef(0);

  useEffect(() => {
    if (!open) return;
    setTab("app");
    setGewaehlt(new Set());
    setFehler(null);
    setLaden("laedt");
    setWeitere(false);
    aufnahmen.current = 0;
    let aktuell = true;
    void (async () => {
      try {
        const [b, p, m] = await Promise.all([
          commands.libraryList(),
          commands.pagesList(),
          commands.meetingsList(0, SEITE),
        ]);
        const liste = [
          ...alsKandidaten<{ id: string; title: string }>("buch", b),
          ...alsKandidaten<{ id: string; title: string }>("text", p),
          ...alsKandidaten<{ id: string; title: string }>("aufnahme", m),
        ];
        if (!aktuell) return;
        const n =
          m.status === "ok" && Array.isArray(m.data) ? m.data.length : 0;
        aufnahmen.current = n;
        setWeitere(n >= SEITE);
        setKandidaten(liste);
        setLaden("fertig");
      } catch {
        if (aktuell) setLaden("fehler");
      }
    })();
    return () => {
      aktuell = false;
    };
  }, [open, versuch]);

  const mehrLaden = async () => {
    if (ladeMehr) return;
    setLadeMehr(true);
    setFehler(null);
    try {
      const r = await commands.meetingsList(aufnahmen.current, SEITE);
      const neu = alsKandidaten<{ id: string; title: string }>("aufnahme", r);
      aufnahmen.current += neu.length;
      setWeitere(neu.length >= SEITE);
      setKandidaten((alt) => {
        const da = new Set(alt.map((k) => k.key));
        return [...alt, ...neu.filter((k) => !da.has(k.key))];
      });
    } catch {
      setFehler(t("projektBereich.dialog.ladefehler"));
    } finally {
      setLadeMehr(false);
    }
  };

  const frei = useMemo(() => {
    const belegt = new Set<string>();
    for (const e of eintraege) {
      const b = buchIdVon(e.link);
      const s = seiteIdVon(e.link);
      const m = besprechungIdVon(e.link);
      if (b) belegt.add(`buch:${b}`);
      if (s) belegt.add(`text:${s}`);
      if (m) belegt.add(`aufnahme:${m}`);
    }
    return kandidaten.filter((k) => !belegt.has(k.key) && !schon.has(k.key));
  }, [kandidaten, eintraege, schon]);

  const umschalten = (key: string) =>
    setGewaehlt((alt) => {
      const neu = new Set(alt);
      if (!neu.delete(key)) neu.add(key);
      return neu;
    });

  const hinzufuegen = useCallback(async () => {
    if (!projekt || laeuft) return;
    setLaeuft(true);
    setFehler(null);
    try {
      for (const k of frei.filter((x) => gewaehlt.has(x.key))) {
        if (k.art === "buch")
          auspacken(await commands.projektBuchZuordnen(projekt.id, k.id));
        else if (k.art === "text")
          auspacken(await commands.projektSeiteZuordnen(projekt.id, k.id));
        else
          auspacken(
            await commands.projektBesprechungZuordnen(projekt.id, k.id),
          );
      }
      onGeaendert();
      onClose();
    } catch (e) {
      setFehler(fehlerText(e, t));
      onGeaendert();
    } finally {
      setLaeuft(false);
    }
  }, [projekt, laeuft, frei, gewaehlt, onGeaendert, onClose, t]);

  const dateien = async () => {
    setFehler(null);
    const f = await onDateien();
    if (f) setFehler(f);
  };

  const anzahl = [...gewaehlt].filter((k) =>
    frei.some((f) => f.key === k),
  ).length;
  const tabs = [
    { id: "app" as const, label: t("projektBereich.dialog.ausDerApp") },
    {
      id: "dateien" as const,
      label: t("projektBereich.dialog.dateienKopieren"),
    },
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("projektBereich.dialog.titel", { name: projekt?.title ?? "" })}
      closeLabel={t("projektBereich.abbrechen")}
      footer={
        tab === "app" ? (
          <>
            <button
              type="button"
              className="pb-btn pb-btn--pri"
              disabled={anzahl === 0 || laeuft}
              data-testid="hinzufuegen-bestaetigen"
              onClick={() => void hinzufuegen()}
            >
              {t("projektBereich.dialog.nHinzufuegen", { count: anzahl })}
            </button>
            <button type="button" className="pb-btn" onClick={onClose}>
              {t("projektBereich.abbrechen")}
            </button>
          </>
        ) : (
          <button type="button" className="pb-btn" onClick={onClose}>
            {t("projektBereich.dialog.schliessen")}
          </button>
        )
      }
    >
      <div className="pb-dlg">
        <div className="pb-tabs" role="tablist">
          {tabs.map((x, i) => (
            <button
              key={x.id}
              type="button"
              role="tab"
              id={`pb-tab-${x.id}`}
              aria-selected={tab === x.id}
              aria-controls="pb-tabpanel"
              tabIndex={tab === x.id ? 0 : -1}
              className="pb-tab"
              onClick={() => setTab(x.id)}
              onKeyDown={(e) => {
                if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
                e.preventDefault();
                const n =
                  tabs[
                    (i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) %
                      tabs.length
                  ];
                setTab(n.id);
                document.getElementById(`pb-tab-${n.id}`)?.focus();
              }}
            >
              {x.label}
            </button>
          ))}
        </div>
        <div id="pb-tabpanel" role="tabpanel" aria-labelledby={`pb-tab-${tab}`}>
          {fehler && (
            <p
              role="alert"
              style={{ color: "var(--color-status-red)", margin: "0 0 8px" }}
            >
              {fehler}
            </p>
          )}
          {tab === "app" ? (
            <>
              <p className="pb-note" style={{ margin: "0 0 4px" }}>
                {t("projektBereich.dialog.appHinweis")}
              </p>
              <div className="pb-dlg-liste">
                {laden === "fehler" && (
                  <div role="alert" data-testid="dialog-ladefehler">
                    <p style={{ color: "var(--color-status-red)" }}>
                      {t("projektBereich.dialog.ladefehler")}
                    </p>
                    <button
                      type="button"
                      className="pb-btn"
                      data-testid="dialog-erneut"
                      onClick={() => setVersuch((v) => v + 1)}
                    >
                      {t("projektBereich.dialog.erneut")}
                    </button>
                  </div>
                )}
                {laden === "fertig" && frei.length === 0 && (
                  <p className="pb-note" data-testid="dialog-nichts">
                    {t("projektBereich.dialog.nichtsFrei")}
                  </p>
                )}
                {ARTEN.map((art) => {
                  const liste = frei.filter((k) => k.art === art);
                  if (liste.length === 0) return null;
                  return (
                    <div
                      key={art}
                      role="group"
                      aria-label={t(
                        `projektBereich.gruppen.${GRUPPE_KEY[art]}`,
                      )}
                    >
                      <div className="pb-grp">
                        {t(`projektBereich.gruppen.${GRUPPE_KEY[art]}`)}
                      </div>
                      {liste.map((k) => (
                        <label key={k.key} className="pb-pick">
                          <input
                            type="checkbox"
                            checked={gewaehlt.has(k.key)}
                            onChange={() => umschalten(k.key)}
                          />
                          <span>{k.titel}</span>
                          <span className="pb-pick__art">
                            {t(`projektBereich.art.${art}`)}
                          </span>
                        </label>
                      ))}
                    </div>
                  );
                })}
                {laden === "fertig" && weitere && (
                  <button
                    type="button"
                    className="pb-btn"
                    data-testid="dialog-mehr"
                    disabled={ladeMehr}
                    onClick={() => void mehrLaden()}
                  >
                    {t("projektBereich.dialog.weitereLaden")}
                  </button>
                )}
              </div>
            </>
          ) : (
            <>
              <p style={{ margin: "4px 0" }}>
                {t("projektBereich.dialog.dateienText")}
              </p>
              <div className="pb-drop">
                <button
                  type="button"
                  className="pb-btn"
                  data-testid="dateien-waehlen"
                  disabled={kopiert}
                  onClick={() => void dateien()}
                >
                  {t("projektBereich.dialog.dateienWaehlen")}
                </button>
                {kopiert && (
                  <p
                    className="pb-note"
                    role="status"
                    data-testid="kopier-hinweis"
                    style={{ margin: "8px 0 0" }}
                  >
                    {t("projektBereich.dialog.kopiertLaeuft")}
                  </p>
                )}
              </div>
              <p className="pb-note" style={{ margin: "8px 0 0" }}>
                {t("projektBereich.dialog.formatHinweis")}
              </p>
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
};
