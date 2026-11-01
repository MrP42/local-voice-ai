import React, {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  Archive,
  ArchiveRestore,
  Cloud,
  MoreHorizontal,
  PanelLeftClose,
  Pencil,
  Phone,
  Pin,
  PinOff,
  Search,
  SquarePen,
  Trash2,
} from "lucide-react";

/**
 * Verlaufsliste der Gespräche: Suche, Gruppen nach Alter, Menü je Eintrag.
 *
 * Rein darstellend (ADR-0021): kein Store, kein Backend, keine Übersetzung.
 * Alles kommt über Props; die Komponente zieht später als Baustein in
 * `@wai/ui` um.
 */
export interface ConversationListItem {
  id: string;
  title: string;
  /** Letzte Aktivität (ms seit Epoche). */
  updatedAt: number;
  pinned: boolean;
  archived: boolean;
  /** Das Hauptgespräch (Tastenkürzel und Telefon der Fußleiste setzen es fort). */
  main?: boolean;
  /** Anfang des jüngsten Beitrags; die Suche schaut auch hier hinein. */
  lastText: string;
}

export interface ConversationListLabels {
  navLabel: string;
  newConversation: string;
  /** Name des Lupenknopfs (öffnet die Suche). */
  search: string;
  /** Tastenkürzel der Suche, im Tooltip der Lupe. */
  searchKey: string;
  /** Name und Tooltip des Verlauf-Symbols im Spaltenkopf. */
  collapse: string;
  groupPinned: string;
  groupToday: string;
  groupWeek: string;
  groupOlder: string;
  empty: string;
  /** Beschriftung des Menüknopfs (und des Menüs) eines Eintrags. */
  menu: (title: string) => string;
  rename: string;
  renameLabel: string;
  pin: string;
  unpin: string;
  archive: string;
  unarchive: string;
  delete: string;
  openInCorporate: string;
  corporateHint: string;
  confirmDelete: (title: string) => string;
  confirmYes: string;
  confirmNo: string;
  showArchived: string;
  archivedBadge: string;
  /** Beschriftung des Hauptgespräch-Symbols. */
  mainBadge: string;
}

export interface ConversationListProps {
  items: ConversationListItem[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  /** Kennung der Spalte (aria-controls des Verlauf-Symbols). */
  controlsId: string;
  /** Spalte einklappen (Symbol im Kopf). */
  onCollapse: () => void;
  /** Suche öffnen (Lupe im Kopf). */
  onSearch: () => void;
  onRename: (id: string, title: string) => void;
  onPin: (id: string, pinned: boolean) => void;
  onArchive: (id: string, archived: boolean) => void;
  onDelete: (id: string) => void;
  showArchived: boolean;
  onShowArchivedChange: (show: boolean) => void;
  labels: ConversationListLabels;
  /** Sprache für Uhrzeit und Datum (BCP 47). */
  locale: string;
  /** „Jetzt“ in ms; nur für Tests und Screenshots. */
  now?: number;
}

export type ConversationGroupId = "pinned" | "today" | "week" | "older";

export interface ConversationGroup {
  id: ConversationGroupId;
  items: ConversationListItem[];
}

const DAY_MS = 86_400_000;
const MENU_WIDTH = 224;
const MENU_HEIGHT = 232;
const MAX_TITLE = 200;

/** Gruppiert nach Anheften und Alter; die Reihenfolge innerhalb bleibt wie geliefert. */
export function groupConversations(
  items: ConversationListItem[],
  now: number,
): ConversationGroup[] {
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  const today = midnight.getTime();
  const week = today - 6 * DAY_MS;
  const groups: ConversationGroup[] = [
    { id: "pinned", items: [] },
    { id: "today", items: [] },
    { id: "week", items: [] },
    { id: "older", items: [] },
  ];
  for (const item of items) {
    const index = item.pinned
      ? 0
      : item.updatedAt >= today
        ? 1
        : item.updatedAt >= week
          ? 2
          : 3;
    groups[index].items.push(item);
  }
  return groups.filter((g) => g.items.length > 0);
}

/** Titel und letzter Text, ohne Rücksicht auf Groß- und Kleinschreibung. */
export function matchesQuery(item: ConversationListItem, query: string) {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return true;
  return (
    item.title.toLocaleLowerCase().includes(q) ||
    item.lastText.toLocaleLowerCase().includes(q)
  );
}

function formatWhen(at: number, now: number, locale: string): string {
  const midnight = new Date(now);
  midnight.setHours(0, 0, 0, 0);
  const today = midnight.getTime();
  const date = new Date(at);
  const options: Intl.DateTimeFormatOptions =
    at >= today
      ? { hour: "2-digit", minute: "2-digit" }
      : at >= today - 6 * DAY_MS
        ? { weekday: "short" }
        : { day: "2-digit", month: "2-digit", year: "2-digit" };
  return date.toLocaleString(locale, options);
}

interface MenuState {
  id: string;
  top: number;
  left: number;
}

export const ConversationList: React.FC<ConversationListProps> = ({
  items,
  activeId,
  onSelect,
  onNew,
  controlsId,
  onCollapse,
  onSearch,
  onRename,
  onPin,
  onArchive,
  onDelete,
  showArchived,
  onShowArchivedChange,
  labels,
  locale,
  now,
}) => {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuButtons = useRef(new Map<string, HTMLButtonElement>());
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const pendingFocus = useRef<string | null>(null);
  const [focusTick, setFocusTick] = useState(0);
  const renameDone = useRef(true);
  const baseId = useId();
  const current = now ?? Date.now();

  const groups = useMemo(
    () => groupConversations(items, current),
    [items, current],
  );
  const groupTitle: Record<ConversationGroupId, string> = {
    pinned: labels.groupPinned,
    today: labels.groupToday,
    week: labels.groupWeek,
    older: labels.groupOlder,
  };
  const menuItem = menu ? items.find((i) => i.id === menu.id) : undefined;

  const closeMenu = (restoreFocus: boolean) => {
    const id = menu?.id;
    setMenu(null);
    if (restoreFocus && id) menuButtons.current.get(id)?.focus();
  };

  /**
   * Fokus zurück auf den Zeilenknopf, sobald er (neu) im DOM steht. Ein Eintrag,
   * der nach dem Anheften in eine andere Gruppe wandert, wird neu eingehängt und
   * verliert den Fokus erneut: darum bleibt der Wunsch bestehen, bis der Nutzer
   * selbst etwas tut oder kurze Zeit vergangen ist.
   */
  const focusRow = (id: string) => {
    pendingFocus.current = id;
    setFocusTick((t) => t + 1);
    const clear = () => {
      if (pendingFocus.current === id) pendingFocus.current = null;
      document.removeEventListener("mousedown", clear, true);
      document.removeEventListener("keydown", clear, true);
      window.clearTimeout(timer);
    };
    const timer = window.setTimeout(clear, 1500);
    document.addEventListener("mousedown", clear, true);
    document.addEventListener("keydown", clear, true);
  };

  useEffect(() => {
    const id = pendingFocus.current;
    if (!id || renamingId || confirmId || menu) return;
    const button = rowButtons.current.get(id);
    if (!button || document.activeElement === button) return;
    button.focus();
  }, [items, focusTick, renamingId, confirmId, menu]);

  useEffect(() => {
    if (!menu) return;
    menuRef.current
      ?.querySelector<HTMLButtonElement>("button:not([aria-disabled='true'])")
      ?.focus();
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (menuRef.current?.contains(target)) return;
      setMenu(null);
    };
    const onScroll = () => setMenu(null);
    document.addEventListener("mousedown", onDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
    // Nur beim Öffnen: der Fokus gehört dem Menü, solange es offen ist.
  }, [menu?.id]);

  const openMenu = (id: string, button: HTMLButtonElement) => {
    if (menu?.id === id) {
      setMenu(null);
      return;
    }
    const r = button.getBoundingClientRect();
    const below = r.bottom + 4;
    const top =
      below + MENU_HEIGHT > window.innerHeight
        ? Math.max(8, r.top - MENU_HEIGHT - 4)
        : below;
    setMenu({
      id,
      top,
      left: Math.max(
        8,
        Math.min(r.right - MENU_WIDTH, window.innerWidth - 8 - MENU_WIDTH),
      ),
    });
  };

  const onMenuKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu(true);
      return;
    }
    if (e.key === "Tab") {
      setMenu(null);
      return;
    }
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>(
        "button[role='menuitem']",
      ) ?? [],
    );
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const move = (to: number) => {
      e.preventDefault();
      buttons[(to + buttons.length) % buttons.length]?.focus();
    };
    if (e.key === "ArrowDown") move(at + 1);
    else if (e.key === "ArrowUp") move(at - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(buttons.length - 1);
  };

  const startRename = (item: ConversationListItem) => {
    renameDone.current = false;
    setDraft(item.title);
    setRenamingId(item.id);
    setMenu(null);
  };

  const finishRename = (item: ConversationListItem, commit: boolean) => {
    if (renameDone.current) return;
    renameDone.current = true;
    setRenamingId(null);
    const title = draft.trim();
    if (commit && title && title !== item.title) onRename(item.id, title);
    focusRow(item.id);
  };

  const menuAction = (id: string, action: () => void) => {
    setMenu(null);
    action();
    focusRow(id);
  };

  const iconButton =
    "grid place-items-center w-[44px] h-[44px] md:w-8 md:h-8 rounded-md cursor-pointer text-text/85 hover:bg-mid-gray/15 hover:text-text";
  const rowButton =
    "min-h-[44px] md:min-h-9 rounded-md cursor-pointer text-start transition-colors";

  return (
    <nav
      aria-label={labels.navLabel}
      className="flex flex-col h-full min-h-0 bg-background"
    >
      <div className="flex flex-col gap-1 p-2 border-b border-mid-gray/20">
        <div className="flex items-center justify-between">
          <button
            type="button"
            aria-controls={controlsId}
            aria-expanded="true"
            aria-label={labels.collapse}
            title={labels.collapse}
            onClick={onCollapse}
            className={iconButton}
          >
            <PanelLeftClose className="w-4 h-4" aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label={labels.search}
            title={`${labels.search} (${labels.searchKey})`}
            data-testid="history-search-open"
            onClick={onSearch}
            className={iconButton}
          >
            <Search className="w-4 h-4" aria-hidden="true" />
          </button>
        </div>
        <button
          type="button"
          onClick={onNew}
          className="flex items-center gap-2 min-h-[44px] md:min-h-9 px-2 rounded-md cursor-pointer text-sm text-text hover:bg-mid-gray/10"
        >
          <SquarePen className="w-4 h-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{labels.newConversation}</span>
        </button>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-2 py-2">
        {groups.length === 0 && (
          <p className="m-0 px-2 py-4 text-sm text-text-muted">
            {labels.empty}
          </p>
        )}
        {groups.map((group) => {
          const headingId = `${baseId}-${group.id}`;
          return (
            <div
              key={group.id}
              role="group"
              aria-labelledby={headingId}
              className="mb-3"
            >
              <div
                id={headingId}
                className="px-2 pb-1 text-xs font-medium text-text-muted"
              >
                {groupTitle[group.id]}
              </div>
              <ul className="m-0 p-0 list-none flex flex-col gap-0.5">
                {group.items.map((item) => {
                  const active = item.id === activeId;
                  const rowTone = active
                    ? "bg-logo-primary/15"
                    : "hover:bg-mid-gray/10";
                  if (confirmId === item.id) {
                    return (
                      <li
                        key={item.id}
                        role="group"
                        aria-label={labels.confirmDelete(item.title)}
                        onKeyDown={(e) => {
                          if (e.key !== "Escape") return;
                          e.preventDefault();
                          e.stopPropagation();
                          setConfirmId(null);
                          focusRow(item.id);
                        }}
                        className="rounded-md border border-red-500/40 bg-red-500/5 p-2 text-sm"
                      >
                        <p className="m-0 mb-2 text-text">
                          {labels.confirmDelete(item.title)}
                        </p>
                        <div className="flex gap-2">
                          <button
                            type="button"
                            onClick={() => {
                              setConfirmId(null);
                              onDelete(item.id);
                            }}
                            className="min-h-[44px] md:min-h-8 px-3 rounded-md bg-red-600 text-white cursor-pointer hover:bg-red-700"
                          >
                            {labels.confirmYes}
                          </button>
                          <button
                            type="button"
                            autoFocus
                            onClick={() => {
                              setConfirmId(null);
                              focusRow(item.id);
                            }}
                            className="min-h-[44px] md:min-h-8 px-3 rounded-md border border-mid-gray/30 text-text cursor-pointer hover:bg-mid-gray/10"
                          >
                            {labels.confirmNo}
                          </button>
                        </div>
                      </li>
                    );
                  }
                  return (
                    <li
                      key={item.id}
                      className={`group relative flex items-center rounded-md ${rowTone}`}
                    >
                      {renamingId === item.id ? (
                        <input
                          autoFocus
                          value={draft}
                          maxLength={MAX_TITLE}
                          aria-label={labels.renameLabel}
                          onFocus={(e) => e.currentTarget.select()}
                          onChange={(e) => setDraft(e.target.value)}
                          onBlur={() => finishRename(item, true)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              finishRename(item, true);
                            } else if (e.key === "Escape") {
                              e.preventDefault();
                              e.stopPropagation();
                              finishRename(item, false);
                            }
                          }}
                          className="flex-1 min-w-0 min-h-[44px] md:min-h-9 mx-1 px-2 rounded-md border border-logo-primary bg-background text-sm text-text focus:outline-none"
                        />
                      ) : (
                        <button
                          type="button"
                          ref={(el) => {
                            if (el) rowButtons.current.set(item.id, el);
                            else rowButtons.current.delete(item.id);
                          }}
                          aria-current={active ? "true" : undefined}
                          onClick={() => onSelect(item.id)}
                          className={`${rowButton} flex-1 min-w-0 flex items-center gap-2 ps-2 pe-1 text-sm text-text`}
                        >
                          {item.pinned && (
                            <Pin
                              className="w-3 h-3 shrink-0 text-text-muted"
                              aria-hidden="true"
                            />
                          )}
                          <span className="flex-1 min-w-0 truncate">
                            {item.title}
                          </span>
                          {item.main && (
                            <Phone
                              className="w-3 h-3 shrink-0 text-logo-primary/80"
                              aria-label={labels.mainBadge}
                              role="img"
                            >
                              <title>{labels.mainBadge}</title>
                            </Phone>
                          )}
                          {item.archived && (
                            <span className="shrink-0 rounded px-1 text-[10px] bg-mid-gray/20 text-text/85">
                              {labels.archivedBadge}
                            </span>
                          )}
                          <time
                            dateTime={new Date(item.updatedAt).toISOString()}
                            className="shrink-0 text-xs text-text-muted"
                          >
                            {formatWhen(item.updatedAt, current, locale)}
                          </time>
                        </button>
                      )}
                      {renamingId !== item.id && (
                        <button
                          type="button"
                          ref={(el) => {
                            if (el) menuButtons.current.set(item.id, el);
                            else menuButtons.current.delete(item.id);
                          }}
                          aria-haspopup="menu"
                          aria-expanded={menu?.id === item.id}
                          aria-controls={
                            menu?.id === item.id ? `${baseId}-menu` : undefined
                          }
                          aria-label={labels.menu(item.title)}
                          title={labels.menu(item.title)}
                          onClick={(e) => openMenu(item.id, e.currentTarget)}
                          className={`grid place-items-center w-[44px] h-[44px] md:w-8 md:h-8 shrink-0 rounded-md cursor-pointer text-text/85 hover:bg-mid-gray/20 focus-visible:opacity-100 md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100 ${
                            menu?.id === item.id ? "md:opacity-100" : ""
                          }`}
                        >
                          <MoreHorizontal
                            className="w-4 h-4"
                            aria-hidden="true"
                          />
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })}
      </div>

      <label className="flex items-center gap-2 min-h-[44px] md:min-h-9 px-3 border-t border-mid-gray/20 text-xs text-text/80 cursor-pointer">
        <input
          type="checkbox"
          role="switch"
          checked={showArchived}
          onChange={(e) => onShowArchivedChange(e.target.checked)}
          className="cursor-pointer"
        />
        <span>{labels.showArchived}</span>
      </label>

      {menu && menuItem && (
        <div
          ref={menuRef}
          id={`${baseId}-menu`}
          role="menu"
          aria-label={labels.menu(menuItem.title)}
          onKeyDown={onMenuKey}
          style={{ position: "fixed", top: menu.top, left: menu.left }}
          className="z-50 rounded-lg border border-mid-gray/30 bg-background p-1 shadow-xl"
        >
          <div style={{ width: MENU_WIDTH - 8 }} className="flex flex-col">
            <MenuButton
              icon={<Pencil className="w-4 h-4" aria-hidden="true" />}
              label={labels.rename}
              onSelect={() => startRename(menuItem)}
            />
            <MenuButton
              icon={
                menuItem.pinned ? (
                  <PinOff className="w-4 h-4" aria-hidden="true" />
                ) : (
                  <Pin className="w-4 h-4" aria-hidden="true" />
                )
              }
              label={menuItem.pinned ? labels.unpin : labels.pin}
              onSelect={() =>
                menuAction(menuItem.id, () =>
                  onPin(menuItem.id, !menuItem.pinned),
                )
              }
            />
            <MenuButton
              icon={
                menuItem.archived ? (
                  <ArchiveRestore className="w-4 h-4" aria-hidden="true" />
                ) : (
                  <Archive className="w-4 h-4" aria-hidden="true" />
                )
              }
              label={menuItem.archived ? labels.unarchive : labels.archive}
              onSelect={() =>
                menuAction(menuItem.id, () =>
                  onArchive(menuItem.id, !menuItem.archived),
                )
              }
            />
            <MenuButton
              icon={<Trash2 className="w-4 h-4" aria-hidden="true" />}
              label={labels.delete}
              danger
              onSelect={() =>
                menuAction(menuItem.id, () => {
                  setConfirmId(menuItem.id);
                })
              }
            />
            <div
              role="separator"
              className="my-1 border-t border-mid-gray/20"
            />
            <MenuButton
              icon={<Cloud className="w-4 h-4" aria-hidden="true" />}
              label={labels.openInCorporate}
              hint={labels.corporateHint}
              disabled
            />
          </div>
        </div>
      )}
    </nav>
  );
};

const MenuButton: React.FC<{
  icon: React.ReactNode;
  label: string;
  hint?: string;
  disabled?: boolean;
  danger?: boolean;
  onSelect?: () => void;
}> = ({ icon, label, hint, disabled, danger, onSelect }) => (
  <button
    type="button"
    role="menuitem"
    aria-disabled={disabled ? "true" : undefined}
    onClick={disabled ? undefined : onSelect}
    className={`flex items-start gap-2 min-h-[44px] md:min-h-9 px-2 py-1.5 rounded-md text-start text-sm ${
      disabled
        ? "cursor-not-allowed opacity-60"
        : `cursor-pointer hover:bg-mid-gray/15 ${danger ? "text-red-500" : "text-text"}`
    }`}
  >
    <span className="mt-0.5 shrink-0">{icon}</span>
    <span className="flex flex-col">
      <span>{label}</span>
      {hint && <small className="text-text-muted">{hint}</small>}
    </span>
  </button>
);
