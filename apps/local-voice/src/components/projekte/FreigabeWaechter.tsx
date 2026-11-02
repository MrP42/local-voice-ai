import "./styles";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type FreigabeAktion, type FreigabeInfo } from "@/bindings";
import {
  auspacken,
  fehlerText,
  freigabeAnfragen,
  freigabeKontext,
  freigabeRestzeit,
  laufText,
} from "@/lib/projekte";
import {
  meldeHaltEnde,
  meldeProjekte,
  useProjekteMeldung,
} from "@/lib/projekteBus";
import { RechercheFreigabe } from "../../vendor/wai-ui/projekt";
import { useFreigabeLabels } from "./labels";

/** Wie oft nach abgelaufenen Halten gesehen wird, solange einer offen ist. */
const PRUEFEN_MS = 20_000;

/**
 * Wächter der Recherche-Freigabe (ADR-0026): zeigt den Dialog, sobald eine Recherche ab der Klasse
 * PERSONAL angehalten wurde. Gesendet wird nur, was die Person ankreuzt; "Abbrechen" sendet nichts
 * ("Nur im Projekt suchen" gibt es erst mit dem Projektindex, PR-9, und hat hier keinen Knopf). Der
 * Dialog nennt die Frage der Recherche, die Datenklasse und die Restzeit des Halts. Einmal pro App
 * einbinden (`App.tsx`); Chat-Spalte, Sprachgespräch und Projektseite melden über
 * `meldeProjekte("freigabe")`.
 *
 * Robustheit: Ein Lesefehler lässt den alten Stand stehen (der Dialog schließt sich nicht, die Person
 * bekommt eine Meldung); während eine Entscheidung läuft, wird nicht gelesen; ein Sperr-Ref hält
 * Doppelklicks ab; ein Fehler gilt nur für den Halt, bei dem er entstand.
 */
export const FreigabeWaechter: React.FC = () => {
  const { t } = useTranslation();
  const labels = useFreigabeLabels();
  const [offen, setOffen] = useState<FreigabeInfo[]>([]);
  const [geladenAm, setGeladenAm] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const [fehler, setFehler] = useState<string | undefined>();
  const sperre = useRef(false);
  const ladeFehlerGemeldet = useRef(false);
  const vorher = useRef<Set<string>>(new Set());
  const entschieden = useRef<Set<string>>(new Set());

  const laden = useCallback(async () => {
    if (sperre.current) return;
    try {
      const liste = auspacken(await commands.rechercheFreigaben());
      ladeFehlerGemeldet.current = false;
      // Ein Halt, der ohne unsere Entscheidung verschwand, ist abgelaufen: Wartende erfahren es.
      const jetzt = new Set(liste.map((h) => h.id));
      for (const id of vorher.current) {
        if (!jetzt.has(id) && !entschieden.current.delete(id)) {
          meldeHaltEnde(id, "abgelaufen");
        }
      }
      vorher.current = jetzt;
      setGeladenAm(Date.now());
      setOffen(liste);
    } catch (e) {
      // Alter Stand bleibt; einmal melden, nicht bei jedem Takt.
      if (!ladeFehlerGemeldet.current) {
        ladeFehlerGemeldet.current = true;
        toast.error(t("projekte.freigabe.ladenFehler"), {
          description: fehlerText(e, t),
        });
      }
    }
  }, [t]);

  useEffect(() => {
    void laden();
  }, [laden]);
  useProjekteMeldung("freigabe", () => void laden());
  useEffect(() => {
    if (offen.length === 0) return;
    const id = window.setInterval(() => void laden(), PRUEFEN_MS);
    return () => window.clearInterval(id);
  }, [offen.length, laden]);

  const aktuell = offen[0];
  const aktuellId = aktuell?.id;
  useEffect(() => {
    setFehler(undefined);
  }, [aktuellId]);

  const entscheiden = async (
    aktion: FreigabeAktion,
    ids: readonly string[],
  ) => {
    if (!aktuell || sperre.current) return;
    const haltId = aktuell.id;
    sperre.current = true;
    setBusy(true);
    setFehler(undefined);
    try {
      const a = auspacken(
        await commands.rechercheFreigabeEntscheiden(haltId, [...ids], aktion),
      );
      if (a.status === "fehler") {
        // Der Halt besteht weiter (Regeln verletzt) oder ist weg: dann steht der Text im Dialog bzw.
        // als Meldung, falls der Dialog nicht mehr offen ist.
        const text = laufText(a, t);
        if (a.haltBleibt) setFehler(text);
        else {
          entschieden.current.add(haltId);
          meldeHaltEnde(haltId, "fehler");
          toast.error(text);
        }
      } else if (a.status === "fertig" && a.rechercheId) {
        entschieden.current.add(haltId);
        toast.success(t("projekte.freigabe.fertig"));
        meldeProjekte("recherche", a.rechercheId);
        meldeProjekte("projekte");
      } else if (a.status === "abgelehnt") {
        entschieden.current.add(haltId);
        meldeHaltEnde(haltId, "abgelehnt");
        toast.info(t("projekte.freigabe.nichtsGesendet"));
      }
    } catch (e) {
      const text = fehlerText(e, t);
      // Ohne offenen Dialog (Halt inzwischen weg) gibt es keine Stelle im Dialog: Meldung.
      if (offen.some((h) => h.id === haltId)) setFehler(text);
      else toast.error(text);
      if (/recherche\.not_found/.test(String(e))) {
        entschieden.current.add(haltId);
        meldeHaltEnde(haltId, "abgelaufen");
      }
    } finally {
      sperre.current = false;
      setBusy(false);
      await laden();
    }
  };

  const rest = aktuell
    ? freigabeRestzeit(aktuell, geladenAm, Date.now(), t)
    : undefined;
  const kontext = aktuell
    ? [freigabeKontext(aktuell, t), rest].filter(Boolean).join("; ")
    : "";

  return (
    <RechercheFreigabe
      key={aktuellId ?? "keine"}
      isOpen={aktuell !== undefined}
      onOpenChange={() => undefined}
      contextLabel={kontext}
      queries={aktuell ? freigabeAnfragen(aktuell, t) : []}
      busy={busy}
      error={fehler}
      labels={labels}
      onSend={(ids) => void entscheiden("senden", ids)}
      onCancel={() => {
        if (!sperre.current) void entscheiden("abbrechen", []);
      }}
    />
  );
};
