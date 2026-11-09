import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox, Radio, RadioGroup } from "@fluentui/react-components";
import {
  commands,
  type AnmeldeWahl,
  type Entscheidung,
  type SyncStatus,
} from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { formatBytes } from "@/lib/library/remove";

interface Props {
  entscheidung: Entscheidung | null;
  /** Nach einer Wahl (auch nach Abbrechen) mit dem neuen Stand. */
  onErledigt: (status: SyncStatus, wahl: AnmeldeWahl | "abbrechen") => void;
}

type Weg = "zusammenfuehren" | "sichern";

/**
 * Modal „Lokale Daten gefunden“ (Spec 2026-11-08, U3). Esc und Hintergrund
 * schließen nicht: Erst eine Wahl oder „Abbrechen“ beendet die wartende
 * Anmeldung. Jede Wahl außer Abbrechen sichert zuerst vollständig und prüft
 * die Sicherung, bevor etwas verändert wird.
 */
export const AnmeldungDatenDialog: React.FC<Props> = ({
  entscheidung,
  onErledigt,
}) => {
  const { t, i18n } = useTranslation();
  const kontowechsel = !!entscheidung?.vorher_konto;
  const [weg, setWeg] = useState<Weg>("zusammenfuehren");
  const [trennung, setTrennung] = useState<"getrennt_halten" | "entfernen">(
    "getrennt_halten",
  );
  const [kopieren, setKopieren] = useState(false);
  const [laeuft, setLaeuft] = useState<null | "wahl" | "abbrechen">(null);
  const [fehler, setFehler] = useState("");

  // Nur beim Öffnen (oder bei anderem Vorkonto) zurücksetzen, nicht bei jedem neuen
  // Stand aus dem Kern: sonst gingen Auswahl und laufende Wahl verloren.
  const offen = entscheidung !== null;
  const vorher = entscheidung?.vorher_konto ?? null;
  useEffect(() => {
    if (!offen) return;
    // Kontowechsel: der vorsichtige Weg ist vorausgewählt (U3).
    setWeg(vorher ? "sichern" : "zusammenfuehren");
    setTrennung("getrennt_halten");
    setKopieren(false);
    setFehler("");
    setLaeuft(null);
  }, [offen, vorher]);

  if (!entscheidung) return null;

  const wahl: AnmeldeWahl =
    weg === "zusammenfuehren" ? "zusammenfuehren" : trennung;
  const gesperrt = weg === "zusammenfuehren" && kontowechsel && !kopieren;

  const bestaetigen = async () => {
    if (laeuft || gesperrt) return;
    setLaeuft("wahl");
    setFehler("");
    const r = await commands.syncAnmeldungEntscheiden(wahl, kopieren);
    setLaeuft(null);
    if (r.status === "error") {
      setFehler(r.error);
      return;
    }
    onErledigt(r.data, wahl);
  };

  const abbrechen = async () => {
    if (laeuft) return;
    setLaeuft("abbrechen");
    setFehler("");
    const r = await commands.syncAnmeldungAbbrechen();
    setLaeuft(null);
    if (r.status === "error") {
      setFehler(r.error);
      return;
    }
    onErledigt(r.data, "abbrechen");
  };

  const zeilen: { key: string; text: string }[] = [
    {
      key: "seiten",
      text: t("sync.anmeldung.seiten", { count: entscheidung.seiten }),
    },
    {
      key: "buecher",
      text: t("sync.anmeldung.buecher", { count: entscheidung.buecher }),
    },
    {
      key: "fortschritte",
      text: t("sync.anmeldung.fortschritte", {
        count: entscheidung.fortschritte,
      }),
    },
    {
      key: "dateien",
      text: t("sync.anmeldung.dateien", { count: entscheidung.dateien }),
    },
  ].filter(
    (_, i) =>
      [
        entscheidung.seiten,
        entscheidung.buecher,
        entscheidung.fortschritte,
        entscheidung.dateien,
      ][i] > 0,
  );
  if (entscheidung.einstellungen) {
    zeilen.push({
      key: "einstellungen",
      text: t("sync.anmeldung.einstellungen"),
    });
  }

  const knopf =
    wahl === "zusammenfuehren"
      ? t("sync.anmeldung.knopfZusammenfuehren")
      : wahl === "getrennt_halten"
        ? t("sync.anmeldung.knopfGetrennt")
        : t("sync.anmeldung.knopfEntfernen");

  return (
    <Dialog
      open={true}
      onOpenChange={() => {}}
      dismissible={false}
      title={
        kontowechsel
          ? t("sync.anmeldung.titelWechsel")
          : t("sync.anmeldung.titel")
      }
      closeLabel={t("sync.anmeldung.abbrechen")}
      className="max-w-xl"
      description={
        kontowechsel
          ? t("sync.anmeldung.beschreibungWechsel", {
              vorher: entscheidung.vorher_konto,
              konto: entscheidung.konto,
            })
          : t("sync.anmeldung.beschreibung", { konto: entscheidung.konto })
      }
      footer={
        <>
          <Button
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            variant="primary"
            onClick={() => void bestaetigen()}
            disabled={!!laeuft || gesperrt}
            data-testid="anmeldung-bestaetigen"
          >
            {laeuft === "wahl" ? t("sync.anmeldung.laeuft") : knopf}
          </Button>
          <Button
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            variant="secondary"
            onClick={() => void abbrechen()}
            disabled={!!laeuft}
            data-testid="anmeldung-abbrechen"
          >
            {t("sync.anmeldung.abbrechen")}
          </Button>
        </>
      }
    >
      <div className="space-y-4 text-sm" data-testid="anmeldung-daten">
        <div className="rounded-md border border-mid-gray/20 px-3 py-2">
          <ul className="space-y-0.5" aria-label={t("sync.anmeldung.bestand")}>
            {zeilen.map((z) => (
              <li key={z.key}>{z.text}</li>
            ))}
          </ul>
          <p className="mt-1 text-text-muted">
            {t("sync.anmeldung.groesse", {
              groesse: formatBytes(entscheidung.sicherung_bytes, i18n.language),
            })}
          </p>
        </div>

        <RadioGroup
          value={weg}
          onChange={(_, d) => {
            if (d.value === "zusammenfuehren" || d.value === "sichern")
              setWeg(d.value);
          }}
          aria-label={t("sync.anmeldung.frage")}
        >
          <Radio
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            value="zusammenfuehren"
            label={
              <span>
                <span className="font-medium">
                  {t("sync.anmeldung.zusammenfuehren")}
                </span>
                <span className="block text-text-muted">
                  {t("sync.anmeldung.zusammenfuehrenText")}
                </span>
              </span>
            }
            data-testid="wahl-zusammenfuehren"
          />
          {weg === "zusammenfuehren" && kontowechsel && (
            <div className="ms-8 mb-1">
              <Checkbox
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                checked={kopieren}
                onChange={(_, d) => setKopieren(!!d.checked)}
                label={t("sync.anmeldung.kopierenBestaetigen", {
                  vorher: entscheidung.vorher_konto,
                  konto: entscheidung.konto,
                })}
                data-testid="wahl-kopieren"
              />
            </div>
          )}
          <Radio
            className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
            value="sichern"
            label={
              <span>
                <span className="font-medium">
                  {t("sync.anmeldung.sichern")}
                </span>
                <span className="block text-text-muted">
                  {t("sync.anmeldung.sichernText")}
                </span>
              </span>
            }
            data-testid="wahl-sichern"
          />
        </RadioGroup>
        {weg === "sichern" && (
          <RadioGroup
            className="ms-8 -mt-3"
            value={trennung}
            onChange={(_, d) =>
              setTrennung(d.value as "getrennt_halten" | "entfernen")
            }
            aria-label={t("sync.anmeldung.sichern")}
          >
            <Radio
              className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
              value="getrennt_halten"
              label={
                <span>
                  <span>{t("sync.anmeldung.getrennt")}</span>
                  <span className="block text-text-muted">
                    {t("sync.anmeldung.getrenntText")}
                  </span>
                </span>
              }
              data-testid="wahl-getrennt"
            />
            <Radio
              className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
              value="entfernen"
              label={
                <span>
                  <span>{t("sync.anmeldung.entfernen")}</span>
                  <span className="block text-text-muted">
                    {t("sync.anmeldung.entfernenText")}
                  </span>
                </span>
              }
              data-testid="wahl-entfernen"
            />
          </RadioGroup>
        )}

        {wahl === "getrennt_halten" && entscheidung.kollisionen.length > 0 && (
          <div data-testid="anmeldung-kollisionen">
            <p className="font-medium">
              {t("sync.anmeldung.kollisionen", {
                count: entscheidung.kollisionen.length,
              })}
            </p>
            <ul className="mt-1 space-y-0.5 text-text-muted">
              {entscheidung.kollisionen.map((k, i) => (
                <li key={`${k.art}-${i}`}>
                  {k.art === "buch"
                    ? t("sync.anmeldung.kollisionBuch", { titel: k.titel })
                    : t("sync.anmeldung.kollisionSeite", { titel: k.titel })}
                </li>
              ))}
            </ul>
          </div>
        )}

        <p className="text-text-muted" data-testid="anmeldung-unberuehrt">
          {t("sync.anmeldung.unberuehrt")}
        </p>
        {fehler && (
          <p className="text-red-500" role="alert">
            {fehler}
          </p>
        )}
      </div>
    </Dialog>
  );
};
