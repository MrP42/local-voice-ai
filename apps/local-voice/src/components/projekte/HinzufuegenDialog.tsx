import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as dateienWaehlen } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
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
}

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

  useEffect(() => {
    if (!open) return;
    setTab("app");
    setGewaehlt(new Set());
    setFehler(null);
    let aktuell = true;
    void (async () => {
      const [b, p, m] = await Promise.all([
        commands.libraryList().catch(() => null),
        commands.pagesList().catch(() => null),
        commands.meetingsList(0, 500).catch(() => null),
      ]);
      if (!aktuell) return;
      const liste = (r: typeof b | typeof p | typeof m) =>
        r && r.status === "ok" && Array.isArray(r.data) ? r.data : [];
      setKandidaten([
        ...liste(b).map((x) => ({
          key: `buch:${x.id}`,
          art: "buch" as const,
          id: x.id,
          titel: (x as { title: string }).title,
        })),
        ...liste(p).map((x) => ({
          key: `text:${x.id}`,
          art: "text" as const,
          id: x.id,
          titel: (x as { title: string }).title,
        })),
        ...liste(m).map((x) => ({
          key: `aufnahme:${x.id}`,
          art: "aufnahme" as const,
          id: x.id,
          titel: (x as { title: string }).title,
        })),
      ]);
    })();
    return () => {
      aktuell = false;
    };
  }, [open]);

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

  const dateien = useCallback(async () => {
    if (!projekt) return;
    setFehler(null);
    try {
      const wahl = await dateienWaehlen({ multiple: true, directory: false });
      const paths = Array.isArray(wahl) ? wahl : wahl ? [wahl] : [];
      if (paths.length === 0) return;
      const r = auspacken(
        await commands.projektDateienHinzufuegen(projekt.id, paths),
      );
      toast.success(
        t("projektBereich.dialog.kopiert", {
          count: r?.length ?? paths.length,
        }),
      );
      onGeaendert();
      onClose();
    } catch (e) {
      setFehler(fehlerText(e, t));
    }
  }, [projekt, onGeaendert, onClose, t]);

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
                {frei.length === 0 && (
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
                  onClick={() => void dateien()}
                >
                  {t("projektBereich.dialog.dateienWaehlen")}
                </button>
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
