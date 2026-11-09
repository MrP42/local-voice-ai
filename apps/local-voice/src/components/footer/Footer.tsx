import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import { LlmSelector } from "./LlmSelector";
import { FunctionModelChip } from "./FunctionModelChip";
import { useFussleisteStore } from "@/stores/fussleisteStore";
import { MicSelector } from "./MicSelector";
import { CallMenu } from "./CallMenu";
import { CallButton } from "./CallButton";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { ResourceMeter } from "./ResourceMeter";
import UpdateChecker from "../update-checker";
import { ComplianceShield } from "../compliance/ComplianceShield";
import { FishServerButton } from "./FishServerButton";
import { ChevronLeft, ChevronRight, Activity } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useDiagnose } from "./diagnose";

/**
 * Welche Modelle das Modul in der Fußleiste zeigt (Patrick 02.11.: je Modul,
 * immer an derselben Stelle links unten). Vorlesen hängt seine Stimme
 * zusätzlich per Portal in den Modulplatz.
 */
function modulModelle(section: string | undefined): React.ReactNode {
  switch (section) {
    case "tts":
      // Diktat nur, wenn es Aufmerksamkeit braucht; Stimme kommt per Portal.
      return (
        <>
          <ModelSelector nurStatus />
          <FunctionModelChip fn="read_aloud" />
        </>
      );
    case "meetings":
      return (
        <>
          <ModelSelector />
          <FunctionModelChip fn="meeting_protocol" />
        </>
      );
    case "home":
    case "voice":
      // Der Sprecher steht im Eingabefeld des Gesprächs (F2).
      return <ModelSelector />;
    default:
      return (
        <>
          <ModelSelector />
          <LlmSelector />
        </>
      );
  }
}

const Footer: React.FC<{ section?: string }> = ({ section }) => {
  const [version, setVersion] = useState("");
  const callPage = useVoiceCallStore((s) => s.active);
  const callRunning = useVoiceCallStore((s) => s.state !== "idle");
  const { t } = useTranslation();
  const [diagnose, setDiagnose] = useDiagnose();
  const setModulPlatz = useFussleisteStore((s) => s.setModulPlatz);
  const status = useFussleisteStore((s) => s.status);
  const loescheStatus = useFussleisteStore((s) => s.loescheStatus);
  // Der Kurzstatus gehört zum Modul, in dem er entstand.
  useEffect(() => loescheStatus(), [section, loescheStatus]);

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion ?? "");
      } catch (error) {
        // Ohne Kernantwort keine erfundene Nummer: dann bleibt die Stelle leer.
        console.error("Failed to get app version:", error);
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="workspace-status w-full">
      <div className="flex justify-between items-center gap-2 px-2 text-text/70">
        {/* Links: die Modelle des geöffneten Moduls (Patrick 02.11., löst
            ADR-0027 E7 „Modellwahl im Profilmenü“ ab), davor der Platz für
            modulspezifische Wahl (Stimme beim Vorlesen), dahinter ein kurzer
            Status, der von selbst verschwindet und nichts verschiebt. */}
        <div
          className="flex items-center gap-1 min-w-0"
          data-testid="fuss-modelle"
          data-modul={section}
        >
          <span
            ref={setModulPlatz}
            className="lv-fuss-modulplatz"
            data-testid="fuss-modulplatz"
          />
          {modulModelle(section)}
          <span
            role="status"
            aria-live="polite"
            className="lv-fuss-status min-w-0 truncate text-text/60"
            data-testid="fuss-status"
          >
            {status?.text ?? ""}
          </span>
        </div>

        {/* Rechts (ADR-0027 E7, Codex 01.11.): Mikrofon, Telefon und Schild.
            Speicher/GPU und Fish-Server nur als bewusst
            eingeschaltete Diagnose (Profilmenü ▸ Hilfe), einklappbar. Die
            Version steht immer ganz rechts (Patrick 05.11., #281). */}
        <div className="flex items-center gap-0.5 shrink-0">
          {diagnose !== "aus" && (
            <span className="lv-diagnose" data-diagnose={diagnose}>
              <button
                type="button"
                className="lv-status-btn lv-diagnose__knopf"
                aria-expanded={diagnose === "auf"}
                aria-label={t("shell.diagnose")}
                title={t("shell.diagnose")}
                data-testid="statusbar-diagnose"
                onClick={() => setDiagnose(diagnose === "auf" ? "zu" : "auf")}
              >
                <Activity size={14} aria-hidden="true" />
                {diagnose === "auf" ? (
                  <ChevronRight size={12} aria-hidden="true" />
                ) : (
                  <ChevronLeft size={12} aria-hidden="true" />
                )}
              </button>
              {diagnose === "auf" && (
                <>
                  <FishServerButton />
                  <ResourceMeter />
                </>
              )}
            </span>
          )}
          <MicSelector />
          {/* Telefon direkt hinter dem Mikrofon: grau ohne, grün mit laufendem
              Gespräch; startet das Hauptgespräch wie das Tastenkürzel. Der Pfeil
              mit den Gesprächseinstellungen nur, solange es etwas einzustellen gibt. */}
          <span className="lv-call-group">
            <CallButton />
            {(callPage || callRunning) && <CallMenu />}
          </span>
          {/* Das Schild steht immer sichtbar ganz rechts, direkt vor der
              Version (Patrick 08.11.2026): grün/gelb/rot für Souveränität
              und Regelwerk, Klick erklärt. */}
          <ComplianceShield />
          {/* Update: sucht beim Start und auf Wunsch (Profilmenü ▸ Hilfe);
              ein gefundenes meldet sich als Fenster mit den Release-Notes. */}
          <UpdateChecker version={version} />
        </div>
      </div>
    </div>
  );
};

export default Footer;
