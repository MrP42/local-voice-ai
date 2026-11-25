import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type FollowupMode,
  type ProtokollEntwurf as Entwurf,
} from "@/bindings";
import { emlFileName, parseAddresses } from "@/lib/meetingFollowup";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Textarea } from "../ui/Textarea";
import { api } from "./api";
import { fehlerMeldung, protokollFehlerSchluessel } from "./protokoll";

type Meldung = { art: "ok" | "hinweis" | "fehler"; text: string };

interface Props {
  open: boolean;
  onOpenChange: (offen: boolean) => void;
  planId: string;
}

/**
 * Entwurf an die Teilnehmenden (lva#736, P5): Empfänger, Betreff, Klassifizierung und Text kommen
 * aus der freigegebenen Fassung des Protokolls und sind hier noch änderbar. Ausgabe wie beim
 * Follow-up: Kopieren, Mailprogramm (`mailto:`), `.eml`-Datei. Es wird nie automatisch versendet.
 */
export const ProtokollEntwurf: React.FC<Props> = ({
  open,
  onOpenChange,
  planId,
}) => {
  const { t } = useTranslation();
  const [entwurf, setEntwurf] = useState<Entwurf | null>(null);
  const [fehler, setFehler] = useState<string | null>(null);
  const [an, setAn] = useState("");
  const [betreff, setBetreff] = useState("");
  const [text, setText] = useState("");
  const [laeuft, setLaeuft] = useState(false);
  const [meldung, setMeldung] = useState<Meldung | null>(null);
  // Schließen oder Neustart: eine spät eintreffende Antwort verwerfen.
  const abruf = useRef(0);

  useEffect(() => {
    if (!open) {
      abruf.current++;
      return;
    }
    const mein = ++abruf.current;
    setEntwurf(null);
    setFehler(null);
    setMeldung(null);
    api.protokollEntwurf(planId).then(
      (e) => {
        if (mein !== abruf.current) return;
        setEntwurf(e);
        setAn(e.entwurf.to.join(", "));
        setBetreff(e.entwurf.subject);
        setText(e.entwurf.body_text);
      },
      (e: unknown) => {
        if (mein !== abruf.current) return;
        setFehler(t(protokollFehlerSchluessel(fehlerMeldung(e))));
      },
    );
  }, [open, planId, t]);

  const aktuell = () => ({
    to: parseAddresses(an),
    subject: betreff,
    body_text: text,
    body_html: "",
  });

  const ausgeben = async (
    modus: FollowupMode,
    pfad: string | null,
    erfolg: string,
  ) => {
    setLaeuft(true);
    setMeldung(null);
    try {
      const r = await commands.meetingFollowupOpen(aktuell(), modus, pfad);
      if (r.status !== "ok") {
        setMeldung({
          art: "fehler",
          text: t("planung.protokoll.entwurf.ausgabeFehler"),
        });
      } else if (modus === "mailto" && r.data) {
        // Zu lang für den Mail-Link: der Text liegt in der Zwischenablage (wie beim Follow-up).
        setMeldung({
          art: "hinweis",
          text: t("meetings.followup.mailtoClipped"),
        });
      } else {
        setMeldung({ art: "ok", text: erfolg });
      }
    } catch {
      setMeldung({
        art: "fehler",
        text: t("planung.protokoll.entwurf.ausgabeFehler"),
      });
    } finally {
      setLaeuft(false);
    }
  };

  const alsDatei = async () => {
    let ziel: string | null;
    try {
      ziel = await save({
        defaultPath: emlFileName(betreff),
        filters: [{ name: "E-Mail", extensions: ["eml"] }],
      });
    } catch {
      setMeldung({
        art: "fehler",
        text: t("planung.protokoll.entwurf.ausgabeFehler"),
      });
      return;
    }
    if (typeof ziel !== "string") return;
    await ausgeben(
      "eml",
      ziel,
      t("planung.protokoll.entwurf.dateiGespeichert"),
    );
  };

  const bereit = entwurf !== null;
  const ohneAdresse = entwurf?.ohne_adresse ?? 0;
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("planung.protokoll.entwurf.titel")}
      closeLabel={t("planung.protokoll.entwurf.schliessen")}
      className="max-w-3xl"
      footer={
        <div className="flex min-w-0 flex-wrap justify-end gap-2">
          {bereit && (
            <>
              <Button
                variant="secondary"
                disabled={laeuft}
                onClick={() =>
                  void ausgeben(
                    "copy",
                    null,
                    t("planung.protokoll.entwurf.kopiert"),
                  )
                }
                data-testid="protokoll-entwurf-kopieren"
              >
                {t("planung.protokoll.entwurf.kopieren")}
              </Button>
              <Button
                variant="secondary"
                disabled={laeuft}
                onClick={() =>
                  void ausgeben(
                    "mailto",
                    null,
                    t("planung.protokoll.entwurf.geoeffnet"),
                  )
                }
                data-testid="protokoll-entwurf-mailto"
              >
                {t("planung.protokoll.entwurf.mailprogramm")}
              </Button>
              <Button
                variant="secondary"
                disabled={laeuft}
                onClick={() => void alsDatei()}
                data-testid="protokoll-entwurf-eml"
              >
                {t("planung.protokoll.entwurf.datei")}
              </Button>
            </>
          )}
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("planung.protokoll.entwurf.schliessen")}
          </Button>
        </div>
      }
    >
      <div className="space-y-3" data-testid="protokoll-entwurf">
        <p className="plg-karte__hilfe">
          {t("planung.protokoll.entwurf.hinweis")}
        </p>
        {!bereit && fehler === null && (
          <p role="status" className="plg-karte__hilfe">
            {t("planung.protokoll.entwurf.laedt")}
          </p>
        )}
        {fehler !== null && (
          <p
            role="alert"
            className="plg-meldung plg-meldung--fehler"
            data-testid="protokoll-entwurf-fehler"
          >
            {fehler}
          </p>
        )}
        {bereit && (
          <>
            <div className="plg-feld">
              <label htmlFor="protokoll-an" className="plg-feld__label">
                {t("planung.protokoll.entwurf.an")}
              </label>
              <Input
                id="protokoll-an"
                className="w-full"
                value={an}
                onChange={(e) => setAn(e.target.value)}
                placeholder="name@example.org"
                data-testid="protokoll-entwurf-an"
              />
              <p
                className="plg-feld__hilfe"
                data-testid="protokoll-entwurf-an-hilfe"
              >
                {parseAddresses(an).length === 0
                  ? t("planung.protokoll.entwurf.keineEmpfaenger")
                  : t("planung.protokoll.entwurf.anHilfe")}
                {ohneAdresse > 0 &&
                  ` ${t("planung.protokoll.entwurf.ohneAdresse", { count: ohneAdresse })}`}
              </p>
            </div>
            <div className="plg-feld">
              <label htmlFor="protokoll-betreff" className="plg-feld__label">
                {t("planung.protokoll.entwurf.betreff")}
              </label>
              <Input
                id="protokoll-betreff"
                className="w-full"
                value={betreff}
                onChange={(e) => setBetreff(e.target.value)}
                data-testid="protokoll-entwurf-betreff"
              />
            </div>
            <div className="plg-feld">
              <label htmlFor="protokoll-text" className="plg-feld__label">
                {t("planung.protokoll.entwurf.text")}
              </label>
              <Textarea
                id="protokoll-text"
                rows={14}
                value={text}
                onChange={(e) => setText(e.target.value)}
                data-testid="protokoll-entwurf-text"
              />
            </div>
          </>
        )}
        {meldung && (
          <p
            role={meldung.art === "fehler" ? "alert" : "status"}
            className={
              meldung.art === "fehler"
                ? "plg-meldung plg-meldung--fehler"
                : "plg-meldung"
            }
            data-testid="protokoll-entwurf-meldung"
            data-art={meldung.art}
          >
            {meldung.text}
          </p>
        )}
      </div>
    </Dialog>
  );
};
