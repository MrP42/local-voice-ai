import React from "react";
import { LlmSelector } from "./LlmSelector";

/**
 * Sprachmodell EINER Funktion in der Fußleiste (Patrick 02.11.), z. B. beim
 * Vorlesen das Modell für Übersetzen und KI-Dokumente. Seit 03.11. derselbe
 * Knopf wie in allen anderen Modulen (Ampel, Name, Abzeichen, Pfeil): die
 * Darstellung bleibt gleich, nur die Wahl gilt für diese Funktion.
 */
export const FunctionModelChip: React.FC<{ fn: string }> = ({ fn }) => (
  <LlmSelector fn={fn} />
);

export default FunctionModelChip;
