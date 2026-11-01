import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { stat } from "@tauri-apps/plugin-fs";
import {
  AlertCircle,
  AlertTriangle,
  AudioLines,
  Cloud,
  Paperclip,
  PanelLeftOpen,
  SquarePen,
} from "lucide-react";
import { commands } from "@/bindings";
import { lineEditable, useVoiceCallStore } from "@/stores/voiceCallStore";
import { mimeForName } from "@/stores/voiceHistory";
import { PageShell } from "../ui/PageShell";
import {
  Composer,
  type ComposerAttachment,
  type ComposerMenuItem,
} from "./Composer";
import { ConversationList } from "./ConversationList";
import { ConversationOptions } from "./ConversationOptions";
import { ConversationSearch, type SearchAction } from "./ConversationSearch";
import { InfoLine } from "./InfoLine";
import { LevelWave } from "./LevelWave";
import { ModelPicker } from "./ModelPicker";
import { PermissionChip } from "./PermissionChip";
import { SettingsPanel } from "./SettingsPanel";
import { Transcript } from "./Transcript";

/** Mehr Text als das gehört nicht in eine Nachricht an die KI. */
const MAX_FILE_CHARS = 20_000;
/** … und alle Anhänge einer Nachricht zusammen. */
const MAX_TOTAL_CHARS = 40_000;
/** Größere Dateien werden gar nicht erst gelesen. */
const MAX_FILE_BYTES = 20 * 1024 * 1024;

interface Attachment extends ComposerAttachment {
  text: string;
  /** Größe in Bytes, falls `stat` sie geliefert hat. */
  size?: number;
}

/** Schlüssel der gespeicherten Wahl: Verlaufsspalte offen oder eingeklappt. */
const HISTORY_OPEN_KEY = "voice-call.history-open";
/** Unter dieser Breite liegt die Verlaufsliste als Überlagerung über dem Gespräch. */
const NARROW_QUERY = "(max-width: 819.98px)";

function readHistoryOpen(): boolean {
  try {
    return window.localStorage.getItem(HISTORY_OPEN_KEY) !== "0";
  } catch {
    return true;
  }
}

function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(
    () => window.matchMedia(NARROW_QUERY).matches,
  );
  useEffect(() => {
    const query = window.matchMedia(NARROW_QUERY);
    const onChange = () => setNarrow(query.matches);
    onChange();
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return narrow;
}

export const VoiceCallPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const state = useVoiceCallStore((s) => s.state);
  const lines = useVoiceCallStore((s) => s.lines);
  const ready = useVoiceCallStore((s) => s.ready);
  const error = useVoiceCallStore((s) => s.error);
  const saveError = useVoiceCallStore((s) => s.saveError);
  const switching = useVoiceCallStore((s) => s.switching);
  const muted = useVoiceCallStore((s) => s.muted);
  const textBusy = useVoiceCallStore((s) => s.textBusy);
  const toolStatus = useVoiceCallStore((s) => s.toolStatus);
  const notice = useVoiceCallStore((s) => s.notice);
  const settingsOpen = useVoiceCallStore((s) => s.settingsOpen);
  const conversations = useVoiceCallStore((s) => s.conversations);
  const conversationId = useVoiceCallStore((s) => s.conversationId);
  const mainConversationId = useVoiceCallStore((s) => s.mainConversationId);
  const showArchived = useVoiceCallStore((s) => s.showArchived);
  const store = useVoiceCallStore;
  const narrow = useNarrow();
  const [wideOpen, setWideOpen] = useState(readHistoryOpen);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const historyOpen = narrow ? overlayOpen : wideOpen;
  const historyId = useId();
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const attachId = useRef(0);

  useEffect(() => {
    store.getState().setActive(true);
    void store.getState().loadConversations();
    return () => void store.getState().leave();
  }, [store]);

  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  const keyNew = isMac ? "⌘N" : t("voiceCall.history.keyNew");
  const keySearch = isMac ? "⌘⇧K" : t("voiceCall.history.keySearch");

  // Strg+N: neues Gespräch, Strg+Umschalt+K: Suche. Strg+K bleibt der App-Suche der Titelleiste.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(isMac ? e.metaKey : e.ctrlKey) || e.altKey) return;
      const key = e.key.toLowerCase();
      if (key === "n" && !e.shiftKey) {
        // Nicht hinter einem offenen Dialog (Suche, Einstellungen) ein Gespräch beenden.
        if (document.querySelector('[role="dialog"], [role="alertdialog"]'))
          return;
        e.preventDefault();
        void store.getState().newConversation();
      } else if (key === "k" && e.shiftKey) {
        // Vor dem Strg+K-Handler der Titelleiste, der auch mit Umschalt anspringen würde.
        e.preventDefault();
        e.stopImmediatePropagation();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [store, isMac]);

  const toggleHistory = () => {
    if (narrow) {
      setOverlayOpen((open) => !open);
      return;
    }
    const next = !wideOpen;
    setWideOpen(next);
    try {
      window.localStorage.setItem(HISTORY_OPEN_KEY, next ? "1" : "0");
    } catch {
      /* nicht merken ist verkraftbar */
    }
  };
  const afterPick = () => {
    if (narrow) setOverlayOpen(false);
  };
  const historyItems = useMemo(
    () =>
      conversations.map((c) => ({
        id: c.id,
        title: c.title,
        updatedAt: Date.parse(c.updated_at) || 0,
        pinned: c.pinned,
        archived: c.archived,
        main: c.id === mainConversationId,
        lastText: c.last_text,
      })),
    [conversations, mainConversationId],
  );
  const historyLabels = useMemo(
    () => ({
      navLabel: t("voiceCall.history.navLabel"),
      newConversation: t("voiceCall.history.newConversation"),
      search: t("voiceCall.history.search"),
      searchKey: keySearch,
      collapse: t("voiceCall.history.toggle"),
      groupPinned: t("voiceCall.history.groupPinned"),
      groupToday: t("voiceCall.history.groupToday"),
      groupWeek: t("voiceCall.history.groupWeek"),
      groupOlder: t("voiceCall.history.groupOlder"),
      empty: t("voiceCall.history.empty"),
      menu: (title: string) => t("voiceCall.history.menu", { title }),
      rename: t("voiceCall.history.rename"),
      renameLabel: t("voiceCall.history.renameLabel"),
      pin: t("voiceCall.history.pin"),
      unpin: t("voiceCall.history.unpin"),
      archive: t("voiceCall.history.archive"),
      unarchive: t("voiceCall.history.unarchive"),
      delete: t("voiceCall.history.delete"),
      openInCorporate: t("voiceCall.history.openInCorporate"),
      corporateHint: t("voiceCall.history.corporateHint"),
      confirmDelete: (title: string) =>
        t("voiceCall.history.confirmDelete", { title }),
      confirmYes: t("voiceCall.history.confirmYes"),
      confirmNo: t("voiceCall.history.confirmNo"),
      showArchived: t("voiceCall.history.showArchived"),
      archivedBadge: t("voiceCall.history.archivedBadge"),
      mainBadge: t("voiceCall.history.mainBadge"),
    }),
    [t, keySearch],
  );
  const historyList = (
    <ConversationList
      items={historyItems}
      activeId={conversationId}
      onSelect={(id) => {
        afterPick();
        void store.getState().openConversation(id);
      }}
      onNew={() => {
        afterPick();
        void store.getState().newConversation();
      }}
      controlsId={historyId}
      onCollapse={toggleHistory}
      onSearch={() => setSearchOpen(true)}
      onRename={(id, title) => void store.getState().rename(id, title)}
      onPin={(id) => void store.getState().togglePin(id)}
      onArchive={(id, archived) => void store.getState().archive(id, archived)}
      onDelete={(id) => void store.getState().remove(id)}
      showArchived={showArchived}
      onShowArchivedChange={(show) =>
        void store.getState().setShowArchived(show)
      }
      labels={historyLabels}
      locale={i18n.language}
    />
  );

  const inCall = state !== "idle";
  const busy = state === "thinking" || state === "speaking";
  const toggleCall = () =>
    void (inCall ? store.getState().stopCall() : store.getState().startCall());
  const callLabel = inCall ? t("voiceCall.end") : t("voiceCall.begin");
  const searchActions: SearchAction[] = [
    {
      id: "new",
      label: t("voiceCall.newConversation"),
      icon: <SquarePen className="w-4 h-4" aria-hidden="true" />,
      shortcut: keyNew,
      onSelect: () => {
        afterPick();
        void store.getState().newConversation();
      },
    },
    {
      id: "call",
      label: callLabel,
      icon: <AudioLines className="w-4 h-4" aria-hidden="true" />,
      onSelect: toggleCall,
    },
  ];
  const searchLabels = {
    title: t("voiceCall.history.search"),
    placeholder: t("voiceCall.history.searchPlaceholder"),
    sectionActions: t("voiceCall.history.sectionActions"),
    sectionRecent: t("voiceCall.history.sectionRecent"),
    sectionResults: t("voiceCall.history.sectionResults"),
    tabAll: t("voiceCall.history.tabAll"),
    tabActions: t("voiceCall.history.tabActions"),
    tabConversations: t("voiceCall.history.tabConversations"),
    tabsLabel: t("voiceCall.history.tabsLabel"),
    noResults: t("voiceCall.history.noResults"),
    close: t("voiceCall.history.close"),
  };

  const send = useCallback(
    (typed: string, files: Attachment[]) => {
      const text = typed.trim();
      if (!text && files.length === 0) return;
      const body = [text, ...files.map((f) => `[Datei: ${f.label}]\n${f.text}`)]
        .filter(Boolean)
        .join("\n\n");
      const display = files.length
        ? [
            text,
            t("voiceCall.composer.attached", {
              names: files.map((f) => f.label).join(", "),
            }),
          ]
            .filter(Boolean)
            .join("\n")
        : undefined;
      void store.getState().sendText(
        body,
        display,
        files.map((f) => ({
          id: f.id,
          name: f.label,
          mime: mimeForName(f.label),
          ...(f.size !== undefined ? { size_bytes: f.size } : {}),
        })),
      );
    },
    [store, t],
  );

  const onSend = () => {
    send(draft, attachments);
    setDraft("");
    setAttachments([]);
  };

  const attachFile = useCallback(async () => {
    setAttachError(null);
    try {
      const picked = await open({
        multiple: false,
        filters: [
          {
            name: t("voiceCall.composer.documents"),
            extensions: ["txt", "md", "pdf", "docx"],
          },
        ],
      });
      if (typeof picked !== "string") return;
      let size: number | undefined;
      try {
        const info = await stat(picked);
        size = info.size;
        if (info.size > MAX_FILE_BYTES) {
          setAttachError(t("voiceCall.composer.tooLarge"));
          return;
        }
      } catch {
        /* ohne Größe (keine Berechtigung) geht es weiter; die Textgrenze greift trotzdem */
      }
      const result = await commands.ttsExtractDocument(picked);
      if (result.status === "error") {
        setAttachError(result.error);
        return;
      }
      const name = picked.split(/[\\/]/).pop() ?? picked;
      const id = `att-${attachId.current++}`;
      setAttachments((all) => {
        const used = all.reduce((n, a) => n + a.text.length, 0);
        const room = Math.min(MAX_FILE_CHARS, MAX_TOTAL_CHARS - used);
        if (room <= 0) {
          setAttachError(t("voiceCall.composer.totalLimit"));
          return all;
        }
        const cut = result.data.length > room;
        return [
          ...all,
          {
            id,
            label: name,
            detail: cut ? t("voiceCall.composer.truncated") : undefined,
            text: cut ? result.data.slice(0, room) : result.data,
            size,
          },
        ];
      });
    } catch (e) {
      setAttachError(String(e));
    }
  }, [t]);

  const menuItems: ComposerMenuItem[] = useMemo(
    () => [
      {
        id: "files",
        label: t("voiceCall.composer.attachFiles"),
        icon: <Paperclip className="w-4 h-4" aria-hidden="true" />,
        onSelect: () => void attachFile(),
      },
      {
        id: "corporate",
        label: t("voiceCall.composer.fromCorporate"),
        icon: <Cloud className="w-4 h-4" aria-hidden="true" />,
        hint: t("voiceCall.composer.corporateHint"),
        disabled: true,
        separatorBefore: true,
      },
    ],
    [t, attachFile],
  );

  const warnings = ready?.warnings ?? [];
  const alerts = [
    ...warnings.map((w) => ({ kind: "warn" as const, text: w })),
    ...(attachError ? [{ kind: "error" as const, text: attachError }] : []),
    ...(error ? [{ kind: "error" as const, text: error }] : []),
    ...(saveError
      ? [
          {
            kind: "error" as const,
            text: `${t("voiceCall.history.saveFailed")} ${saveError}`,
          },
        ]
      : []),
  ];

  return (
    <PageShell title={t("voiceCall.title")} fill>
      <div className="relative flex h-full min-h-0 overflow-hidden">
        {historyOpen && !narrow && (
          <aside
            id={historyId}
            className="w-[260px] shrink-0 min-h-0 border-e border-mid-gray/20"
          >
            {historyList}
          </aside>
        )}
        {historyOpen && narrow && (
          <>
            <button
              type="button"
              tabIndex={-1}
              aria-label={t("voiceCall.history.close")}
              className="absolute inset-0 z-20 bg-black/40 cursor-default"
              onClick={() => setOverlayOpen(false)}
            />
            <aside
              id={historyId}
              onKeyDown={(e) => {
                if (e.key === "Escape") setOverlayOpen(false);
              }}
              className="absolute inset-y-0 start-0 z-30 w-[min(300px,85%)] border-e border-mid-gray/20 shadow-2xl"
            >
              {historyList}
            </aside>
          </>
        )}
        <div className="flex flex-col flex-1 min-w-0 min-h-0">
          <InfoLine
            leading={
              !historyOpen ? (
                <button
                  type="button"
                  aria-controls={historyId}
                  aria-expanded="false"
                  aria-label={t("voiceCall.history.toggle")}
                  title={t("voiceCall.history.toggle")}
                  onClick={toggleHistory}
                  className="grid place-items-center w-8 h-8 shrink-0 rounded cursor-pointer text-text/70 hover:bg-mid-gray/15 hover:text-text"
                >
                  <PanelLeftOpen className="w-4 h-4" aria-hidden="true" />
                </button>
              ) : null
            }
            trailing={<ConversationOptions />}
          />
          {alerts.map((a) => (
            <div
              key={`${a.kind}-${a.text}`}
              role={a.kind === "error" ? "alert" : "note"}
              className={`flex items-center gap-2 px-4 py-1 text-xs border-b border-mid-gray/20 ${
                a.kind === "error"
                  ? "bg-red-500/10 text-red-500"
                  : "bg-yellow-500/10 text-yellow-600"
              }`}
            >
              {a.kind === "error" ? (
                <AlertCircle
                  className="w-3.5 h-3.5 shrink-0"
                  aria-hidden="true"
                />
              ) : (
                <AlertTriangle
                  className="w-3.5 h-3.5 shrink-0"
                  aria-hidden="true"
                />
              )}
              <span className="line-clamp-2" title={a.text}>
                {a.text}
              </span>
            </div>
          ))}
          <Transcript
            lines={lines}
            active={inCall}
            onSuggest={(text) => send(text, [])}
            toolStatus={toolStatus}
            notice={notice}
            onOpenSettings={() => store.getState().openSettings("models")}
            onEditLine={(id, text) => void store.getState().editLine(id, text)}
            canEdit={(line) => lineEditable(line, state, textBusy)}
          />
          <Composer
            value={draft}
            onChange={setDraft}
            onSend={onSend}
            attachments={attachments}
            onRemoveAttachment={(id) =>
              setAttachments((all) => all.filter((a) => a.id !== id))
            }
            menuItems={menuItems}
            leading={<PermissionChip />}
            tools={<ModelPicker />}
            callAction={{
              active: inCall,
              label: callLabel,
              onClick: toggleCall,
            }}
            muted={muted}
            onToggleMute={() => store.getState().setMuted(!muted)}
            sending={textBusy}
            disabled={switching}
            onStop={() => store.getState().stopText()}
            labels={{
              message: t("voiceCall.composer.message"),
              placeholder: t("voiceCall.composer.placeholder"),
              add: t("voiceCall.composer.add"),
              mute: t("voiceCall.composer.mute"),
              send: t("voiceCall.composer.send"),
              stop: t("voiceCall.composer.stop"),
              removeAttachment: (label) =>
                t("voiceCall.composer.removeAttachment", { name: label }),
            }}
          >
            <span
              className={`items-center gap-2 px-2 text-xs text-text/70 ${
                inCall || textBusy ? "inline-flex" : "inline-flex max-sm:hidden"
              }`}
            >
              <LevelWave live={inCall && !muted} busy={busy || textBusy} />
              <span className="max-sm:hidden">
                {textBusy
                  ? t("voiceCall.state.thinking")
                  : t(`voiceCall.state.${state}`)}
              </span>
            </span>
          </Composer>
        </div>
        {settingsOpen && <SettingsPanel />}
      </div>
      <ConversationSearch
        open={searchOpen}
        onOpenChange={setSearchOpen}
        items={historyItems}
        actions={searchActions}
        onSelect={(id) => {
          afterPick();
          void store.getState().openConversation(id);
        }}
        labels={searchLabels}
      />
    </PageShell>
  );
};
