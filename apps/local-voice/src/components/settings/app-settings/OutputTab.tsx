import React from "react";
import { ReadAloudTab } from "./ReadAloudTab";
import { SoundTab } from "./SoundTab";

/** Reiter "Ausgabe": was die App ausgibt -- Vorlesen und hoerbare Rueckmeldung. */
export const OutputTab: React.FC = () => (
  <div className="w-full space-y-6">
    <ReadAloudTab />
    <SoundTab />
  </div>
);
