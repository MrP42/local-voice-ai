import React, {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  ClipboardPaste,
  Copy,
  Scissors,
  Tag,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { Input } from "@/components/ui/Input";
import {
  TAG_CATEGORIES,
  TAG_REGISTRY,
  localizedLabel,
  searchTags,
} from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";

interface TagContextMenuProps {
  x: number;
  y: number;
  hasSelection: boolean;
  onClose: () => void;
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  /** Bekommt den fertigen Klammertext, z. B. `[whisper]`. */
  onInsertTag: (tagText: string) => void;
}

/** Höchstens so viele Suchtreffer zeigt die Tag-Zeile im Menü. */
const MAX_MENU_RESULTS = 12;

const MenuRow: React.FC<{
  icon: LucideIcon;
  label: string;
  disabled?: boolean;
  onClick: () => void;
}> = ({ icon: Icon, label, disabled = false, onClick }) => (
  <button
    type="button"
    role="menuitem"
    disabled={disabled}
    onClick={onClick}
    className="flex min-h-[44px] w-full cursor-pointer items-center gap-2 px-3 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:bg-mid-gray/15 focus-visible:text-text disabled:cursor-default disabled:text-text/30 disabled:hover:bg-transparent"
  >
    <Icon width={15} height={15} aria-hidden="true" />
    {label}
  </button>
);

const TagRow: React.FC<{
  tag: TagDef;
  uiLang: string;
  onPick: (tag: TagDef) => void;
}> = ({ tag, uiLang, onPick }) => (
  <button
    type="button"
    role="menuitem"
    onClick={() => onPick(tag)}
    title={uiLang === "de" ? tag.description?.de : tag.description?.en}
    className="flex min-h-[44px] w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2 text-start text-sm text-text/80 hover:bg-logo-primary/10 hover:text-text focus-visible:outline-none focus-visible:bg-logo-primary/10 focus-visible:text-text"
  >
    <span className="truncate">{localizedLabel(tag, uiLang)}</span>
    <span className="shrink-0 text-xs text-text/45">[{tag.insert}]</span>
  </button>
);

/**
 * Ersatz für das native Kontextmenü des Editors (das per `preventDefault`
 * unterdrückt wird — deshalb gehören die Grundfunktionen Ausschneiden/
 * Kopieren/Einfügen hier hinein): dazu „Tag einfügen" mit Suchzeile,
 * Favoriten und den Registry-Kategorien. Tastatur: ↑/↓ wandern, Enter wählt,
 * Escape schließt.
 */
export const TagContextMenu: React.FC<TagContextMenuProps> = ({
  x,
  y,
  hasSelection,
  onClose,
  onCut,
  onCopy,
  onPaste,
  onInsertTag,
}) => {
  const { t, i18n } = useTranslation();
  const uiLang = i18n.language?.split("-")[0] ?? "en";
  const { getSetting } = useSettings();
  const favoriteIds = getSetting("tts_tag_favorites") ?? [];
  const favorites = useMemo(
    () => TAG_REGISTRY.filter((tag) => favoriteIds.includes(tag.id)),
    // Der Settings-Hook liefert je Render ein frisches Array — der Inhalt
    // zählt, nicht die Identität.
    [favoriteIds.join("|")],
  );

  const [subOpen, setSubOpen] = useState(false);
  const [query, setQuery] = useState("");
  const results = useMemo(
    () =>
      query.trim()
        ? searchTags(query.trim(), uiLang).slice(0, MAX_MENU_RESULTS)
        : [],
    [query, uiLang],
  );

  const menuRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // Position an den Viewport klemmen — auch nach dem Aufklappen des
  // Tag-Bereichs, der das Menü höher macht.
  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const left = Math.min(
      Math.max(8, x),
      Math.max(8, window.innerWidth - width - 8),
    );
    const top = Math.min(
      Math.max(8, y),
      Math.max(8, window.innerHeight - height - 8),
    );
    setPos((prev) =>
      prev && prev.left === left && prev.top === top ? prev : { left, top },
    );
  }, [x, y, subOpen, results.length, favorites.length]);

  // Erstfokus auf den ersten Menüpunkt; beim Aufklappen auf die Suchzeile.
  useEffect(() => {
    const first = menuRef.current?.querySelector<HTMLElement>(
      "button:not(:disabled)",
    );
    first?.focus();
  }, []);
  useEffect(() => {
    if (subOpen) searchRef.current?.focus();
  }, [subOpen]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    // Wheel geht durch das Backdrop hindurch: scrollt die Seite, stünde das
    // fixierte Menü an veralteten Viewport-Koordinaten — schließen. Scrollen
    // INNERHALB des Menüs (Tag-Liste) bleibt davon unberührt. capture, weil
    // scroll-Ereignisse nicht bubbeln.
    const onScroll = (event: Event) => {
      if (
        menuRef.current &&
        event.target instanceof Node &&
        menuRef.current.contains(event.target)
      ) {
        return;
      }
      onClose();
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [onClose]);

  const moveFocus = (direction: 1 | -1) => {
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLElement>(
        "button:not(:disabled), input",
      ) ?? [],
    );
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLElement);
    const next =
      items[(index + direction + items.length) % items.length] ?? items[0];
    next.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveFocus(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveFocus(-1);
    }
  };

  const pick = (tag: TagDef) => onInsertTag(`[${tag.insert}]`);

  return createPortal(
    <>
      {/* Unsichtbarer Fang für den Klick daneben — wie beim Plus-Menü. */}
      <div
        className="fixed inset-0 z-40"
        onPointerDown={onClose}
        onContextMenu={(event) => {
          event.preventDefault();
          onClose();
        }}
      />
      <div
        ref={menuRef}
        role="menu"
        aria-label={t("tts.editor.menuAria")}
        onKeyDown={handleKeyDown}
        style={
          pos
            ? { left: pos.left, top: pos.top }
            : { left: -9999, top: 0, visibility: "hidden" }
        }
        className="fixed z-50 w-72 rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
      >
        <MenuRow
          icon={Scissors}
          label={t("tts.editor.menuCut")}
          disabled={!hasSelection}
          onClick={onCut}
        />
        <MenuRow
          icon={Copy}
          label={t("tts.editor.menuCopy")}
          disabled={!hasSelection}
          onClick={onCopy}
        />
        <MenuRow
          icon={ClipboardPaste}
          label={t("tts.editor.menuPaste")}
          onClick={onPaste}
        />
        <div className="my-1 border-t border-mid-gray/15" aria-hidden="true" />
        <button
          type="button"
          role="menuitem"
          aria-expanded={subOpen}
          onClick={() => setSubOpen((open) => !open)}
          className="flex min-h-[44px] w-full cursor-pointer items-center gap-2 px-3 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:bg-mid-gray/15 focus-visible:text-text"
        >
          <Tag width={15} height={15} aria-hidden="true" />
          <span className="flex-1">{t("tts.editor.menuInsertTag")}</span>
          {subOpen ? (
            <ChevronDown width={15} height={15} aria-hidden="true" />
          ) : (
            <ChevronRight width={15} height={15} aria-hidden="true" />
          )}
        </button>

        {subOpen && (
          <div className="border-t border-mid-gray/15 px-2 pt-2 pb-1">
            <Input
              ref={searchRef}
              type="text"
              variant="compact"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("tts.tags.searchPlaceholder")}
              aria-label={t("tts.tags.searchPlaceholder")}
              className="mb-1 w-full"
            />
            <div className="max-h-64 overflow-y-auto">
              {query.trim() ? (
                results.length === 0 ? (
                  <p className="px-2 py-3 text-xs text-text/50">
                    {t("tts.tags.emptySearch")}
                  </p>
                ) : (
                  results.map((tag) => (
                    <TagRow
                      key={tag.id}
                      tag={tag}
                      uiLang={uiLang}
                      onPick={pick}
                    />
                  ))
                )
              ) : (
                <>
                  {favorites.length > 0 && (
                    <>
                      <p className="px-2 pt-1 pb-0.5 text-xs font-semibold uppercase tracking-wide text-text/40">
                        {t("tts.tags.favorites")}
                      </p>
                      {favorites.map((tag) => (
                        <TagRow
                          key={tag.id}
                          tag={tag}
                          uiLang={uiLang}
                          onPick={pick}
                        />
                      ))}
                    </>
                  )}
                  {TAG_CATEGORIES.map((category) => (
                    <React.Fragment key={category.id}>
                      <p className="px-2 pt-1 pb-0.5 text-xs font-semibold uppercase tracking-wide text-text/40">
                        {t(`tts.tags.categories.${category.id}`)}
                      </p>
                      {TAG_REGISTRY.filter(
                        (tag) => tag.category === category.id,
                      ).map((tag) => (
                        <TagRow
                          key={tag.id}
                          tag={tag}
                          uiLang={uiLang}
                          onPick={pick}
                        />
                      ))}
                    </React.Fragment>
                  ))}
                </>
              )}
            </div>
          </div>
        )}
      </div>
    </>,
    document.body,
  );
};
