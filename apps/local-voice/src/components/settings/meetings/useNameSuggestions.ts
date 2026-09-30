import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type NameSuggestion } from "@/bindings";

export interface NameSuggestions {
  list: NameSuggestion[];
  /** Der Vorschlag für diesen Sprecher, falls es einen gibt. */
  forSpeaker: (channel: number, speakerIndex: number) => NameSuggestion | null;
  /** Dauerhaft verwerfen; der Vorschlag verschwindet sofort. */
  dismiss: (suggestion: NameSuggestion) => Promise<void>;
}

/**
 * U8: Namensvorschläge aus dem Gesagten ("Person 2 ist vermutlich André").
 * Das Backend rechnet nur; übernommen wird ausschließlich mit Klick (über das
 * Benennen des Sprechers). `refreshKey` ändert sich, wenn sich Segmente oder
 * Sprecher geändert haben; `enabled` ist während einer wachsenden Aufnahme
 * falsch, damit nicht bei jedem Segment gerechnet wird.
 */
export const useNameSuggestions = (
  meetingId: string,
  refreshKey: unknown,
  enabled: boolean,
): NameSuggestions => {
  const [list, setList] = useState<NameSuggestion[]>([]);
  const token = useRef(0);

  useEffect(() => {
    if (!enabled) {
      setList([]);
      return;
    }
    const mine = ++token.current;
    void commands
      .meetingSpeakerSuggestions(meetingId)
      .then((result) => {
        if (mine !== token.current) return;
        setList(
          result.status === "ok" && Array.isArray(result.data)
            ? result.data
            : [],
        );
      })
      .catch(() => {
        if (mine === token.current) setList([]);
      });
    return () => {
      // Ein späteres Ergebnis dieses Laufs wird verworfen.
      token.current += 1;
    };
  }, [meetingId, refreshKey, enabled]);

  const forSpeaker = useCallback(
    (channel: number, speakerIndex: number) =>
      list.find(
        (s) => s.channel === channel && s.speaker_index === speakerIndex,
      ) ?? null,
    [list],
  );

  const dismiss = useCallback(
    async (suggestion: NameSuggestion) => {
      setList((old) => old.filter((s) => s !== suggestion));
      await commands.meetingSpeakerSuggestionDismiss(
        meetingId,
        suggestion.channel,
        suggestion.speaker_index,
        suggestion.name,
      );
    },
    [meetingId],
  );

  return { list, forSpeaker, dismiss };
};

/**
 * Übernimmt einen Vorschlag: benennt den Sprecher. Gibt den Fehlercode des
 * Backends zurück (`null` bei Erfolg); der Aufrufer lädt danach neu.
 */
export const acceptNameSuggestion = async (
  meetingId: string,
  suggestion: NameSuggestion,
): Promise<string | null> => {
  const result = await commands.meetingSpeakerRename(
    meetingId,
    suggestion.channel,
    suggestion.speaker_index,
    suggestion.name,
  );
  return result.status === "error" ? result.error : null;
};
