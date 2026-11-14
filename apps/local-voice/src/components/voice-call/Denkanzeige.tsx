import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight } from "lucide-react";
import {
  useVoiceCallStore,
  type Denkprozess,
  type DenkSchritt,
} from "@/stores/voiceCallStore";

// Denk-Anzeige (Patrick 12.11.2026): statt einer gesprochenen Floskel („Ich denke kurz nach.“) steht
// unter der letzten Nachricht ein graues, animiertes „Denkt nach …“. Ein Klick klappt den Denkprozess
// auf, der live mitläuft (Modell, Abruf eigener Inhalte, Werkzeuge, Zwischenschritte und der gestreamte
// Denktext des Modells, wo Anbieter oder CLI ihn liefern). Nach der Antwort
// bleibt er als „Nachgedacht · 7,9 s“ an der Antwort aufklappbar. Nichts davon wird gesprochen,
// gespeichert oder an ein Modell gegeben.

function SchrittText({ s }: { s: DenkSchritt }): React.ReactElement {
  const { t } = useTranslation();
  if (s.art === "text") return <>{s.text}</>;
  if (s.art === "sofort")
    return <>{t("voiceCall.denken.sofort", { modell: s.modell })}</>;
  if (s.art === "auftrag")
    return <>{t("voiceCall.denken.auftrag", { frage: s.frage })}</>;
  if (s.art === "kontext")
    return <>{t("voiceCall.denken.kontext", { count: s.treffer })}</>;
  return (
    <>
      {t([`voiceCall.tool.${s.name}`, "voiceCall.tool.generic"], {
        name: s.name,
      })}
    </>
  );
}

/**
 * Fokusübergabe: Hatte der Knopf der laufenden Anzeige den Fokus, als sie verschwand (die Antwort kam),
 * übernimmt ihn der Knopf des abgeschlossenen Prozesses an der Antwort (gleiches `seit`).
 */
let fokusUebergabe: number | null = null;

/** Wartet eine Fokusübergabe auf diesen Prozess (dann muss er an der Antwort sichtbar sein)? */
export function wartetAufFokus(d: Denkprozess): boolean {
  return fokusUebergabe === d.seit;
}

/** Sekunden seit `seit`, jede halbe Sekunde neu (nur solange die Anzeige läuft). */
function useSekunden(seit: number, laeuft: boolean): number {
  const [jetzt, setJetzt] = useState(() => Date.now());
  useEffect(() => {
    if (!laeuft) return;
    const id = window.setInterval(() => setJetzt(Date.now()), 500);
    return () => window.clearInterval(id);
  }, [laeuft]);
  return Math.max(0, (jetzt - seit) / 1000);
}

export const Denkanzeige: React.FC<{
  denken: Denkprozess;
  /** Läuft noch (vor der ersten Antwort) oder ist abgeschlossen (an der Antwort). */
  laeuft: boolean;
  /** Name des Modells, das antwortet (erster Schritt), falls bekannt. */
  modell?: string | null;
}> = ({ denken, laeuft, modell }) => {
  const { t, i18n } = useTranslation();
  const offen = denken.offen ?? false;
  const denkOffen = useVoiceCallStore((s) => s.denkOffen);
  const knopf = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const seit = denken.seit;
  // Abgeschlossene Anzeige: einen gemerkten Fokus der laufenden übernehmen (sie ist verschwunden).
  useLayoutEffect(() => {
    if (laeuft || fokusUebergabe !== seit) return;
    fokusUebergabe = null;
    const aktiv = document.activeElement;
    if (!aktiv || aktiv === document.body) knopf.current?.focus();
  }, [laeuft, seit]);
  // Laufende Anzeige: Fokus merken, solange er auf dem Knopf liegt. Verschwindet der Knopf mit dem
  // Fokus, bleibt die Merkung; ein echtes Verlassen (Knopf noch da) löscht sie.
  const fokusHandler = laeuft
    ? {
        onFocus: () => {
          fokusUebergabe = seit;
        },
        onBlur: () => {
          const el = knopf.current;
          window.setTimeout(() => {
            if (el?.isConnected && fokusUebergabe === seit)
              fokusUebergabe = null;
          }, 0);
        },
      }
    : {};
  const name = denken.modell ?? modell;
  const sek = useSekunden(denken.seit, laeuft);
  const dauer = (laeuft ? sek : (denken.ms ?? 0) / 1000).toLocaleString(
    i18n.language,
    { maximumFractionDigits: 1, minimumFractionDigits: laeuft ? 0 : 1 },
  );
  const label = laeuft
    ? t("voiceCall.denken.laeuft")
    : t("voiceCall.denken.fertig", { dauer });
  const Pfeil = offen ? ChevronDown : ChevronRight;
  return (
    <div
      className={
        laeuft ? "self-start flex flex-col gap-1 pl-9" : "flex flex-col gap-1"
      }
      data-testid={laeuft ? "denkanzeige" : "denkprozess"}
    >
      <button
        ref={knopf}
        type="button"
        aria-expanded={offen}
        aria-controls={listId}
        onClick={() => denkOffen(seit, !offen)}
        {...fokusHandler}
        className="self-start inline-flex items-center gap-1 min-h-7 -mx-1 px-1 rounded text-xs text-text/70 hover:text-text hover:bg-mid-gray/10 cursor-pointer"
      >
        <span
          className={laeuft ? "lv-denken-schimmer" : undefined}
          data-testid="denkanzeige-label"
        >
          {label}
        </span>
        {laeuft && (
          <span className="tabular-nums text-text/70" aria-hidden="true">
            {t("voiceCall.seconds", {
              value: Math.floor(sek).toLocaleString(i18n.language),
            })}
          </span>
        )}
        <Pfeil className="w-3.5 h-3.5" aria-hidden="true" />
      </button>
      {offen && (
        <ol
          id={listId}
          aria-label={t("voiceCall.denken.titel")}
          data-testid="denkprozess-liste"
          className="m-0 ml-1 pl-3 border-l border-mid-gray/30 list-none flex flex-col gap-1 text-xs text-text/70"
        >
          {name && <li>{t("voiceCall.denken.modell", { modell: name })}</li>}
          {denken.schritte.map((s, i) => (
            <li key={i}>
              <SchrittText s={s} />
            </li>
          ))}
          {denken.gedanken && (
            <li
              data-testid="denkprozess-gedanken"
              className="whitespace-pre-wrap break-words max-h-64 overflow-y-auto"
            >
              {denken.gedanken}
            </li>
          )}
          {laeuft && (
            <li className="lv-denken-schimmer">
              {t("voiceCall.denken.arbeitet")}
            </li>
          )}
          {!laeuft &&
            denken.schritte.length === 0 &&
            !denken.gedanken &&
            !name && <li>{t("voiceCall.denken.keineSchritte")}</li>}
        </ol>
      )}
    </div>
  );
};
