import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import type { Aufbereitung, Dokument } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import {
  anzeigeText,
  ergebnisText,
  fenster,
  gruppenUebersicht,
  strukturVergleich,
  uebernahmeVerwerfen,
  vergleichSegmente,
} from "@/lib/reader/aufbereiten";

interface AufbereitenDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  dokument: Dokument;
  text: string;
  onApply(neuerText: string): void;
  readOnly?: boolean;
}

const REGELN = [
  "nfc",
  "ligaturen",
  "weiche_trennstriche",
  "anfuehrungszeichen",
  "gedankenstrich",
  "auslassung",
  "leerzeichen",
  "listen",
] as const;
const FEHLER = [
  "text_too_large",
  "unknown_rule",
  "unsupported_document",
  "wrong_status",
  "invalid_changes",
  "storage",
] as const;
const PRO_SEITE = 20;
const TEXT_SEITE = 8000;

interface AktuellerEditor {
  aktiv: boolean;
  text: string;
  readOnly: boolean;
  dokument: Dokument;
  onApply(neuerText: string): void;
}

const AnzeigeText: React.FC<{ text: string }> = ({ text }) => {
  const { t } = useTranslation();
  const anzeige = anzeigeText(text);
  const hinweis = t("tts.aufbereiten.truncated", { length: anzeige.laenge });
  return (
    <span title={anzeige.gekuerzt ? hinweis : undefined}>
      {anzeige.text}
      {anzeige.gekuerzt && <span className="text-text-muted"> ({hinweis})</span>}
    </span>
  );
};

/** Jede Öffnung beginnt mit allen Regeln und einem eigenen Entwurf. */
export const AufbereitenDialog: React.FC<AufbereitenDialogProps> = (props) => {
  // Die Toast-Aktion lebt länger als die geöffnete Sitzung.
  const aktuell = useRef<AktuellerEditor>({
    ...props,
    aktiv: true,
    readOnly: props.readOnly ?? false,
  });
  aktuell.current = {
    ...props,
    aktiv: true,
    readOnly: props.readOnly ?? false,
  };
  useEffect(() => {
    aktuell.current.aktiv = true;
    return () => {
      aktuell.current.aktiv = false;
    };
  }, []);
  return props.open ? (
    <AufbereitenSitzung
      key={JSON.stringify(props.dokument)}
      {...props}
      aktuell={aktuell}
    />
  ) : null;
};

const AufbereitenSitzung: React.FC<
  AufbereitenDialogProps & {
    aktuell: React.MutableRefObject<AktuellerEditor>;
  }
> = ({
  open,
  onOpenChange,
  dokument,
  onApply,
  readOnly = false,
  aktuell,
}) => {
  const { t } = useTranslation();
  const [regeln, setRegeln] = useState<string[]>([...REGELN]);
  const [entwurf, setEntwurf] = useState<Aufbereitung | null>(null);
  const [basis, setBasis] = useState("");
  const [bereiche, setBereiche] = useState<[number, number][]>([]);
  const [original, setOriginal] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const [veraltet, setVeraltet] = useState(false);
  const [revision, setRevision] = useState(0);
  const [ansicht, setAnsicht] = useState<
    "vergleich" | "vorher" | "nachher"
  >("vergleich");
  const [seite, setSeite] = useState(0);
  const sperre = useRef(false);
  const lebt = useRef(true);
  const dokumentRef = useRef(dokument);

  useEffect(() => {
    lebt.current = true;
    return () => {
      lebt.current = false;
    };
  }, []);

  useEffect(() => {
    let aktiv = true;
    setOriginal(null);
    void commands.aufbereitungOriginal(dokumentRef.current)
      .then((result) => {
        if (!aktiv) return;
        if (result.status === "ok") setOriginal(result.data);
        else setFehler(result.error);
      })
      .catch(() => {
        if (aktiv) setFehler("general");
      });
    return () => {
      aktiv = false;
    };
  }, [revision]);

  // Der Basistext bleibt bis zur Neuberechnung unverändert. Spätere Eingaben
  // werden erst beim Übernehmen an das Backend zur Prüfung gegeben.
  useEffect(() => {
    let aktiv = true;
    const berechnen = async () => {
      sperre.current = true;
      setBusy(true);
      setFehler(null);
      setEntwurf(null);
      setBereiche([]);
      setVeraltet(false);
      setSeite(0);
      const vorher = aktuell.current.text;
      try {
        const result = await commands.aufbereitungEntwurfAnlegen(
          dokumentRef.current,
          vorher,
          regeln,
        );
        if (!aktiv) return;
        if (result.status === "error") {
          setFehler(result.error);
          return;
        }
        const grenzen = await commands.aufbereitungUtf16Bereiche(
          result.data.id,
          dokumentRef.current,
        );
        if (!aktiv) return;
        if (grenzen.status === "error") {
          setFehler(grenzen.error);
          return;
        }
        setBasis(vorher);
        setBereiche(grenzen.data);
        setEntwurf(result.data);
        setVeraltet(result.data.status === "veraltet");
      } catch {
        if (aktiv) setFehler("general");
      } finally {
        if (aktiv) {
          sperre.current = false;
          setBusy(false);
        }
      }
    };
    void berechnen();
    return () => {
      aktiv = false;
    };
  }, [aktuell, regeln, revision]);

  const aenderungen = entwurf?.aenderungen;
  const gruppen = useMemo(
    () => gruppenUebersicht(aenderungen ?? [], basis, bereiche),
    [aenderungen, basis, bereiche],
  );
  const segmente = useMemo(
    () => vergleichSegmente(basis, aenderungen ?? [], bereiche),
    [basis, aenderungen, bereiche],
  );
  const nachher = useMemo(
    () => ergebnisText(basis, aenderungen ?? [], bereiche, "angenommen"),
    [basis, aenderungen, bereiche],
  );
  const struktur = useMemo(
    () => strukturVergleich(basis, nachher),
    [basis, nachher],
  );
  const vorschau = ansicht === "vorher" ? basis : nachher;
  const seiten = Math.max(
    1,
    Math.ceil(
      ansicht === "vergleich"
        ? segmente.filter((s) => s.art === "aenderung").length / PRO_SEITE
        : vorschau.length / TEXT_SEITE,
    ),
  );
  const aktuelleSeite = Math.min(seite, seiten - 1);
  const gesperrt = busy || readOnly;

  const fehlermeldung = (code: string) => {
    const bekannt = FEHLER.find((wert) => wert === code);
    return t(`tts.aufbereiten.errors.${bekannt ?? "general"}`);
  };

  const entscheiden = async (
    entscheidung: "angenommen" | "abgelehnt",
    regel?: string,
  ) => {
    if (sperre.current || readOnly || !entwurf || veraltet) return;
    sperre.current = true;
    setBusy(true);
    setFehler(null);
    try {
      const result = await commands.aufbereitungEntscheidenRegel(
        dokument,
        entwurf.id,
        regel ?? null,
        entscheidung,
      );
      if (!lebt.current) return;
      if (result.status === "error") {
        setFehler(result.error);
        return;
      }
      setEntwurf(result.data);
    } catch {
      if (lebt.current) setFehler("general");
    } finally {
      sperre.current = false;
      if (lebt.current) setBusy(false);
    }
  };

  const uebernehmen = async () => {
    if (sperre.current || readOnly || !entwurf || veraltet) return;
    sperre.current = true;
    setBusy(true);
    setFehler(null);
    const vorher = aktuell.current.text;
    try {
      const result = await commands.aufbereitungUebernehmen(
        dokument,
        entwurf.id,
        vorher,
      );
      if (result.status === "error") {
        if (lebt.current) setFehler(result.error);
        return;
      }
      const neuerText = result.data.text;
      if (
        !lebt.current ||
        !aktuell.current.aktiv ||
        aktuell.current.text !== vorher ||
        aktuell.current.readOnly ||
        JSON.stringify(aktuell.current.dokument) !== JSON.stringify(dokument)
      ) {
        await uebernahmeVerwerfen(result.data, true, async () => {
          try {
            // Nur die Aktivierung zurücknehmen, den Antworttext niemals anwenden.
            const rueck = await commands.aufbereitungRueckgaengig(
              dokument,
              entwurf.id,
            );
            if (rueck.status === "error" || rueck.data.warnung) {
              toast(t("tts.aufbereiten.stale"));
            }
          } catch {
            toast(t("tts.aufbereiten.stale"));
          }
        });
        if (lebt.current) setVeraltet(true);
        else toast(t("tts.aufbereiten.stale"));
        return;
      }
      if (neuerText === null || result.data.aufbereitung.status === "veraltet") {
        setVeraltet(true);
        return;
      }
      onApply(neuerText);
      onOpenChange(false);
      if (result.data.warnung) toast(t("tts.aufbereiten.partialWarning"));
      toast(t("tts.aufbereiten.applied"), {
        action: {
          label: t("tts.aufbereiten.undo"),
          onClick: () => {
            void (async () => {
              if (!aktuell.current.aktiv || aktuell.current.readOnly ||
                aktuell.current.text !== neuerText ||
                JSON.stringify(aktuell.current.dokument) !==
                  JSON.stringify(dokument)) {
                toast(t("tts.aufbereiten.stale"));
                return;
              }
              try {
                const rueck = await commands.aufbereitungRueckgaengig(
                  dokument,
                  entwurf.id,
                );
                if (rueck.status === "error") {
                  toast.error(fehlermeldung(rueck.error));
                } else if (aktuell.current.aktiv &&
                  !aktuell.current.readOnly &&
                  aktuell.current.text === neuerText &&
                  JSON.stringify(aktuell.current.dokument) ===
                    JSON.stringify(dokument)) {
                  aktuell.current.onApply(rueck.data.text);
                  if (rueck.data.warnung) {
                    toast(t("tts.aufbereiten.partialWarning"));
                  }
                } else {
                  // Das Backend hat bereits zurückgenommen. Erneutes Übernehmen wäre
                  // bei Handänderungen riskant; die abgelöste Fassung bleibt im Verlauf.
                  toast(t("tts.aufbereiten.stale"));
                  toast(t("tts.aufbereiten.savedInHistory"));
                }
              } catch {
                toast.error(fehlermeldung("general"));
              }
            })();
          },
        },
      });
    } catch {
      if (lebt.current) setFehler("general");
    } finally {
      sperre.current = false;
      if (lebt.current) setBusy(false);
    }
  };

  const wiederherstellen = async () => {
    if (sperre.current || readOnly || original === null) return;
    sperre.current = true;
    setBusy(true);
    const vorher = aktuell.current.text;
    try {
      const result = await commands.aufbereitungOriginal(dokument);
      if (!lebt.current) return;
      if (result.status === "error") setFehler(result.error);
      else if (aktuell.current.text !== vorher || aktuell.current.readOnly) {
        setVeraltet(true);
      } else if (result.data !== null) {
        onApply(result.data);
        onOpenChange(false);
      } else setOriginal(null);
    } catch {
      if (lebt.current) setFehler("general");
    } finally {
      sperre.current = false;
      if (lebt.current) setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.editorMenu.aufbereiten")}
      description={t("tts.editorMenu.aufbereitenHint")}
      closeLabel={t("tts.aufbereiten.close")}
      className="w-full h-full max-w-4xl sm:h-auto"
      contentClassName="overflow-y-auto sm:max-h-[65vh]"
      footer={
        <div className="flex w-full flex-wrap gap-2">
          <Button
            className="min-h-[44px]"
            variant="primary"
            disabled={gesperrt || !entwurf || veraltet ||
              entwurf.status !== "entwurf"}
            data-testid="aufbereiten-uebernehmen"
            onClick={() => void uebernehmen()}
          >
            {t("tts.aufbereiten.apply")}
          </Button>
          {original !== null && (
            <Button
              className="min-h-[44px]"
              variant="secondary"
              disabled={gesperrt}
              data-testid="aufbereiten-original"
              onClick={() => void wiederherstellen()}
            >
              {t("tts.aufbereiten.original")}
            </Button>
          )}
          <Button
            className="min-h-[44px]"
            variant="secondary"
            onClick={() => onOpenChange(false)}
          >
            {t("tts.aufbereiten.close")}
          </Button>
        </div>
      }
    >
      <div className="flex flex-col gap-3" data-testid="aufbereiten-dialog">
        <fieldset disabled={gesperrt}>
          <legend className="text-sm font-medium">
            {t("tts.aufbereiten.rulesLabel")}
          </legend>
          <div className="flex flex-wrap gap-x-4">
            {REGELN.map((regel) => (
              <label key={regel} className="flex min-h-[44px] items-center gap-2">
                <input
                  type="checkbox"
                  checked={regeln.includes(regel)}
                  data-testid={`aufbereiten-regel-${regel}`}
                  onChange={(e) => {
                    const an = e.target.checked;
                    setRegeln((alt) => an
                      ? [...alt, regel]
                      : alt.filter((r) => r !== regel));
                  }}
                />
                {t(`tts.aufbereiten.rules.${regel}`)}
              </label>
            ))}
          </div>
        </fieldset>
        {busy && <p role="status">{t("tts.aufbereiten.busy")}</p>}
        {fehler && (
          <p role="alert" className="text-sm">
            {fehlermeldung(fehler)}
          </p>
        )}
        {(veraltet || fehler !== null) && (
          <div data-testid={veraltet ? "aufbereiten-veraltet" : undefined}>
            {veraltet && <p role="status">{t("tts.aufbereiten.stale")}</p>}
            <Button
              className="min-h-[44px]"
              variant="secondary"
              disabled={gesperrt}
              data-testid="aufbereiten-neu-berechnen"
              onClick={() => setRevision((n) => n + 1)}
            >
              {t("tts.aufbereiten.recalculate")}
            </Button>
          </div>
        )}
        {entwurf && (
          <>
            <p className="text-sm text-text-muted">
              {t("tts.aufbereiten.structure", {
                paragraphsBefore: struktur.absaetze.vorher,
                paragraphsAfter: struktur.absaetze.nachher,
                headingsBefore: struktur.ueberschriften.vorher,
                headingsAfter: struktur.ueberschriften.nachher,
                pagesBefore: struktur.seitenmarken.vorher,
                pagesAfter: struktur.seitenmarken.nachher,
              })}
            </p>
            {aenderungen?.length === 0 && (
              <p data-testid="aufbereiten-leer">
                {t("tts.aufbereiten.empty")}
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              {(["angenommen", "abgelehnt"] as const).map((status) => (
                <Button
                  key={status}
                  className="min-h-[44px]"
                  variant="secondary"
                  disabled={gesperrt || veraltet}
                  data-testid={status === "angenommen"
                    ? "aufbereiten-alle-annehmen"
                    : "aufbereiten-alle-ablehnen"}
                  onClick={() => void entscheiden(status)}
                >
                  {t(status === "angenommen"
                    ? "tts.aufbereiten.acceptAll"
                    : "tts.aufbereiten.rejectAll")}
                </Button>
              ))}
            </div>
            <ul className="flex flex-col gap-3">
              {gruppen.map((gruppe) => (
                <li
                  key={gruppe.regel}
                  className="rounded-md border border-mid-gray/20 p-2"
                  data-testid={`aufbereiten-gruppe-${gruppe.regel}`}
                >
                  <p className="text-sm font-medium">
                    {t(`tts.aufbereiten.rules.${gruppe.regel}`)}{" · "}
                    {t("tts.aufbereiten.counts", {
                      changes: gruppe.anzahl,
                      groups: gruppe.gruppen,
                    })}{" · "}
                    {t(`tts.aufbereiten.status.${gruppe.status}`)}
                  </p>
                  <p
                    className="break-words text-sm"
                    title={
                      gruppe.beispielGekuerzt
                        ? t("tts.aufbereiten.truncated", {
                            length: gruppe.beispielLaenge,
                          })
                        : undefined
                    }
                  >
                    {gruppe.beispiel}
                    {gruppe.beispielGekuerzt && (
                      <span className="text-text-muted">
                        {" "}(
                        {t("tts.aufbereiten.truncated", {
                          length: gruppe.beispielLaenge,
                        })}
                        )
                      </span>
                    )}
                  </p>
                  <div className="flex flex-wrap gap-2">
                    {(["angenommen", "abgelehnt"] as const).map((status) => (
                      <Button
                        key={status}
                        className="min-h-[44px]"
                        variant="secondary"
                        disabled={gesperrt || veraltet}
                        data-testid={`aufbereiten-${status === "angenommen"
                          ? "annehmen" : "ablehnen"}-${gruppe.regel}`}
                        onClick={() => void entscheiden(status, gruppe.regel)}
                      >
                        {t(status === "angenommen"
                          ? "tts.aufbereiten.accept"
                          : "tts.aufbereiten.reject")}
                      </Button>
                    ))}
                  </div>
                </li>
              ))}
            </ul>
            <div className="flex flex-wrap gap-2">
              {(["vergleich", "vorher", "nachher"] as const).map((wert) => (
                <Button
                  key={wert}
                  className="min-h-[44px]"
                  variant="secondary"
                  aria-pressed={ansicht === wert}
                  data-testid={`aufbereiten-ansicht-${wert}`}
                  onClick={() => {
                    setAnsicht(wert);
                    setSeite(0);
                  }}
                >
                  {t(`tts.aufbereiten.views.${wert}`)}
                </Button>
              ))}
            </div>
            <div className="whitespace-pre-wrap break-words text-sm">
              {ansicht !== "vergleich"
                ? vorschau.slice(
                    aktuelleSeite * TEXT_SEITE,
                    (aktuelleSeite + 1) * TEXT_SEITE,
                  )
                : fenster(segmente, aktuelleSeite, PRO_SEITE).map((s, i) => (
                    <span key={i}>
                      {s.art === "gleich" ? (
                        // Auch ein einzelner Kontextblock kann 1 MB groß sein.
                        s.text.length > 500
                          ? `${s.text.slice(0, 250)} … ${s.text.slice(-250)}`
                          : s.text
                      ) : (
                        <>
                          <span className="sr-only">
                            {t(`tts.aufbereiten.status.${s.status ?? "offen"}`)}
                          </span>
                          <del className="text-red-700 dark:text-red-300">
                            <span className="sr-only">
                              {t("tts.aufbereiten.removed")}{" "}
                            </span>
                            <AnzeigeText text={s.alt ?? ""} />
                          </del>
                          <ins className="text-green-700 dark:text-green-300">
                            <span className="sr-only">
                              {t("tts.aufbereiten.inserted")}{" "}
                            </span>
                            <AnzeigeText text={s.neu ?? ""} />
                          </ins>
                        </>
                      )}
                    </span>
                  ))}
            </div>
            <div className="flex items-center gap-2">
              <Button
                className="min-h-[44px]"
                variant="secondary"
                disabled={aktuelleSeite === 0}
                onClick={() => setSeite(aktuelleSeite - 1)}
              >
                {t("tts.aufbereiten.previous")}
              </Button>
              <span role="status">
                {t("tts.aufbereiten.page", {
                  page: aktuelleSeite + 1,
                  total: seiten,
                })}
              </span>
              <Button
                className="min-h-[44px]"
                variant="secondary"
                disabled={aktuelleSeite + 1 >= seiten}
                onClick={() => setSeite(aktuelleSeite + 1)}
              >
                {t("tts.aufbereiten.next")}
              </Button>
            </div>
          </>
        )}
      </div>
    </Dialog>
  );
};
