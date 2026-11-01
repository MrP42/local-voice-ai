import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import { MicSelector } from "./MicSelector";
import { CallMenu } from "./CallMenu";
import { CallButton } from "./CallButton";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { LlmSelector } from "./LlmSelector";
import { ResourceMeter } from "./ResourceMeter";
import UpdateChecker from "../update-checker";
import { ComplianceShield } from "../compliance/ComplianceShield";
import { FishServerButton } from "./FishServerButton";

const Footer: React.FC = () => {
  const [version, setVersion] = useState("");
  const callPage = useVoiceCallStore((s) => s.active);
  const callRunning = useVoiceCallStore((s) => s.state !== "idle");

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
        {/* Links: die zwei Modelle mit Rollensymbol. Sie beantworten, ob
            Diktat und Zusammenfassung gerade funktionieren. */}
        <div className="flex items-center gap-1 min-w-0">
          <ModelSelector />
          <LlmSelector />
        </div>

        {/* Rechts: Mikrofon, Telefon (Gespräch), Fish-Server, Schild, Speicher,
            Trenner, Version. */}
        <div className="flex items-center gap-0.5 shrink-0">
          <MicSelector />
          {/* Telefon direkt hinter dem Mikrofon: grau ohne, grün mit laufendem
              Gespräch; startet das Hauptgespräch wie das Tastenkürzel. Der Pfeil
              mit den Gesprächseinstellungen nur, solange es etwas einzustellen gibt. */}
          <span className="lv-call-group">
            <CallButton />
            {(callPage || callRunning) && <CallMenu />}
          </span>
          {/* Fish-Speech-Server (Vorlesen). */}
          <FishServerButton />
          {/* Schild: sicher und regelkonform? Immer sichtbar, auch schmal. */}
          <ComplianceShield />
          <ResourceMeter />
          <span className="lv-status-sep" aria-hidden="true" />
          {/* Version: ein Klick sucht nach Updates; ein gefundenes meldet
              sich beim Start als Fenster mit den Release-Notes. */}
          <UpdateChecker version={version} />
        </div>
      </div>
    </div>
  );
};

export default Footer;
