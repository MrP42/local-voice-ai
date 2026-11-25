import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, FolderOpen, MessageSquare, Plus } from "lucide-react";
import { commands, type ConversationSummary } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { meldeProjekte, oeffneProjekt } from "@/lib/projekteBus";
import { useProjektRaum } from "@/lib/projektRaum";
import { anzahlVon, useProjekteStand } from "../projekte/useProjekte";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import "../projekte/projektbereich.css";

const KLASSEN = ["PUBLIC", "INTERNAL", "PERSONAL", "SPECIAL", "RESTRICTED"];

/** Zeilen-Grundform wie in der Verlaufsliste (gleiche Höhe, gleiche Hervorhebung). */
const ZEILE =
  "flex items-center gap-2 min-h-[44px] md:min-h-9 rounded-md cursor-pointer text-sm text-text text-start";

/**
 * Abschnitt „Projekte“ oben in der Seitenleiste des Chats (lva#826, Muster „Räume“): je Projekt eine
 * Zeile mit Zahl der Quellen; ein Klick öffnet den Projektraum im Chat, der Pfeil klappt die Gespräche
 * des Projekts auf. „＋“ legt ein Projekt an. Projektgespräche stehen nur hier, nicht im allgemeinen
 * Verlauf.
 */
export const ProjektRaeume: React.FC<{
  /** Alle Gespräche der Liste; die mit `project_id` werden unter ihrem Projekt gezeigt. */
  gespraeche: ConversationSummary[];
  aktivesGespraech: string | null;
  onGespraech: (id: string) => void;
  /** Nach jeder Wahl (z. B. Schublade der schmalen Ansicht schließen). */
  onGewaehlt?: () => void;
}> = ({ gespraeche, aktivesGespraech, onGespraech, onGewaehlt }) => {
  const { t } = useTranslation();
  const stand = useProjekteStand();
  const raumId = useProjektRaum((s) => s.raumId);
  const [offen, setOffen] = useState<ReadonlySet<string>>(new Set());
  const [neu, setNeu] = useState(false);
  const [titel, setTitel] = useState("");
  const [klasse, setKlasse] = useState("PERSONAL");
  const [fehler, setFehler] = useState<string | null>(null);

  const umschalten = (id: string) =>
    setOffen((alt) => {
      const n = new Set(alt);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const anlegen = async () => {
    try {
      const p = auspacken(
        await commands.projektAnlegen(titel.trim(), klasse, false),
      );
      setNeu(false);
      setTitel("");
      setFehler(null);
      meldeProjekte("projekte");
      onGewaehlt?.();
      oeffneProjekt(p.id);
    } catch (x) {
      setFehler(fehlerText(x, t));
    }
  };

  return (
    <div
      role="group"
      aria-labelledby="pr-raeume-titel"
      data-testid="projekt-raeume"
      className="mb-3"
    >
      <div className="flex items-center justify-between ps-2">
        <span
          id="pr-raeume-titel"
          className="pb-1 text-xs font-medium text-text-muted"
        >
          {t("projektRaum.titel")}
        </span>
        <button
          type="button"
          data-testid="projekt-neu"
          aria-label={t("projektRaum.neu")}
          title={t("projektRaum.neu")}
          onClick={() => setNeu(true)}
          className="grid place-items-center w-7 h-7 max-sm:w-[44px] max-sm:h-[44px] rounded cursor-pointer text-text/80 hover:bg-mid-gray/15 hover:text-text"
        >
          <Plus className="w-4 h-4" aria-hidden="true" />
        </button>
      </div>
      {stand.projekte.length === 0 && stand.geladen && (
        <button
          type="button"
          onClick={() => setNeu(true)}
          className={`${ZEILE} w-full px-2 text-text-muted hover:bg-mid-gray/10`}
        >
          <Plus className="w-4 h-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{t("projektRaum.neu")}</span>
        </button>
      )}
      <ul className="m-0 p-0 list-none flex flex-col gap-0.5">
        {stand.projekte.map((p) => {
          const auf = offen.has(p.id);
          const eigene = gespraeche.filter((g) => g.project_id === p.id);
          const aktiv = raumId === p.id;
          const listeId = `pr-raum-${p.id}`;
          return (
            <li key={p.id}>
              <div
                className={`flex items-center rounded-md ${aktiv ? "bg-logo-primary/15" : "hover:bg-mid-gray/10"}`}
              >
                <button
                  type="button"
                  aria-expanded={auf}
                  aria-controls={listeId}
                  aria-label={t(
                    auf ? "projektRaum.zuklappen" : "projektRaum.aufklappen",
                    { name: p.title },
                  )}
                  title={t(
                    auf ? "projektRaum.zuklappen" : "projektRaum.aufklappen",
                    { name: p.title },
                  )}
                  onClick={() => umschalten(p.id)}
                  className="grid place-items-center w-7 h-9 max-sm:w-[44px] max-sm:h-[44px] shrink-0 rounded cursor-pointer text-text/70 hover:text-text"
                >
                  <ChevronRight
                    className={`w-3.5 h-3.5 transition-transform ${auf ? "rotate-90" : ""}`}
                    aria-hidden="true"
                  />
                </button>
                <button
                  type="button"
                  data-testid="projekt-zeile"
                  data-projekt={p.id}
                  aria-current={aktiv ? "true" : undefined}
                  title={p.title}
                  onClick={() => {
                    onGewaehlt?.();
                    oeffneProjekt(p.id);
                  }}
                  className={`${ZEILE} flex-1 min-w-0 pe-2 ${aktiv ? "font-semibold" : ""}`}
                >
                  <FolderOpen
                    className="w-4 h-4 shrink-0 text-text/80"
                    aria-hidden="true"
                  />
                  <span className="truncate flex-1">{p.title}</span>
                  <span
                    className="text-xs text-text-muted font-normal"
                    data-testid="projekt-zahl"
                    aria-label={t("projektRaum.quellenZahl", {
                      count: anzahlVon(stand, p.id),
                    })}
                  >
                    {anzahlVon(stand, p.id)}
                  </span>
                </button>
              </div>
              {auf && (
                <ul
                  id={listeId}
                  data-testid="projekt-raum-gespraeche"
                  className="m-0 ps-7 list-none flex flex-col gap-0.5"
                >
                  {eigene.length === 0 && (
                    <li className="px-2 py-1 text-xs text-text-muted">
                      {t("projektRaum.keineGespraeche")}
                    </li>
                  )}
                  {eigene.map((g) => (
                    <li key={g.id}>
                      <button
                        type="button"
                        data-testid="projekt-raum-gespraech"
                        aria-current={
                          g.id === aktivesGespraech ? "true" : undefined
                        }
                        onClick={() => {
                          onGewaehlt?.();
                          onGespraech(g.id);
                        }}
                        className={`${ZEILE} w-full px-2 ${g.id === aktivesGespraech ? "bg-logo-primary/15" : "hover:bg-mid-gray/10"}`}
                      >
                        <MessageSquare
                          className="w-3.5 h-3.5 shrink-0 text-text/60"
                          aria-hidden="true"
                        />
                        <span className="truncate">
                          {g.title || t("projektRaum.ohneTitel")}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
      </ul>

      <Dialog
        contentFades={false}
        open={neu}
        onOpenChange={(o) => {
          setNeu(o);
          if (!o) setFehler(null);
        }}
        title={t("projektRaum.neu")}
        closeLabel={t("projektBereich.abbrechen")}
        footer={
          <>
            <button
              type="button"
              className="pb-btn pb-btn--pri"
              data-testid="projekt-anlegen"
              disabled={titel.trim() === ""}
              onClick={() => void anlegen()}
            >
              {t("projektBereich.anlegen")}
            </button>
            <button
              type="button"
              className="pb-btn"
              onClick={() => setNeu(false)}
            >
              {t("projektBereich.abbrechen")}
            </button>
          </>
        }
      >
        <div className="pb-dlg" style={{ display: "grid", gap: 12 }}>
          <p className="pb-note" style={{ margin: 0 }}>
            {t("projektRaum.neuText")}
          </p>
          <label>
            <span className="pb-note">{t("projektBereich.projektName")}</span>
            <Input
              value={titel}
              maxLength={200}
              aria-label={t("projektBereich.projektName")}
              onChange={(e) => setTitel(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && titel.trim() !== "") void anlegen();
              }}
            />
          </label>
          <div>
            <span className="pb-note">{t("projektBereich.klasse")}</span>
            <Select
              value={klasse}
              options={KLASSEN.map((k) => ({
                value: k,
                label: t(`projekte.klassen.${k}`),
              }))}
              ariaLabel={t("projektBereich.klasse")}
              onChange={(v) => v && setKlasse(v)}
              menuPortal
              isClearable={false}
            />
          </div>
          {fehler && (
            <p role="alert" className="pb-note" style={{ margin: 0 }}>
              {fehler}
            </p>
          )}
        </div>
      </Dialog>
    </div>
  );
};
