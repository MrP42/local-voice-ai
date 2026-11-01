import { useCallback, useMemo } from "react";
import { useSettings } from "@/hooks/useSettings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import {
  KNOWLEDGE_AREAS,
  knowledgeFrom,
  type KnowledgeArea,
  type KnowledgeState,
} from "./orchestration";

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

/**
 * Die Bereiche samt Schreibzugriff: dieselbe Einstellung für den Reiter „Wissen“ und den
 * Berechtigungs-Chip im Chatfeld. Nach dem Speichern verwirft das Gespräch Werkzeuge und Hinweis;
 * der nächste Schritt lädt neu.
 */
export function useKnowledgeEditor() {
  const { getSetting, updateSetting } = useSettings();
  const knowledge = useKnowledge();
  const knowledgeChanged = useVoiceCallStore((s) => s.knowledgeChanged);
  const save = useCallback(
    (
      key: "voice_call_knowledge" | "voice_call_read_transcripts",
      value: string[] | boolean,
    ) => void updateSetting(key, value as never).then(knowledgeChanged),
    [updateSetting, knowledgeChanged],
  );
  const toggle = useCallback(
    (area: KnowledgeArea, on: boolean) => {
      const current = (getSetting("voice_call_knowledge") ?? []) as string[];
      save(
        "voice_call_knowledge",
        KNOWLEDGE_AREAS.filter((a) => (a === area ? on : current.includes(a))),
      );
    },
    [getSetting, save],
  );
  return { knowledge, save, toggle };
}
