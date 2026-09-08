import React, { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";

import ModelSelector from "../model-selector";
import { MicSelector } from "./MicSelector";
import UpdateChecker from "../update-checker";

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
    <div className="workspace-status w-full border-t border-mid-gray/20 py-2">
      <div className="flex flex-wrap justify-between items-center gap-2 text-xs px-3 text-text/70">
        {/* Model and microphone side by side: together they answer "will a
            dictation work right now?" without leaving the page. */}
        <div className="flex flex-wrap items-center gap-3 min-w-0">
          <ModelSelector />
          <MicSelector />
        </div>

        {/* Update Status */}
        <div className="hidden sm:flex items-center gap-1">
          <UpdateChecker />
          <span>•</span>
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span>v{version}</span>
        </div>
      </div>
    </div>
  );
};

export default Footer;
