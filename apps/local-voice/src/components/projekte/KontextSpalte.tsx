import React from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type DokumentErgebnis,
  type KontextUebersicht,
} from "@/bindings";
import { auspacken, fehlerText, klassenName } from "@/lib/projekte";
import { meldeProjekte } from "@/lib/projekteBus";
import { nichtImKontext, zahl } from "@/lib/projekteKontext";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import type { Modellfenster } from "./useProjekte";

const anteil = (u: KontextUebersicht | null, f: Modellfenster): number =>
  u && f.tokens > 0
    ? Math.min(100, Math.round((u.tokens / f.tokens) * 100))
    : 0;

export const zuGross = (u: KontextUebersicht | null, f: Modellfenster) =>
  !!u && u.tokens > f.tokens;

/** Summe, Anzahl und Fenster: der obere Teil der rechten Spalte. */
export const KontextSumme: React.FC<{
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
}> = ({ uebersicht, fenster }) => {
  const { t } = useTranslation();
  const summe = uebersicht?.tokens ?? 0;
  const count = uebersicht?.count ?? 0;
  const gross = zuGross(uebersicht, fenster);
  return (
    <div className="pb-summe" data-testid="kontext-summe">
      <b data-testid="kontext-summe-text">
        {t("projektBereich.summe", {
          summe: zahl(summe),
          fenster: zahl(fenster.tokens),
        })}
      </b>
      <div
        className={`pb-meter${gross ? " pb-meter--zu" : ""}`}
        role="img"
        aria-label={t("projektBereich.meter", {
          prozent: anteil(uebersicht, fenster),
        })}
      >
        <i style={{ width: `${anteil(uebersicht, fenster)}%` }} />
      </div>
      <span className="pb-note" data-testid="kontext-anzahl">
        {count === 0
          ? t("projektBereich.zaehlKeine")
          : t("projektBereich.zaehl", { count })}
        {" · "}
        {fenster.modell
          ? t("projektBereich.fensterModell", { modell: fenster.modell })
          : t("projektBereich.fenster")}
      </span>
    </div>
  );
};

/**
 * „Gespräch im Projekt beginnen“ (lva#365 P4c-2). Das Gespräch merkt sich den `stand` der Übersicht, die
 * die Person gesehen hat; weicht er beim Senden ab, stoppt das Tor mit `projekt.stand_veraltet`.
 */
const GespraechBeginnen: React.FC<{
  projektId: string;
  stand: string | null;
}> = ({ projektId, stand }) => {
  const { t } = useTranslation();
  const [busy, setBusy] = React.useState(false);
  const beginnen = async () => {
    setBusy(true);
    try {
      await useVoiceCallStore
        .getState()
        .newProjectConversation(projektId, stand);
      window.dispatchEvent(
        new CustomEvent("lv-navigate", { detail: { section: "home" } }),
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button
        type="button"
        className="pb-btn pb-btn--pri"
        style={{ marginTop: 8 }}
        disabled={busy || stand === null}
        aria-describedby="pb-ki-start-hinweis"
        data-testid="projekt-gespraech-beginnen"
        onClick={() => void beginnen()}
      >
        {t("projektBereich.ki.start")}
      </button>
      <p
        id="pb-ki-start-hinweis"
        className="pb-note"
        style={{ margin: "6px 0 0" }}
      >
        {t("projektBereich.ki.startHinweis")}
      </p>
    </>
  );
};

/**
 * „KI-Dokument erstellen“ (lva#365 P4c-3). Die KI schreibt aus der Auswahl im Projekt ein Dokument und legt
 * es als Entwurf in der Dokumentenablage ab; im Projekt bleibt es ausgeschaltet, bis die Person es
 * einschaltet. Klasse und Quellen bestimmt das Backend (die Oberfläche kann die Klasse nicht senken).
 */
const DokumentErstellen: React.FC<{
  projektId: string;
  stand: string | null;
}> = ({ projektId, stand }) => {
  const { t } = useTranslation();
  const [offen, setOffen] = React.useState(false);
  const [auftrag, setAuftrag] = React.useState("");
  const [name, setName] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [fehler, setFehler] = React.useState<string | null>(null);
  const [fertig, setFertig] = React.useState<DokumentErgebnis | null>(null);
  const bereit =
    !busy && auftrag.trim() !== "" && name.trim() !== "" && stand !== null;
  const erstellen = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!bereit) return;
    setBusy(true);
    setFehler(null);
    setFertig(null);
    try {
      const r = auspacken(
        await commands.projektKiDokumentErstellen(
          projektId,
          auftrag.trim(),
          null,
          name.trim(),
          stand,
        ),
      );
      setFertig(r);
      setOffen(false);
      setAuftrag("");
      setName("");
      meldeProjekte("projekte");
    } catch (err) {
      setFehler(fehlerText(err, t));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button
        type="button"
        className="pb-btn"
        style={{ marginTop: 8 }}
        disabled={busy || stand === null}
        aria-expanded={offen}
        title={t("projektBereich.ki.dokumentHinweis")}
        data-testid="projekt-dokument-erstellen"
        onClick={() => setOffen((o) => !o)}
      >
        {t("projektBereich.ki.dokument")}
      </button>
      {offen && (
        <form
          className="pb-dok"
          data-testid="projekt-dokument-formular"
          onSubmit={(e) => void erstellen(e)}
        >
          <label>
            {t("projektBereich.ki.dokumentAuftrag")}
            <textarea
              value={auftrag}
              maxLength={4000}
              rows={4}
              onChange={(e) => setAuftrag(e.target.value)}
              data-testid="projekt-dokument-auftrag"
            />
          </label>
          <label>
            {t("projektBereich.ki.dokumentName")}
            <input
              type="text"
              value={name}
              maxLength={200}
              onChange={(e) => setName(e.target.value)}
              data-testid="projekt-dokument-name"
            />
          </label>
          <p className="pb-note" style={{ margin: 0 }}>
            {t("projektBereich.ki.dokumentRegel")}
          </p>
          <div className="pb-dok__acts">
            <button
              type="submit"
              className="pb-btn pb-btn--pri"
              disabled={!bereit}
              data-testid="projekt-dokument-senden"
            >
              {busy
                ? t("projektBereich.ki.dokumentLaeuft")
                : t("projektBereich.ki.dokumentSenden")}
            </button>
            <button
              type="button"
              className="pb-btn"
              disabled={busy}
              onClick={() => setOffen(false)}
            >
              {t("projektBereich.abbrechen")}
            </button>
          </div>
        </form>
      )}
      {fehler && (
        <p
          role="alert"
          className="pb-note"
          style={{ margin: "6px 0 0" }}
          data-testid="projekt-dokument-fehler"
        >
          {fehler}
        </p>
      )}
      {fertig && (
        <p
          role="status"
          className="pb-note"
          style={{ margin: "6px 0 0" }}
          data-testid="projekt-dokument-fertig"
        >
          {t(
            fertig.imProjekt
              ? "projektBereich.ki.dokumentFertig"
              : "projektBereich.ki.dokumentNurAblage",
            { name: fertig.name, klasse: klassenName(fertig.klasse, t) },
          )}
        </p>
      )}
    </>
  );
};

/** Inhalt der rechten Spalte „Kontext im Projekt“ (auch im Dialog „Details“ unter 999 px). */
export const KontextInhalt: React.FC<{
  projektId?: string;
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
}> = ({ projektId, uebersicht, fenster }) => {
  const { t } = useTranslation();
  const fehlt = nichtImKontext(uebersicht);
  return (
    <>
      <KontextSumme uebersicht={uebersicht} fenster={fenster} />
      {fehlt.length > 0 && (
        <section className="pb-box" data-testid="nicht-im-kontext">
          <h3>{t("projektBereich.nichtImKontext")}</h3>
          <ul>
            {fehlt.map((f) => (
              <li key={f.art}>
                {t(`projektBereich.fehlt.${f.art}`, { count: f.count })}
              </li>
            ))}
          </ul>
        </section>
      )}
      <section className="pb-box">
        <h3>{t("projektBereich.geltung.titel")}</h3>
        {t("projektBereich.geltung.text")}
      </section>
      <section className="pb-box" data-testid="ki-im-projekt">
        <h3>{t("projektBereich.ki.titel")}</h3>
        <p style={{ margin: 0 }}>{t("projektBereich.ki.text")}</p>
        {projektId && (
          <>
            <GespraechBeginnen
              projektId={projektId}
              stand={uebersicht?.stand ?? null}
            />
            <DokumentErstellen
              projektId={projektId}
              stand={uebersicht?.stand ?? null}
            />
          </>
        )}
      </section>
    </>
  );
};

/** Leiste unten statt rechter Spalte (unter 999 px). */
export const Summenleiste: React.FC<{
  uebersicht: KontextUebersicht | null;
  fenster: Modellfenster;
  onDetails: () => void;
}> = ({ uebersicht, fenster, onDetails }) => {
  const { t } = useTranslation();
  const count = uebersicht?.count ?? 0;
  const gross = zuGross(uebersicht, fenster);
  return (
    <div className="pb-sumbar" data-testid="summenleiste">
      <b>
        {t("projektBereich.summeKurz", {
          summe: zahl(uebersicht?.tokens ?? 0),
        })}
      </b>
      <span
        className={`pb-meter${gross ? " pb-meter--zu" : ""}`}
        aria-hidden="true"
      >
        <i style={{ width: `${anteil(uebersicht, fenster)}%` }} />
      </span>
      <span className="pb-note">
        {t("projektBereich.eintraegeKurz", { count })}
      </span>
      <button type="button" className="pb-btn pb-btn--link" onClick={onDetails}>
        {t("projektBereich.details")}
      </button>
    </div>
  );
};
