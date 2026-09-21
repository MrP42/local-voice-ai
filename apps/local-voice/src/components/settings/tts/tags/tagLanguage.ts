import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { tagInsertFor } from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";

export type TagLanguage = "en" | "de";

/**
 * In welcher Sprache Tags im Text stehen: Einstellung `tts_tag_language`
 * ("auto" = Sprache der Oberflaeche, sonst fest "de"/"en"). Die Engine
 * bekommt immer die englische Form -- `canonicalizeTags` uebersetzt vor
 * jedem Aufruf; hier geht es nur um das, was der Nutzer sieht und tippt.
 */
export function useTagLanguage(): TagLanguage {
  const { i18n } = useTranslation();
  const { getSetting } = useSettings();
  const setting = (getSetting("tts_tag_language") ?? "auto") as string;
  if (setting === "de" || setting === "en") return setting;
  return i18n.language?.split("-")[0] === "de" ? "de" : "en";
}

/** Fertiger Klammertext eines Tags in der eingestellten Sprache. */
export const tagTextFor = (tag: TagDef, lang: TagLanguage): string =>
  `[${tagInsertFor(tag, lang)}]`;
