import { modelAllowed, type Souveraenitaet, type VoiceModel } from "../../vendor/wai-voice/model.ts";

type Modell = Pick<VoiceModel, "id" | "label" | "local" | "jurisdiction">;

/** Dieselbe Entscheidung für Orchestrator, Auswahl und Schild. */
export function blockedBy(model: Modell, stufe: Souveraenitaet): boolean {
  return !modelAllowed({
    ...model,
    supportsTools: false,
    turn: () => Promise.reject(new Error("nicht aufrufbar")),
  }, stufe);
}

export function gespraechsSperren(
  sprecher: Modell | undefined,
  denker: Modell | undefined,
  stufe: Souveraenitaet,
): ("sprecher" | "denker")[] {
  const sperren: ("sprecher" | "denker")[] = [];
  if (sprecher && blockedBy(sprecher, stufe)) sperren.push("sprecher");
  if (denker && blockedBy(denker, stufe)) sperren.push("denker");
  return sperren;
}
