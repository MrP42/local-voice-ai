import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Star, Trash2 } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { Input } from "@/components/ui/Input";
import { TAG_REGISTRY, localizedLabel, searchTags } from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";
import type {
  ChipMatch,
  ChipPopoverApi,
  ChipProvider,
} from "../editor/TtsChipEditor";

/**
 * TS-Spiegel der Rust-Regel `tag_spans` aus
 * `src-tauri/src/managers/tts/protocol.rs`: Ein Tag reicht von `[` bis zum
 * nächsten `]` OHNE Zeilenumbruch dazwischen — eine vergessene schließende
 * Klammer darf nie den Resttext zum Tag machen. Keine Verschachtelung: das
 * erste `]` schließt, auch wenn davor ein weiteres `[` steht. Rust liefert
 * Byte-Ranges, hier sind es UTF-16-Offsets — da `[`, `]` und `\n` ASCII
 * sind, bezeichnen beide exakt dieselben Spans.
 */
export function scanTagMatches(text: string): ChipMatch[] {
  const matches: ChipMatch[] = [];
  let i = 0;
  while (i < text.length) {
    if (text[i] === "[") {
      let closed = -1;
      for (let j = i + 1; j < text.length; j++) {
        const c = text[j];
        if (c === "]") {
          closed = j;
          break;
        }
        if (c === "\n") break;
      }
      if (closed !== -1) {
        matches.push({
          start: i,
          end: closed + 1,
          raw: text.slice(i, closed + 1),
        });
        i = closed + 1;
        continue;
      }
    }
    i++;
  }
  return matches;
}

/** Registry-Eintrag zum Klammerinhalt — case-insensitiv über `insert`;
 *  Freitext-Tags (S2-Pro versteht sie) haben keinen Eintrag. */
const findTagDef = (inner: string): TagDef | undefined => {
  const key = inner.trim().toLowerCase();
  return TAG_REGISTRY.find((tag) => tag.insert.toLowerCase() === key);
};

/** Höchstens so viele Zeilen zeigt die Ersetzen-Liste im Popover. */
const MAX_POPOVER_RESULTS = 30;

/**
 * Popover eines Tag-Chips: Suchfeld mit Vorschlagsliste (`searchTags`),
 * ↑/↓/Enter wählt, Klick ersetzt das Tag; dazu Löschen und Favoriten-Stern.
 */
const TagChipPopover: React.FC<{ match: ChipMatch; api: ChipPopoverApi }> = ({
  match,
  api,
}) => {
  const { t, i18n } = useTranslation();
  const uiLang = i18n.language?.split("-")[0] ?? "en";
  const { getSetting, updateSetting } = useSettings();

  const inner = match.raw.slice(1, -1).trim();
  const def = findTagDef(inner);
  const label = def ? localizedLabel(def, uiLang) : inner;

  const favorites = getSetting("tts_tag_favorites") ?? [];
  const isFavorite = def !== undefined && favorites.includes(def.id);

  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const results = useMemo(
    () => searchTags(query.trim(), uiLang).slice(0, MAX_POPOVER_RESULTS),
    [query, uiLang],
  );
  useEffect(() => {
    setSelected(0);
  }, [query]);
  const clampedSelected = Math.min(selected, Math.max(0, results.length - 1));

  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [clampedSelected, results]);

  const replaceWith = (tag: TagDef) => {
    api.replaceRange(match.start, match.end, `[${tag.insert}]`);
    api.close();
  };

  const remove = () => {
    api.replaceRange(match.start, match.end, "");
    api.close();
  };

  const toggleFavorite = () => {
    if (!def) return;
    const next = isFavorite
      ? favorites.filter((id) => id !== def.id)
      : [...favorites, def.id];
    void updateSetting("tts_tag_favorites", next);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (results.length === 0) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelected((clampedSelected + 1) % results.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelected((clampedSelected - 1 + results.length) % results.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      replaceWith(results[clampedSelected]);
    }
  };

  return (
    <div className="w-72">
      <div className="flex items-center gap-1 border-b border-mid-gray/15 px-2 py-1.5">
        <span
          className="min-w-0 flex-1 truncate text-sm font-medium text-text"
          title={
            def && uiLang === "de" ? def.description?.de : def?.description?.en
          }
        >
          {label}
        </span>
        {def && (
          <button
            type="button"
            onClick={toggleFavorite}
            title={
              isFavorite
                ? t("tts.tags.favoriteRemove", { tag: label })
                : t("tts.tags.favoriteAdd", { tag: label })
            }
            aria-label={
              isFavorite
                ? t("tts.tags.favoriteRemove", { tag: label })
                : t("tts.tags.favoriteAdd", { tag: label })
            }
            aria-pressed={isFavorite}
            className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/50 transition-colors hover:bg-mid-gray/15 hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
          >
            <Star
              width={15}
              height={15}
              aria-hidden="true"
              className={
                isFavorite ? "fill-logo-primary text-logo-primary" : undefined
              }
            />
          </button>
        )}
        <button
          type="button"
          onClick={remove}
          title={t("tts.editor.deleteTag")}
          aria-label={t("tts.editor.deleteTag")}
          className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/50 transition-colors hover:bg-mid-gray/15 hover:text-red-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        >
          <Trash2 width={15} height={15} aria-hidden="true" />
        </button>
      </div>

      <div className="px-2 pt-2">
        <Input
          ref={inputRef}
          type="text"
          variant="compact"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={t("tts.tags.searchPlaceholder")}
          aria-label={t("tts.tags.searchPlaceholder")}
          className="w-full"
        />
        <p className="px-1 pt-1 text-[11px] leading-4 text-text/45">
          {t("tts.editor.replaceHint")}
        </p>
      </div>

      <div
        ref={listRef}
        role="listbox"
        aria-label={t("tts.tags.searchPlaceholder")}
        className="max-h-64 overflow-y-auto p-1"
      >
        {results.length === 0 ? (
          <p className="px-2 py-3 text-xs text-text/50">
            {t("tts.tags.emptySearch")}
          </p>
        ) : (
          results.map((tag, index) => (
            <button
              key={tag.id}
              type="button"
              role="option"
              aria-selected={index === clampedSelected}
              onClick={() => replaceWith(tag)}
              onMouseEnter={() => setSelected(index)}
              title={
                uiLang === "de" ? tag.description?.de : tag.description?.en
              }
              className={`flex min-h-[44px] w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2 text-start text-sm ${
                index === clampedSelected
                  ? "bg-logo-primary/15 text-text"
                  : "text-text/80"
              }`}
            >
              <span className="truncate">{localizedLabel(tag, uiLang)}</span>
              <span className="shrink-0 text-xs text-text/45">
                [{tag.insert}]
              </span>
            </button>
          ))
        )}
      </div>
    </div>
  );
};

/**
 * Der Tag-Provider für den `TtsChipEditor` — erfüllt den bindenden
 * `ChipProvider`-Vertrag aus dem Editor. Alle Tags sind `state: "ok"`:
 * S2-Pro versteht auch Freitext in eckigen Klammern, es gibt hier also
 * keine „ungültigen" Tags.
 */
export function useTagProvider(): ChipProvider {
  const { i18n } = useTranslation();
  const uiLang = i18n.language?.split("-")[0] ?? "en";

  return useMemo<ChipProvider>(
    () => ({
      id: "tag",
      scan: scanTagMatches,
      render: (m) => {
        const inner = m.raw.slice(1, -1).trim();
        const def = findTagDef(inner);
        return {
          label: def ? localizedLabel(def, uiLang) : inner,
          state: "ok",
        };
      },
      popover: (m, api) => <TagChipPopover match={m} api={api} />,
    }),
    [uiLang],
  );
}
