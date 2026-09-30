import { useCallback, useEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { commands, type TranscriptVariant } from "@/bindings";
import { translateYoutubeError } from "../youtube/youtubeErrors";

/**
 * Die Fassungen des Transkripts einer Besprechung (A3). Laedt beim Wechsel der
 * Besprechung und auf Anforderung (`reload`); eine spaete Antwort einer alten
 * Anfrage ueberschreibt nie eine neuere.
 */
export function useVariants(meetingId: string) {
  const [variants, setVariants] = useState<TranscriptVariant[]>([]);
  const run = useRef(0);

  const reload = useCallback(async () => {
    const seq = ++run.current;
    const result = await commands.transcriptVariants(meetingId);
    if (seq !== run.current) return;
    setVariants(result.status === "ok" ? (result.data ?? []) : []);
  }, [meetingId]);

  useEffect(() => {
    setVariants([]);
    void reload();
    return () => {
      run.current += 1;
    };
  }, [reload]);

  return { variants, reload };
}

/** Der Teil vor dem ersten Doppelpunkt waehlt den Text, der Rest ist `{{detail}}`. */
export const translateVariantError = (raw: string, t: TFunction): string => {
  const code = raw.split(":")[0].trim();
  if (code.startsWith("youtube_")) return translateYoutubeError(raw, t);
  const detail = raw.includes(":")
    ? raw.slice(raw.indexOf(":") + 1).trim()
    : "";
  if (code.startsWith("variant_") || code.startsWith("merge_")) {
    return t(`meetings.variants.errors.${code}`, {
      detail,
      defaultValue: raw,
    }).trim();
  }
  if (code === "no_provider" || code === "no_model") {
    return t(`meetings.variants.errors.${code}`);
  }
  return raw;
};

/** `v2 · Eigene Transkription (de)` als Text fuer Listen und Auswahlfelder. */
export const variantKindLabel = (v: TranscriptVariant, t: TFunction) =>
  `${t(`meetings.variants.kinds.${v.kind}`, { defaultValue: v.kind })}${
    v.language ? ` (${v.language})` : ""
  }`;
