import type { LucideIcon } from "lucide-react";

/**
 * Die sieben Schubladen der Tag-Palette. Reihenfolge ist auch die
 * Reiterreihenfolge in der UI (nach Favoriten und Zuletzt) — Emotionen
 * zuerst, weil sie den Großteil der Registry ausmachen und am häufigsten
 * gebraucht werden, Spezial zuletzt als Sammelbecken fürs Ungewöhnliche.
 */
export type TagCategoryId =
  | "emotionBasic"
  | "emotionAdvanced"
  | "tone"
  | "dynamics"
  | "effects"
  | "pauses"
  | "special";

/**
 * Ein einzelnes Tag der Registry.
 *
 * `insert` ist bewusst von `id` getrennt: `id` ist ein stabiler,
 * bindestrich-getrennter Schlüssel (für React-Keys, Favoriten-Speicherung,
 * Suche), `insert` ist der Wortlaut, der tatsächlich zwischen die eckigen
 * Klammern kommt — bei Mehrwort-Tags wie "short pause" identisch bis auf die
 * Bindestriche.
 */
export interface TagDef {
  /** Kanonisch, stabil, bindestrich-getrennt — z. B. "short-pause". */
  id: string;
  /** Was zwischen die eckigen Klammern kommt — z. B. "short pause". */
  insert: string;
  category: TagCategoryId;
  label: { en: string; de: string };
  /** Ein Satz Wirkung — Tooltip-Text auf dem Chip. */
  description?: { en: string; de: string };
  /** Suchbegriffe de+en, die nicht schon Label/Insert sind. */
  aliases?: string[];
  /**
   * S1-Pendant für die (…)-Klammer-Schreibweise, falls es eines gibt —
   * bei identischem Namen derselbe String, sonst `null`. Dient späteren
   * Paketen als Mapping-Grundlage; A2 selbst zeigt nur die S2-Klammerform.
   */
  s1?: string | null;
}

export interface TagCategoryDef {
  id: TagCategoryId;
  icon: LucideIcon;
}
