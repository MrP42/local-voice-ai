import React, { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { LucideIcon } from "lucide-react";
import { List } from "lucide-react";
import {
  ChevronDown,
  ChevronUp,
  Clock,
  Plus,
  Search,
  Star,
} from "lucide-react";
import { usePersistentState } from "@/hooks/usePersistentState";
import { useSettings } from "@/hooks/useSettings";
import { Input } from "@/components/ui/Input";
import { Button } from "@/components/ui/Button";
import {
  TAG_CATEGORIES,
  TAG_REGISTRY,
  localizedLabel,
  resolveTag,
  searchTags,
  tagClass,
  tagInsertFor,
} from "@/lib/tags/registry";
import { useTagLanguage } from "./tagLanguage";
import type { TagCategoryId, TagDef } from "@/lib/tags/types";
import { TagChip } from "./TagChip";

/** "Zuletzt benutzt" merkt sich hoechstens diese vielen Eintraege. */
const MAX_RECENT = 8;

type ActiveTab = "favorites" | "recent" | "all" | TagCategoryId;

const isTagCategory = (value: string): value is TagCategoryId =>
  TAG_CATEGORIES.some((category) => category.id === value);

const isActiveTab = (value: string): value is ActiveTab =>
  value === "favorites" ||
  value === "recent" ||
  value === "all" ||
  isTagCategory(value);

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
  /** Steht das Tag in der offiziellen Fish-Audio-Liste (oder ist es eine
   *  Pause, die die App selbst erzeugt)? Sonst: bernstein statt gelb. */
  reliable: boolean;
}

const toChipItem = (tag: TagDef, uiLang: string, tagLang = "en"): ChipItem => ({
  key: tag.id,
  insertText: tagInsertFor(tag, tagLang),
  label: localizedLabel(tag, uiLang),
  description: uiLang === "de" ? tag.description?.de : tag.description?.en,
  registryId: tag.id,
  // Fish nimmt jedes Tag; die Palette markiert nur die Doku-Beispiele.
  reliable: true,
});

const toCustomChipItem = (text: string): ChipItem => ({
  key: `custom:${text}`,
  insertText: text,
  label: text,
  reliable: false,
});

/** Filterchip der Palette. Bewusst neutral (nicht gelb, keine Reiter-Optik):
 *  Gelb gehoert der einen Hauptaktion "Abspielen". Die Rolle "tab" bleibt, weil
 *  genau ein Filter aktiv ist. */
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
    className={`inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs whitespace-nowrap transition-colors cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-text ${
      active
        ? "bg-mid-gray/20 font-medium text-text"
        : "text-text/60 hover:text-text"
    }`}
  >
    <Icon width={13} height={13} aria-hidden="true" />
    {label}
  </button>
);

/**
 * Einzeilige, waagrecht scrollende Filterleiste mit weichem Verlauf am linken
 * und rechten Rand, solange dort noch Reiter verborgen sind. Die Verläufe sind
 * absolut über der Leiste (kein Layoutsprung) und schlucken keine Klicks; sie
 * blenden nur per Deckkraft ein und aus.
 */
const TabStrip: React.FC<{
  label: string;
  children: React.ReactNode;
}> = ({ label, children }) => {
  const listRef = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState({ start: false, end: false });

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const update = () => {
      // 1 px Toleranz: Bruchpixel beim Skalieren.
      const start = list.scrollLeft > 1;
      const end = list.scrollLeft + list.clientWidth < list.scrollWidth - 1;
      setMore((old) =>
        old.start === start && old.end === end ? old : { start, end },
      );
    };
    update();
    list.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(list);
    return () => {
      list.removeEventListener("scroll", update);
      observer.disconnect();
    };
  }, []);

  const fade =
    "pointer-events-none absolute inset-y-0 z-[1] w-8 transition-opacity duration-150 motion-reduce:transition-none";
  return (
    <div className="relative">
      <div
        ref={listRef}
        role="tablist"
        aria-label={label}
        // Einzeilig und waagrecht scrollend statt dreizeilig umbrechend
        // (A24); die Bildlaufleiste bleibt unsichtbar, das Mausrad
        // scrollt waagrecht mit.
        onWheel={(event) => {
          if (event.deltaY === 0) return;
          event.currentTarget.scrollLeft += event.deltaY;
        }}
        className="flex flex-nowrap gap-1 overflow-x-auto px-2 pt-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {children}
      </div>
      <div
        aria-hidden="true"
        data-testid="tag-tabs-fade-start"
        className={`${fade} left-0 bg-[linear-gradient(to_right,var(--color-background),transparent)] ${
          more.start ? "opacity-100" : "opacity-0"
        }`}
      />
      <div
        aria-hidden="true"
        data-testid="tag-tabs-fade-end"
        className={`${fade} right-0 bg-[linear-gradient(to_left,var(--color-background),transparent)] ${
          more.end ? "opacity-100" : "opacity-0"
        }`}
      />
    </div>
  );
};

/**
 * Die Tag-Palette: Suche, Favoriten/Zuletzt/Kategorien-Reiter, ein Chip-Grid
 * und eine Freitext-Zeile fuer Tags, die nicht in der Registry stehen.
 * Eigenstaendig und ueberall einbettbar — ein spaeteres Paket montiert sie in
 * die Vorlesen-Seite; hier steht nur `onInsert` zwischen ihr und dem Text.
 */
export const TagPalette: React.FC<{
  onInsert: (tagText: string) => void;
  /** Pointer-Drag aus der Palette (kein HTML5-Drag&Drop — das kollidiert
   *  mit dem Tauri-File-Drop): bekommt die Viewport-Koordinaten des
   *  Loslassens und den fertigen Klammertext; `true` heißt eingefügt
   *  (zählt dann für „Zuletzt"). */
  onDragInsert?: (x: number, y: number, tagText: string) => boolean;
  uiLang: string;
}> = ({ onInsert, onDragInsert, uiLang }) => {
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
  const tagLang = useTagLanguage();

  const rememberRecent = (insertText: string) => {
    const next = [
      insertText,
      ...recent.filter((entry) => entry !== insertText),
    ].slice(0, MAX_RECENT);
    setRecentRaw(JSON.stringify(next));
  };

  /** Ghost-Chip, der beim Pointer-Drag dem Cursor folgt. */
  const [ghost, setGhost] = useState<{
    x: number;
    y: number;
    label: string;
  } | null>(null);
  const suppressClickRef = useRef(false);

  const insertItem = (item: ChipItem) => {
    // Nach einem Drag feuert der Browser auf dem Ursprungs-Chip noch ein
    // click — das darf nicht ZUSÄTZLICH am Cursor einfügen.
    if (suppressClickRef.current) return;
    onInsert(`[${item.insertText}]`);
    rememberRecent(item.insertText);
  };

  const handleChipPointerDown =
    (item: ChipItem) => (event: React.PointerEvent<HTMLButtonElement>) => {
      if (!onDragInsert) return;
      if (event.pointerType === "mouse" && event.button !== 0) return;
      const el = event.currentTarget;
      const drag = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        moved: false,
      };
      // Sofort einfangen: sonst enden die Move-Ereignisse, sobald der
      // Zeiger den Chip verlässt. Ein normaler Klick bleibt ein Klick.
      try {
        el.setPointerCapture(drag.pointerId);
      } catch {
        // z. B. Pointer schon weg — dann eben kein Drag.
      }
      const onMove = (ev: PointerEvent) => {
        if (ev.pointerId !== drag.pointerId) return;
        if (!drag.moved) {
          if (
            Math.hypot(ev.clientX - drag.startX, ev.clientY - drag.startY) < 5
          ) {
            return;
          }
          drag.moved = true;
        }
        setGhost({ x: ev.clientX, y: ev.clientY, label: item.label });
      };
      const finish = (ev: PointerEvent, drop: boolean) => {
        if (ev.pointerId !== drag.pointerId) return;
        el.removeEventListener("pointermove", onMove);
        el.removeEventListener("pointerup", onUp);
        el.removeEventListener("pointercancel", onCancel);
        try {
          el.releasePointerCapture(drag.pointerId);
        } catch {
          // schon freigegeben
        }
        setGhost(null);
        if (drag.moved) {
          suppressClickRef.current = true;
          // click feuert (wenn überhaupt) synchron direkt nach pointerup —
          // der Timeout räumt die Sperre danach zuverlässig wieder ab.
          window.setTimeout(() => {
            suppressClickRef.current = false;
          }, 0);
          if (drop) {
            const inserted = onDragInsert(
              ev.clientX,
              ev.clientY,
              `[${item.insertText}]`,
            );
            if (inserted) rememberRecent(item.insertText);
          }
        }
      };
      const onUp = (ev: PointerEvent) => finish(ev, true);
      const onCancel = (ev: PointerEvent) => finish(ev, false);
      el.addEventListener("pointermove", onMove);
      el.addEventListener("pointerup", onUp);
      el.addEventListener("pointercancel", onCancel);
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
        toChipItem(tag, uiLang, tagLang),
      );
    }
    if (activeTab === "favorites") {
      return TAG_REGISTRY.filter((tag) => favorites.includes(tag.id)).map(
        (tag) => toChipItem(tag, uiLang, tagLang),
      );
    }
    if (activeTab === "recent") {
      return recent.map((text) => {
        const match = resolveTag(text);
        return match
          ? toChipItem(match, uiLang, tagLang)
          : toCustomChipItem(text);
      });
    }
    if (activeTab === "all") {
      // Die ganze Liste, dokumentierte zuerst -- zum Stoebern und Pruefen.
      return [...TAG_REGISTRY]
        .sort(
          (a, b) =>
            Number(tagClass(b) === "documented") -
            Number(tagClass(a) === "documented"),
        )
        .map((tag) => toChipItem(tag, uiLang, tagLang));
    }
    return TAG_REGISTRY.filter((tag) => tag.category === activeTab).map((tag) =>
      toChipItem(tag, uiLang, tagLang),
    );
  }, [query, uiLang, tagLang, activeTab, favorites, recent]);

  const emptyMessage = query.trim()
    ? t("tts.tags.emptySearch")
    : activeTab === "favorites"
      ? t("tts.tags.emptyFavorites")
      : activeTab === "recent"
        ? t("tts.tags.emptyRecent")
        : null;

  return (
    <div className="rounded-lg border border-mid-gray/20">
      {/* Kopf (Titel, Suche) und Filterreiter kleben oben, wenn die Palette
          in einem scrollenden Klappbereich liegt: 44px tiefer als die
          Ueberschrift des Klappbereichs (sie ist selbst 44px hoch). */}
      <div className="sticky top-11 z-[1] rounded-t-lg bg-background pb-1">
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
          <TabStrip label={t("tts.tags.tabsAriaLabel")}>
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
            <TabButton
              active={activeTab === "all"}
              icon={List}
              label={t("tts.tags.all")}
              onClick={() => setActiveTabRaw("all")}
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
          </TabStrip>
        )}
      </div>

      {isOpen && (
        <>
          {/* Legende: was die drei Farben bedeuten -- einmal, nicht als
              Tooltip auf jedem Chip. */}
          <p
            className="px-2 pt-1 text-xs text-text/50"
            data-testid="tag-legend"
          >
            {t("tts.tags.legend", {
              documented: TAG_REGISTRY.filter(
                (tag) => tagClass(tag) === "documented",
              ).length,
              total: TAG_REGISTRY.length,
            })}
          </p>
          <div className="flex flex-wrap gap-x-1 gap-y-0.5 p-2">
            {visibleTags.length === 0 ? (
              <p className="px-1 py-2 text-xs text-text/50">{emptyMessage}</p>
            ) : (
              visibleTags.map((item) => {
                const isFavorite =
                  item.registryId !== undefined &&
                  favorites.includes(item.registryId);
                return (
                  <div key={item.key} className="relative inline-flex">
                    {/* p-1: Am Desktop reicht eine kompakte Klickflaeche (die
                        Pille bleibt ~20px hoch, mit Polster ~28px, ueber den
                        24px des WCAG 2.5.8); p-3 (~46px fuer Touch) machte
                        95 Tags zu 11 Zeilen a 47px. */}
                    <TagChip
                      label={item.label}
                      state={item.reliable ? "normal" : "unverified"}
                      onClick={() => insertItem(item)}
                      onPointerDown={
                        onDragInsert ? handleChipPointerDown(item) : undefined
                      }
                      title={
                        item.reliable
                          ? item.description
                          : [item.description, t("tts.tags.undocumentedHint")]
                              .filter(Boolean)
                              .join(" — ")
                      }
                      className="p-1"
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
                        className="group absolute -top-[12px] -right-[12px] flex size-[24px] cursor-pointer items-center justify-center text-text/40 transition-colors hover:text-logo-primary"
                      >
                        {/* Klickflaeche 24x24 px (WCAG 2.5.8; px statt rem, die Wurzelschrift
                            ist 15 px), sichtbar bleibt
                            der kleine runde Stern in der Chip-Ecke. */}
                        <span className="rounded-full border border-mid-gray/30 bg-background p-0.5">
                          <Star
                            width={12}
                            height={12}
                            className={
                              isFavorite
                                ? "fill-logo-primary text-logo-primary"
                                : undefined
                            }
                          />
                        </span>
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

      {ghost &&
        createPortal(
          <span
            aria-hidden="true"
            style={{ left: ghost.x, top: ghost.y }}
            className="pointer-events-none fixed z-[60] -translate-x-1/2 -translate-y-1/2 rounded border border-logo-primary/40 bg-background px-2 py-1 text-xs font-medium text-text shadow-lg"
          >
            {ghost.label}
          </span>,
          document.body,
        )}
    </div>
  );
};
