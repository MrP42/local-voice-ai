import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
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

const Footer: React.FC = () => {
  const [version, setVersion] = useState("");
  const callPage = useVoiceCallStore((s) => s.active);
  const callRunning = useVoiceCallStore((s) => s.state !== "idle");
  const { t } = useTranslation();
  const [diagnose, setDiagnose] = useDiagnose();

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("0.1.2");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="workspace-status w-full">
      <div className="flex justify-between items-center gap-2 px-2 text-text/70">
        {/* Links (ADR-0027 E7, schlank): Diktatmodell nur, solange es
            Aufmerksamkeit braucht (lädt, Download, Fehler). Die Modellwahl
            steht im Profilmenü ▸ Modelle. */}
        <div className="flex items-center gap-1 min-w-0">
          <ModelSelector nurStatus />
        </div>

        {/* Rechts (ADR-0027 E7, Codex 01.11.): nur Mikrofon und Telefon.
            Speicher/GPU, Fish-Server und Schild nur als bewusst
            eingeschaltete Diagnose (Profilmenü ▸ Hilfe), einklappbar. Die
            Version nur, wenn ein Update bereitsteht (sonst in „Über“). */}
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
                  <ComplianceShield />
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
          {/* Update: sucht beim Start und auf Wunsch (Profilmenü ▸ Hilfe);
              ein gefundenes meldet sich als Fenster mit den Release-Notes. */}
          <UpdateChecker version={version} nurBeiUpdate />
        </div>
      </div>
    </div>
  );
};

export default Footer;
