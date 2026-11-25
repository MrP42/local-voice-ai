import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUp, FolderOpen, MessageSquare, X } from "lucide-react";
import { commands } from "@/bindings";
import { auspacken, fehlerText } from "@/lib/projekte";
import { useProjektRaum } from "@/lib/projektRaum";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { ProjektePage } from "../projekte/ProjektePage";
import { anzahlVon, useProjekteStand } from "../projekte/useProjekte";
import "../projekte/projektbereich.css";

/**
 * Projektraum im Chat (lva#826, Muster „Räume“): Startansicht eines Projekts an der Stelle des
 * Gesprächs. Oben die Frage im Projekt (die erste Nachricht legt das Projektgespräch an), darunter die
 * Gespräche des Projekts und seine Quellen mit dem Schalter „Als Kontext“ (die bisherige Projektseite,
 * eingebettet). Unter den Quellen ist Platz für das Wissen des Projekts (lva#827).
 */
export const ProjektRaum: React.FC<{ projektId: string }> = ({ projektId }) => {
  const { t, i18n } = useTranslation();
  const stand = useProjekteStand();
  const schliessen = useProjektRaum((s) => s.schliessen);
  const gespraeche = useVoiceCallStore((s) => s.conversations);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  // Aushängen (Seite verlassen, anderer Raum) entwertet eine laufende erste Frage synchron.
  useEffect(() => () => useProjektRaum.getState().entwerten(), []);
  const projekt = stand.projekte.find((p) => p.id === projektId);
  const eigene = useMemo(
    () => gespraeche.filter((g) => g.project_id === projektId),
    [gespraeche, projektId],
  );
  const datum = (iso: string) => {
    const ms = Date.parse(iso);
    return Number.isFinite(ms)
      ? new Date(ms).toLocaleDateString(i18n.language, {
          day: "numeric",
          month: "short",
        })
      : "";
  };

  /**
   * Erste Frage im Projekt: Stand der Auswahl frisch lesen (geht als `expectedStand` an das Tor), das
   * Gespräch im Projekt anlegen und die Frage senden. Hat die Person inzwischen den Raum verlassen,
   * ein anderes Projekt oder Gespräch geöffnet, wird nichts angelegt und nichts gesendet. Gesendet wird
   * nur in genau das Gespräch, das hier angelegt wurde.
   */
  const fragen = async () => {
    const frage = text.trim();
    if (!frage || busy) return;
    const gen = useProjektRaum.getState().gen;
    const gespraech = useVoiceCallStore.getState().conversationId;
    const nochHier = () =>
      useProjektRaum.getState().gen === gen &&
      useProjektRaum.getState().raumId === projektId &&
      useVoiceCallStore.getState().conversationId === gespraech;
    setBusy(true);
    setFehler(null);
    try {
      const u = auspacken(await commands.projektKontextUebersicht(projektId));
      if (!nochHier()) return;
      const id = await useVoiceCallStore
        .getState()
        .newProjectConversation(projektId, u.stand, nochHier);
      const jetzt = useVoiceCallStore.getState();
      if (!id) {
        // Überholt (anderes Gespräch geöffnet) ist kein Fehler; sonst bleibt der Raum mit Meldung stehen.
        if (useProjektRaum.getState().gen === gen)
          setFehler(jetzt.error ?? t("projektRaum.fehlerStart"));
        return;
      }
      // Der Wechsel ins neue Gespräch schließt den Raum (gen + 1); vor dem Senden zählt nur noch, dass
      // genau dieses Gespräch offen ist und die Chatseite noch steht.
      if (
        jetzt.conversationId !== id ||
        jetzt.projectId !== projektId ||
        !jetzt.active
      )
        return;
      setText("");
      await jetzt.sendText(frage, frage, []);
    } catch (x) {
      if (useProjektRaum.getState().gen === gen) setFehler(fehlerText(x, t));
    } finally {
      setBusy(false);
    }
  };

  if (stand.geladen && !projekt)
    return (
      <div className="pr-raum" data-testid="projekt-raum">
        <div className="pb-leer">
          <h2>{t("projektRaum.weg")}</h2>
          <button type="button" className="pb-btn" onClick={schliessen}>
            {t("projektRaum.schliessen")}
          </button>
        </div>
      </div>
    );

  return (
    <div
      className="pr-raum"
      data-testid="projekt-raum"
      data-projekt={projektId}
    >
      <div className="pr-raum__innen">
        <header className="pr-kopf">
          <span className="pr-kopf__symbol" aria-hidden="true">
            <FolderOpen width={22} height={22} />
          </span>
          <h2 className="pr-kopf__titel" data-testid="projekt-raum-titel">
            {projekt?.title ?? ""}
          </h2>
          <span className="pb-chip">
            {t("projektBereich.eintraege", {
              count: anzahlVon(stand, projektId),
            })}
          </span>
          <button
            type="button"
            className="pb-ib pr-kopf__zu"
            aria-label={t("projektRaum.schliessen")}
            title={t("projektRaum.schliessen")}
            data-testid="projekt-raum-schliessen"
            onClick={schliessen}
          >
            <X width={18} height={18} aria-hidden="true" />
          </button>
        </header>

        <form
          className="pr-frage"
          onSubmit={(e) => {
            e.preventDefault();
            void fragen();
          }}
        >
          <textarea
            className="pr-frage__feld"
            rows={2}
            value={text}
            disabled={busy}
            data-testid="projekt-raum-eingabe"
            aria-label={t("projektRaum.eingabe", {
              name: projekt?.title ?? "",
            })}
            placeholder={t("projektRaum.eingabe", {
              name: projekt?.title ?? "",
            })}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void fragen();
              }
            }}
          />
          <button
            type="submit"
            className="pr-frage__senden"
            disabled={busy || text.trim() === ""}
            aria-label={t("projektRaum.senden")}
            title={t("projektRaum.senden")}
            data-testid="projekt-raum-senden"
          >
            <ArrowUp width={18} height={18} aria-hidden="true" />
          </button>
        </form>
        <p className="pb-note pr-hinweis">{t("projektRaum.hinweis")}</p>
        {fehler && (
          <div className="pb-banner pb-banner--warn" role="alert">
            <span className="pb-banner__tx">{fehler}</span>
          </div>
        )}

        <section
          className="pr-karte"
          aria-labelledby="pr-gespraeche-titel"
          data-testid="projekt-raum-liste"
        >
          <h3 id="pr-gespraeche-titel" className="pr-karte__titel">
            {t("projektRaum.gespraeche")}
            <span>{eigene.length}</span>
          </h3>
          {eigene.length === 0 ? (
            <p className="pb-note pr-karte__leer">
              {t("projektRaum.keineGespraecheLang")}
            </p>
          ) : (
            <ul className="pr-gespraeche">
              {eigene.map((g) => (
                <li key={g.id}>
                  <button
                    type="button"
                    className="pr-gespraech"
                    data-testid="projekt-raum-eintrag"
                    onClick={() => {
                      schliessen();
                      void useVoiceCallStore.getState().openConversation(g.id);
                    }}
                  >
                    <MessageSquare width={16} height={16} aria-hidden="true" />
                    <span className="pr-gespraech__titel">
                      {g.title || t("projektRaum.ohneTitel")}
                      {g.last_text && (
                        <span className="pr-gespraech__text">
                          {g.last_text}
                        </span>
                      )}
                    </span>
                    <span className="pr-gespraech__datum">
                      {datum(g.updated_at)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="pr-quellen" aria-label={t("projektRaum.quellen")}>
          <ProjektePage raum={projektId} />
        </section>
      </div>
    </div>
  );
};
