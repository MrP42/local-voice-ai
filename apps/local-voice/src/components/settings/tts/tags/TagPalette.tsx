import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LucideIcon } from "lucide-react";
import { ChevronDown, ChevronUp, Clock, Plus, Search, Star } from "lucide-react";
import { usePersistentState } from "@/hooks/usePersistentState";
import { useSettings } from "@/hooks/useSettings";
import { Input } from "@/components/ui/Input";
import { Button } from "@/components/ui/Button";
import {
  TAG_CATEGORIES,
  TAG_REGISTRY,
  localizedLabel,
  searchTags,
} from "@/lib/tags/registry";
import type { TagCategoryId, TagDef } from "@/lib/tags/types";
import { TagChip } from "./TagChip";

/** "Zuletzt benutzt" merkt sich hoechstens diese vielen Eintraege. */
const MAX_RECENT = 8;

type ActiveTab = "favorites" | "recent" | TagCategoryId;

const isTagCategory = (value: string): value is TagCategoryId =>
  TAG_CATEGORIES.some((category) => category.id === value);

const isActiveTab = (value: string): value is ActiveTab =>
  value === "favorites" || value === "recent" || isTagCategory(value);

/** Grosszuegig gegen kaputten localStorage-Inhalt: alles, was keine Liste
 *  von Strings ist, gilt als leer statt als Fehler. */
const parseRecent = (raw: string): string[] => {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is string => typeof entry === "string");
  } catch {
    return [];
  }
};

/** Ein Chip-Grid-Eintrag — entweder ein Registry-Tag (favorisierbar,
 *  durchsuchbar) oder ein frei getipptes Tag aus "Zuletzt" (weder noch). */
interface ChipItem {
  key: string;
  insertText: string;
  label: string;
  description?: string;
  registryId?: string;
}

const toChipItem = (tag: TagDef, uiLang: string): ChipItem => ({
  key: tag.id,
  insertText: tag.insert,
  label: localizedLabel(tag, uiLang),
  description: uiLang === "de" ? tag.description?.de : tag.description?.en,
  registryId: tag.id,
});

const toCustomChipItem = (text: string): ChipItem => ({
  key: `custom:${text}`,
  insertText: text,
  label: text,
});

const TabButton: React.FC<{
  active: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
}> = ({ active, icon: Icon, label, onClick }) => (
  <button
    type="button"
    role="tab"
    aria-selected={active}
    onClick={onClick}
    className={`inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium whitespace-nowrap transition-colors cursor-pointer ${
      active
        ? "bg-logo-primary text-on-accent"
        : "text-text/60 hover:text-text hover:bg-mid-gray/15"
    }`}
  >
    <Icon width={13} height={13} aria-hidden="true" />
    {label}
  </button>
);

/**
 * Die Tag-Palette: Suche, Favoriten/Zuletzt/Kategorien-Reiter, ein Chip-Grid
 * und eine Freitext-Zeile fuer Tags, die nicht in der Registry stehen.
 * Eigenstaendig und ueberall einbettbar — ein spaeteres Paket montiert sie in
 * die Vorlesen-Seite; hier steht nur `onInsert` zwischen ihr und dem Text.
 */
export const TagPalette: React.FC<{
  onInsert: (tagText: string) => void;
  uiLang: string;
}> = ({ onInsert, uiLang }) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();

  const [isOpenRaw, setIsOpenRaw] = usePersistentState<string>(
    "tts.tags.paletteOpen",
    "1",
  );
  const isOpen = isOpenRaw === "1";

  const [activeTabRaw, setActiveTabRaw] = usePersistentState<string>(
    "tts.tags.activeCat",
    "favorites",
    isActiveTab,
  );
  const activeTab = activeTabRaw as ActiveTab;

  const [recentRaw, setRecentRaw] = usePersistentState<string>(
    "tts.tags.recent",
    "[]",
  );
  const recent = useMemo(() => parseRecent(recentRaw), [recentRaw]);

  const [query, setQuery] = useState("");
  const [customText, setCustomText] = useState("");

  const favorites = getSetting("tts_tag_favorites") ?? [];

  const rememberRecent = (insertText: string) => {
    const next = [
      insertText,
      ...recent.filter((entry) => entry !== insertText),
    ].slice(0, MAX_RECENT);
    setRecentRaw(JSON.stringify(next));
  };

  const insertItem = (item: ChipItem) => {
    onInsert(`[${item.insertText}]`);
    rememberRecent(item.insertText);
  };

  const insertCustom = () => {
    const text = customText.trim();
    if (!text) return;
    onInsert(`[${text}]`);
    rememberRecent(text);
    setCustomText("");
  };

  const toggleFavorite = (id: string) => {
    const next = favorites.includes(id)
      ? favorites.filter((favoriteId) => favoriteId !== id)
      : [...favorites, id];
    void updateSetting("tts_tag_favorites", next);
  };

  const visibleTags: ChipItem[] = useMemo(() => {
    const trimmedQuery = query.trim();
    if (trimmedQuery) {
      return searchTags(trimmedQuery, uiLang).map((tag) =>
        toChipItem(tag, uiLang),
      );
    }
    if (activeTab === "favorites") {
      return TAG_REGISTRY.filter((tag) => favorites.includes(tag.id)).map(
        (tag) => toChipItem(tag, uiLang),
      );
    }
    if (activeTab === "recent") {
      return recent.map((text) => {
        const match = TAG_REGISTRY.find((tag) => tag.insert === text);
        return match ? toChipItem(match, uiLang) : toCustomChipItem(text);
      });
    }
    return TAG_REGISTRY.filter((tag) => tag.category === activeTab).map(
      (tag) => toChipItem(tag, uiLang),
    );
  }, [query, uiLang, activeTab, favorites, recent]);

  const emptyMessage = query.trim()
    ? t("tts.tags.emptySearch")
    : activeTab === "favorites"
      ? t("tts.tags.emptyFavorites")
      : activeTab === "recent"
        ? t("tts.tags.emptyRecent")
        : null;

  return (
    <div className="rounded-lg border border-mid-gray/20">
      <div className="flex items-center gap-2 px-2 py-1.5">
        <span className="shrink-0 text-xs font-semibold uppercase tracking-wide text-text/50">
          {t("tts.tags.title")}
        </span>
        {isOpen && (
          <div className="relative min-w-0 flex-1">
            <Search
              width={14}
              height={14}
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-text/40"
            />
            <Input
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("tts.tags.searchPlaceholder")}
              className="w-full pl-7"
            />
          </div>
        )}
        <button
          type="button"
          onClick={() => setIsOpenRaw(isOpen ? "0" : "1")}
          title={isOpen ? t("tts.tags.collapse") : t("tts.tags.expand")}
          aria-label={isOpen ? t("tts.tags.collapse") : t("tts.tags.expand")}
          className="shrink-0 cursor-pointer rounded-md p-1 text-text/50 transition-colors hover:bg-mid-gray/20 hover:text-text"
        >
          {isOpen ? (
            <ChevronUp width={16} height={16} />
          ) : (
            <ChevronDown width={16} height={16} />
          )}
        </button>
      </div>

      {isOpen && (
        <>
          <div
            role="tablist"
            aria-label={t("tts.tags.tabsAriaLabel")}
            className="flex flex-wrap gap-1 px-2 pt-1"
          >
            <TabButton
              active={activeTab === "favorites"}
              icon={Star}
              label={t("tts.tags.favorites")}
              onClick={() => setActiveTabRaw("favorites")}
            />
            <TabButton
              active={activeTab === "recent"}
              icon={Clock}
              label={t("tts.tags.recent")}
              onClick={() => setActiveTabRaw("recent")}
            />
            {TAG_CATEGORIES.map((category) => (
              <TabButton
                key={category.id}
                active={activeTab === category.id}
                icon={category.icon}
                label={t(`tts.tags.categories.${category.id}`)}
                onClick={() => setActiveTabRaw(category.id)}
              />
            ))}
          </div>

          <div className="flex flex-wrap gap-1 p-2">
            {visibleTags.length === 0 ? (
              <p className="px-1 py-2 text-xs text-text/50">
                {emptyMessage}
              </p>
            ) : (
              visibleTags.map((item) => {
                const isFavorite =
                  item.registryId !== undefined &&
                  favorites.includes(item.registryId);
                return (
                  <div key={item.key} className="relative inline-flex">
                    {/* p-3: der sichtbare Chip bleibt ~22px hoch, das Polster
                        hebt die Klickflaeche auf ~46px — ueber der 44px-
                        Mindestvorgabe fuer Touch-Ziele. */}
                    <TagChip
                      label={item.label}
                      onClick={() => insertItem(item)}
                      title={item.description}
                      className="p-3"
                    />
                    {item.registryId && (
                      <button
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          toggleFavorite(item.registryId!);
                        }}
                        title={
                          isFavorite
                            ? t("tts.tags.favoriteRemove", { tag: item.label })
                            : t("tts.tags.favoriteAdd", { tag: item.label })
                        }
                        aria-label={
                          isFavorite
                            ? t("tts.tags.favoriteRemove", { tag: item.label })
                            : t("tts.tags.favoriteAdd", { tag: item.label })
                        }
                        className="absolute -top-1.5 -right-1.5 cursor-pointer rounded-full border border-mid-gray/30 bg-background p-0.5 text-text/40 transition-colors hover:text-logo-primary"
                      >
                        <Star
                          width={10}
                          height={10}
                          className={
                            isFavorite
                              ? "fill-logo-primary text-logo-primary"
                              : undefined
                          }
                        />
                      </button>
                    )}
                  </div>
                );
              })
            )}
          </div>

          <div className="flex items-center gap-2 border-t border-mid-gray/10 px-2 pt-1.5 pb-2">
            <Input
              type="text"
              value={customText}
              onChange={(event) => setCustomText(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  insertCustom();
                }
              }}
              placeholder={t("tts.tags.customPlaceholder")}
              title={t("tts.tags.customHint")}
              className="min-w-0 flex-1"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={insertCustom}
              disabled={!customText.trim()}
            >
              <Plus width={14} height={14} />
              {t("tts.tags.customInsert")}
            </Button>
          </div>
        </>
      )}
    </div>
  );
};
