import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import {
  Avatar,
  Button as FluentButton,
  Menu,
  MenuDivider,
  MenuGroupHeader,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Popover,
  PopoverSurface,
  PopoverTrigger,
  SearchBox,
  ToggleButton,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";
import { Bell, HelpCircle, Settings, User } from "lucide-react";
import { Dialog } from "../ui/Dialog";
import { useSettings } from "../../hooks/useSettings";
import { togglePageHelp, useHasPageHelp } from "../help/pageHelp";
import { SECTIONS_CONFIG, type SidebarSection } from "../Sidebar";
import { APP_NAME } from "./frame";

interface TitlebarToolsProps {
  currentSection: SidebarSection;
  onNavigate: (section: SidebarSection) => void;
}

const ICON = 18;
const IS_MAC =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform);

/**
 * Werkzeuge rechts in der Titelleiste, vor den Fensterknöpfen: Suche
 * (Strg+K), Benachrichtigungen, Hilfe, Einstellungen und Profil. Alles
 * Bedienelemente der Suite-Rahmens; die Suche durchsucht vorerst die
 * Bereichsnamen der Navigation.
 */
export const TitlebarTools: React.FC<TitlebarToolsProps> = ({
  currentSection,
  onNavigate,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [version, setVersion] = useState("");
  const hasPageHelp = useHasPageHelp();
  const blurTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(blurTimer.current), []);
  const searchKey = IS_MAC ? "⌘K" : t("titlebar.searchKey");
  const { settings } = useSettings();

  // Strg+K (macOS: Cmd+K) springt ins Suchfeld, von überall in der App.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        (IS_MAC ? event.metaKey : event.ctrlKey) &&
        !event.altKey &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!aboutOpen || version) return;
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
  }, [aboutOpen, version]);

  const sections = useMemo(
    () =>
      (Object.keys(SECTIONS_CONFIG) as SidebarSection[]).map((id) => ({
        id,
        label: t(SECTIONS_CONFIG[id].labelKey),
        Icon: SECTIONS_CONFIG[id].icon,
      })),
    [t],
  );
  const needle = query.trim().toLowerCase();
  const results = needle
    ? sections.filter((s) => s.label.toLowerCase().includes(needle))
    : [];

  const go = (id: SidebarSection) => {
    onNavigate(id);
    setQuery("");
    setSearchOpen(false);
    inputRef.current?.blur();
  };

  const shortcuts = Object.values(settings?.bindings ?? {}).filter(
    (b): b is NonNullable<typeof b> => !!b,
  );

  return (
    <div className="lv-titlebar__tools" role="group" aria-label={APP_NAME}>
      <div className="lv-search">
        <SearchBox
          ref={inputRef}
          size="medium"
          placeholder={t("titlebar.searchPlaceholder", { key: searchKey })}
          input={{
            role: "combobox",
            "aria-autocomplete": "list",
            "aria-expanded": searchOpen && !!needle,
            "aria-controls": "lv-search-list",
            "aria-activedescendant":
              searchOpen && results[active]
                ? `lv-search-opt-${results[active].id}`
                : undefined,
          }}
          aria-label={t("titlebar.search")}
          value={query}
          onChange={(_e, data) => {
            setQuery(data.value);
            setSearchOpen(true);
            setActive(0);
          }}
          onFocus={() => setSearchOpen(true)}
          onBlur={() => {
            window.clearTimeout(blurTimer.current);
            blurTimer.current = window.setTimeout(
              () => setSearchOpen(false),
              120,
            );
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setQuery("");
              setSearchOpen(false);
              inputRef.current?.blur();
            } else if (event.key === "ArrowDown" && results.length) {
              event.preventDefault();
              setActive((i) => (i + 1) % results.length);
            } else if (event.key === "ArrowUp" && results.length) {
              event.preventDefault();
              setActive((i) => (i - 1 + results.length) % results.length);
            } else if (event.key === "Enter" && results[active]) {
              event.preventDefault();
              go(results[active].id);
            }
          }}
          data-titlebar-search
        />
        {searchOpen && needle && (
          <ul
            id="lv-search-list"
            className="lv-search__list"
            role="listbox"
            aria-label={t("titlebar.searchResults")}
          >
            {results.length === 0 && (
              <li className="lv-search__none">{t("titlebar.searchNone")}</li>
            )}
            {results.map(({ id, label, Icon }, i) => (
              <li
                key={id}
                id={`lv-search-opt-${id}`}
                role="option"
                aria-selected={i === active}
                className="lv-search__item"
                data-active={i === active}
                // mousedown statt click: das Feld verliert sonst vorher den
                // Fokus und blendet die Liste aus.
                onMouseDown={(event) => {
                  event.preventDefault();
                  go(id);
                }}
              >
                <Icon size={16} aria-hidden="true" />
                {label}
              </li>
            ))}
          </ul>
        )}
      </div>

      <Popover positioning="below-end" withArrow>
        <FluentTooltip
          content={t("titlebar.notifications")}
          relationship="label"
        >
          <PopoverTrigger disableButtonEnhancement>
            <FluentButton
              appearance="subtle"
              icon={<Bell size={ICON} aria-hidden="true" />}
              data-titlebar-notifications
            />
          </PopoverTrigger>
        </FluentTooltip>
        <PopoverSurface aria-label={t("titlebar.notifications")}>
          <p className="lv-empty-note">{t("titlebar.noNotifications")}</p>
        </PopoverSurface>
      </Popover>

      <Menu>
        <FluentTooltip content={t("titlebar.help")} relationship="label">
          <MenuTrigger disableButtonEnhancement>
            <FluentButton
              appearance="subtle"
              icon={<HelpCircle size={ICON} aria-hidden="true" />}
              data-titlebar-help
            />
          </MenuTrigger>
        </FluentTooltip>
        <MenuPopover>
          <MenuList>
            <MenuItem disabled={!hasPageHelp} onClick={togglePageHelp}>
              {t("titlebar.helpPage")}
            </MenuItem>
            <MenuItem onClick={() => setShortcutsOpen(true)}>
              {t("titlebar.shortcuts")}
            </MenuItem>
            <MenuDivider />
            <MenuItem onClick={() => setAboutOpen(true)}>
              {t("titlebar.about")}
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>

      <FluentTooltip content={t("titlebar.settings")} relationship="label">
        <ToggleButton
          appearance="subtle"
          checked={currentSection === "settings"}
          aria-pressed={currentSection === "settings"}
          icon={<Settings size={ICON} aria-hidden="true" />}
          onClick={() => onNavigate("settings")}
          data-titlebar-settings
        />
      </FluentTooltip>

      <Menu>
        <FluentTooltip content={t("titlebar.profile")} relationship="label">
          <MenuTrigger disableButtonEnhancement>
            <FluentButton
              appearance="subtle"
              className="lv-titlebar__profile"
              aria-label={t("titlebar.profile")}
              data-titlebar-profile
              icon={
                <Avatar
                  size={28}
                  color="brand"
                  aria-hidden="true"
                  icon={<User size={16} aria-hidden="true" />}
                />
              }
            />
          </MenuTrigger>
        </FluentTooltip>
        <MenuPopover>
          <MenuList>
            <MenuGroupHeader>{t("titlebar.localProfile")}</MenuGroupHeader>
            <MenuItem onClick={() => onNavigate("settings")}>
              {t("sidebar.settings")}
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>

      <Dialog
        open={aboutOpen}
        onOpenChange={setAboutOpen}
        title={t("titlebar.about")}
        closeLabel={t("titlebar.close")}
        className="max-w-sm"
      >
        <div className="space-y-1" data-about-dialog>
          <p className="text-base font-semibold">{t("titlebar.aboutName")}</p>
          <p className="text-sm text-text-muted">{t("titlebar.aboutSuite")}</p>
          {version && (
            <p className="text-sm text-text-muted">
              {t("titlebar.aboutVersion", { version })}
            </p>
          )}
        </div>
      </Dialog>

      <Dialog
        open={shortcutsOpen}
        onOpenChange={setShortcutsOpen}
        title={t("titlebar.shortcutsTitle")}
        closeLabel={t("titlebar.close")}
        className="max-w-md"
      >
        <dl className="lv-shortcuts">
          {shortcuts.map((b) => (
            <React.Fragment key={b.id}>
              <dt>{b.name}</dt>
              <dd>
                <kbd>{b.current_binding}</kbd>
              </dd>
            </React.Fragment>
          ))}
          <dt>{t("titlebar.shortcutSearch")}</dt>
          <dd>
            <kbd>{searchKey}</kbd>
          </dd>
        </dl>
      </Dialog>
    </div>
  );
};
