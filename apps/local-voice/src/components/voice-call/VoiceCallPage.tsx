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
  Cloud,
  Paperclip,
  PanelLeft,
  Phone,
  PhoneOff,
  Settings,
  SquarePen,
} from "lucide-react";
import { commands } from "@/bindings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { mimeForName } from "@/stores/voiceHistory";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import {
  Composer,
  type ComposerAttachment,
  type ComposerMenuItem,
} from "./Composer";
import { ConversationList } from "./ConversationList";
import { InfoLine } from "./InfoLine";
import { LevelWave } from "./LevelWave";
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
  const showArchived = useVoiceCallStore((s) => s.showArchived);
  const store = useVoiceCallStore;
  const narrow = useNarrow();
  const [wideOpen, setWideOpen] = useState(readHistoryOpen);
  const [overlayOpen, setOverlayOpen] = useState(false);
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
        lastText: c.last_text,
      })),
    [conversations],
  );
  const historyLabels = useMemo(
    () => ({
      navLabel: t("voiceCall.history.navLabel"),
      newConversation: t("voiceCall.history.newConversation"),
      search: t("voiceCall.history.search"),
      searchPlaceholder: t("voiceCall.history.searchPlaceholder"),
      groupPinned: t("voiceCall.history.groupPinned"),
      groupToday: t("voiceCall.history.groupToday"),
      groupWeek: t("voiceCall.history.groupWeek"),
      groupOlder: t("voiceCall.history.groupOlder"),
      empty: t("voiceCall.history.empty"),
      noResults: t("voiceCall.history.noResults"),
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
    }),
    [t],
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
    <PageShell
      title={t("voiceCall.title")}
      fill
      actions={
        <>
          <Button
            variant="ghost"
            size="sm"
            type="button"
            aria-pressed={historyOpen}
            aria-controls={historyId}
            aria-label={t("voiceCall.history.toggle")}
            title={t("voiceCall.history.toggle")}
            icon={<PanelLeft className="w-4 h-4" aria-hidden="true" />}
            onClick={toggleHistory}
          >
            <span className="max-md:hidden">
              {t("voiceCall.history.toggle")}
            </span>
          </Button>
          {lines.length > 0 && !historyOpen && (
            <Button
              variant="ghost"
              size="sm"
              type="button"
              icon={<SquarePen className="w-4 h-4" aria-hidden="true" />}
              aria-label={t("voiceCall.newConversation")}
              title={t("voiceCall.newConversation")}
              onClick={() => void store.getState().newConversation()}
            >
              <span className="max-md:hidden">
                {t("voiceCall.newConversation")}
              </span>
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            type="button"
            aria-pressed={settingsOpen}
            aria-label={t("voiceCall.settings.title")}
            title={t("voiceCall.settings.title")}
            icon={<Settings className="w-4 h-4" aria-hidden="true" />}
            onClick={() =>
              settingsOpen
                ? store.getState().closeSettings()
                : store.getState().openSettings()
            }
          />
          {inCall ? (
            <Button
              variant="danger"
              size="sm"
              type="button"
              onClick={() => void store.getState().stopCall()}
            >
              <PhoneOff className="w-4 h-4 sm:mr-1" aria-hidden="true" />
              <span className="max-sm:sr-only">{t("voiceCall.end")}</span>
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              type="button"
              onClick={() => void store.getState().startCall()}
            >
              <Phone className="w-4 h-4 sm:mr-1" aria-hidden="true" />
              <span className="max-sm:sr-only">{t("voiceCall.begin")}</span>
            </Button>
          )}
        </>
      }
    >
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
          <InfoLine />
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
            <span className="inline-flex items-center gap-2 px-2 text-xs text-text/70">
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
    </PageShell>
  );
};
