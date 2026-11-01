import { useMemo } from "react";
import { useSettings } from "@/hooks/useSettings";
import { knowledgeFrom, type KnowledgeState } from "./orchestration";

/** Die freigegebenen Wissensbereiche des Gesprächs, wie die Einstellungen sie halten. */
export function useKnowledge(): KnowledgeState {
  const { getSetting } = useSettings();
  const areas = getSetting("voice_call_knowledge");
  const transcripts = getSetting("voice_call_read_transcripts");
  return useMemo(
    () =>
      knowledgeFrom({
        voice_call_knowledge: areas,
        voice_call_read_transcripts: transcripts,
      }),
    [areas, transcripts],
  );
}
