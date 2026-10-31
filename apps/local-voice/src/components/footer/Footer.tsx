import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import { MicSelector } from "./MicSelector";
import { LlmSelector } from "./LlmSelector";
import { ResourceMeter } from "./ResourceMeter";
import UpdateChecker from "../update-checker";
import { ComplianceShield } from "../compliance/ComplianceShield";
import { FishServerButton } from "./FishServerButton";

const Footer: React.FC = () => {
  const [version, setVersion] = useState("");

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

        {/* Rechts: Mikrofon, Anrufmenü, Fish-Server, Schild, Speicher,
            Trenner, Version. */}
        <div className="flex items-center gap-0.5 shrink-0">
          <MicSelector />
          {/* Platzhalter: <CallMenu /> (src/components/footer/CallMenu.tsx)
              kommt mit Branch claude/project-thread-4aeogv und gehört hier
              direkt hinter das Mikrofon. */}
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
