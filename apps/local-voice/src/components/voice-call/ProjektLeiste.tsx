import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button as FluentButton,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
} from "@fluentui/react-components";
import { FolderOpen, X } from "lucide-react";
import { commands } from "@/bindings";
import { auspacken } from "@/lib/projekte";
import { oeffneProjekt } from "@/lib/projekteBus";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { istInfo, type ProjektHinweis } from "./projekt";

/** Namen der Projekte, einmal je Kennung geholt (der Name ändert die Bindung nicht). */
const NAMEN = new Map<string, string>();

function useProjektName(id: string | null): string | null {
  const [name, setName] = useState<string | null>(
    id ? (NAMEN.get(id) ?? null) : null,
  );
  useEffect(() => {
    if (!id) {
      setName(null);
      return;
    }
    const bekannt = NAMEN.get(id);
    if (bekannt) {
      setName(bekannt);
      return;
    }
    let aktuell = true;
    commands
      .projekteListe()
      .then((r) => {
        for (const p of auspacken(r)) NAMEN.set(p.id, p.title);
        if (aktuell) setName(NAMEN.get(id) ?? null);
      })
      .catch(() => undefined);
    return () => {
      aktuell = false;
    };
  }, [id]);
  return name;
}

/** Fehler des Projektwegs, die auf ein Problem im System hindeuten (nicht auf eine Eingabe der Person). */
const STOERUNG: readonly string[] = [
  "projekt.register",
  "projekt.bindung",
  "projekt.fremdkontext",
  "projekt.ablage",
];

const zahl = (n: number | undefined, sprache: string): string =>
  (n ?? 0).toLocaleString(sprache);

/** Hinweisleiste im Stil der Fluent-2-Meldungsleiste (MessageBar) mit Schließen und, wo sinnvoll, Aktion. */
const Hinweis: React.FC<{ hinweis: ProjektHinweis; projektId: string }> = ({
  hinweis,
  projektId,
}) => {
  const { t, i18n } = useTranslation();
  const dismiss = useVoiceCallStore((s) => s.dismissProjektHinweis);
  const setStand = useVoiceCallStore((s) => s.setProjektStand);
  const [busy, setBusy] = useState(false);
  const name = hinweis.code.replace(/^projekt\./, "");
  const info = istInfo(hinweis) || name === "widerrufen";
  const intent = info
    ? "info"
    : STOERUNG.includes(hinweis.code)
      ? "error"
      : "warning";
  const key = `voiceCall.projekt.hinweis.${name}`;
  const sprache = i18n.language;
  const b = hinweis.budget;
  const text = i18n.exists(key)
    ? t(key, {
        count: hinweis.entzogen ?? 0,
        benoetigt: zahl(b?.benoetigt, sprache),
        fenster: zahl(b?.fenster, sprache),
        kontext: zahl(b?.kontext, sprache),
        historie: zahl(b?.historie, sprache),
        auszuege: zahl(b?.auszuege, sprache),
        frage: zahl(b?.frage, sprache),
        ausgabe: zahl(b?.ausgabe, sprache),
      })
    : t("voiceCall.projekt.hinweis.andere", { code: hinweis.code });
  const titel =
    name === "widerrufen"
      ? t("voiceCall.projekt.titel.widerrufen")
      : info
        ? t("voiceCall.projekt.titel.info")
        : t("voiceCall.projekt.titel.stopp");

  const uebernehmen = async () => {
    setBusy(true);
    try {
      const u = auspacken(await commands.projektKontextUebersicht(projektId));
      setStand(u.stand);
    } catch {
      /* ohne Übersicht bleibt der Hinweis stehen */
    } finally {
      setBusy(false);
    }
  };

  return (
    <MessageBar
      intent={intent}
      layout="multiline"
      shape="square"
      data-testid="projekt-hinweis"
      data-code={hinweis.code}
    >
      <MessageBarBody>
        <MessageBarTitle>{titel}</MessageBarTitle>
        {text}
      </MessageBarBody>
      <MessageBarActions
        containerAction={
          <FluentButton
            appearance="transparent"
            size="small"
            icon={<X aria-hidden="true" width={16} height={16} />}
            aria-label={t("voiceCall.projekt.schliessen")}
            onClick={dismiss}
            data-testid="projekt-hinweis-schliessen"
          />
        }
      >
        {hinweis.code === "projekt.stand_veraltet" && (
          <FluentButton
            size="small"
            disabled={busy}
            onClick={() => void uebernehmen()}
            data-testid="projekt-stand-uebernehmen"
          >
            {t("voiceCall.projekt.uebernehmen")}
          </FluentButton>
        )}
      </MessageBarActions>
    </MessageBar>
  );
};

/**
 * Projektchip und Hinweisleiste im Gespräch (lva#365 P4c-2). Ohne Projekt erscheint nichts. Der Chip
 * sagt, dass die Antworten auf der Auswahl im Projekt beruhen (kein Notizbuch, keine App-Daten, keine
 * Werkzeuge); die Leiste zeigt, warum nichts gesendet wurde oder was am Verlauf geändert wurde.
 */
export const ProjektLeiste: React.FC = () => {
  const { t } = useTranslation();
  const projektId = useVoiceCallStore((s) => s.projectId);
  const hinweis = useVoiceCallStore((s) => s.projektHinweis);
  const name = useProjektName(projektId);
  if (!projektId) return null;
  return (
    <div data-testid="projekt-leiste" className="min-w-0">
      <div className="flex items-center gap-2 px-4 py-1 border-b border-mid-gray/20 text-xs min-w-0">
        <button
          type="button"
          data-testid="projekt-chip"
          onClick={() => oeffneProjekt(projektId)}
          title={t("voiceCall.projekt.chipTitel")}
          className="inline-flex items-center gap-1.5 min-w-0 max-w-full rounded-full border border-mid-gray/30 px-2.5 min-h-6 max-sm:min-h-[44px] cursor-pointer text-text/85 hover:bg-mid-gray/15 hover:text-text"
        >
          <FolderOpen className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
          <span className="truncate">
            {name
              ? t("voiceCall.projekt.chip", { name })
              : t("voiceCall.projekt.chipOhneName")}
          </span>
        </button>
        <span className="text-text/60 truncate max-sm:hidden">
          {t("voiceCall.projekt.chipText")}
        </span>
      </div>
      {hinweis && <Hinweis hinweis={hinweis} projektId={projektId} />}
    </div>
  );
};
