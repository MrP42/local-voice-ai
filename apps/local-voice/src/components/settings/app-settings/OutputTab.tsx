import React from "react";
import { ReadAloudTab } from "./ReadAloudTab";
import { SoundTab } from "./SoundTab";
import { DocumentsTab } from "./DocumentsTab";

/**
 * Reiter "Ausgabe": was die App ausgibt -- Vorlesen, hoerbare Rueckmeldung und
 * exportierte Dokumente (Briefkopf fuer Word und PDF).
 */
export const OutputTab: React.FC = () => (
  <div className="w-full space-y-6">
    <ReadAloudTab />
    <SoundTab />
    <DocumentsTab />
  </div>
);
