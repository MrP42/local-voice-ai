import React, {
  useCallback,
  useEffect,
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
  Phone,
  PhoneOff,
  Settings,
  SquarePen,
} from "lucide-react";
import { commands } from "@/bindings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import {
  Composer,
  type ComposerAttachment,
  type ComposerMenuItem,
} from "./Composer";
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
}

export const VoiceCallPage: React.FC = () => {
  const { t } = useTranslation();
  const state = useVoiceCallStore((s) => s.state);
  const lines = useVoiceCallStore((s) => s.lines);
  const ready = useVoiceCallStore((s) => s.ready);
  const error = useVoiceCallStore((s) => s.error);
  const muted = useVoiceCallStore((s) => s.muted);
  const textBusy = useVoiceCallStore((s) => s.textBusy);
  const settingsOpen = useVoiceCallStore((s) => s.settingsOpen);
  const store = useVoiceCallStore;
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const attachId = useRef(0);

  useEffect(() => {
    store.getState().setActive(true);
    return () => void store.getState().leave();
  }, [store]);

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
      void store.getState().sendText(body, display);
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
      try {
        const info = await stat(picked);
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
  ];

  return (
    <PageShell
      title={t("voiceCall.title")}
      fill
      actions={
        <>
          {lines.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              type="button"
              icon={<SquarePen className="w-4 h-4" aria-hidden="true" />}
              aria-label={t("voiceCall.newConversation")}
              title={t("voiceCall.newConversation")}
              onClick={() => store.getState().newConversation()}
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
