import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type FollowupMode,
  type MailDraft,
  type PostProcessProvider,
} from "@/bindings";
import { chatErrorCode, chatErrorKey } from "@/lib/meetingChat";
import {
  emlFileName,
  FOLLOWUP_ERROR_CODES,
  parseAddresses,
} from "@/lib/meetingFollowup";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";
import { Textarea } from "../../ui/Textarea";
import { isLocalProvider } from "./MeetingChatNotice";

type Phase = "idle" | "loading" | "ready" | "error";
type Status = { kind: "ok" | "info" | "error"; text: string };

/** Fehler von `meeting_followup_*` auf einen Text (Chat-Codes inklusive). */
const errorText = (
  t: (key: string, options?: Record<string, unknown>) => string,
  error: unknown,
): string => {
  const code = chatErrorCode(error);
  if ((FOLLOWUP_ERROR_CODES as readonly string[]).includes(code)) {
    return t(`meetings.followup.errors.${code}`);
  }
  const key = chatErrorKey(code);
  return key === "meetings.chat.errors.unknown"
    ? t("meetings.followup.errors.generic", { error: String(error) })
    : t(key, { code });
};

interface FollowupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
}

/**
 * M6-P6c: Follow-up-Mail. Erzeugt über das Recipe „Follow-up-E-Mail“ (Chat,
 * Scope der Besprechung) einen Entwurf; An, Betreff und Text sind
 * bearbeitbar. Ausgänge: Kopieren (HTML + Text), Mailprogramm (`mailto:`),
 * .eml-Datei. Externer Anbieter: gelbe Leiste, der Entwurf entsteht erst
 * nach einem Klick auf „Entwurf erzeugen“.
 */
export const FollowupDialog: React.FC<FollowupDialogProps> = ({
  open,
  onOpenChange,
  meetingId,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const providerId = getSetting("post_process_provider_id");
  const providers = (getSetting("post_process_providers") ??
    []) as PostProcessProvider[];
  const active = providers.find((p) => p.id === providerId);
  const remote = active && !isLocalProvider(active) ? active : null;
  const remoteId = remote?.id ?? null;

  const [phase, setPhase] = useState<Phase>("idle");
  const [to, setTo] = useState("");
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [copyHighlight, setCopyHighlight] = useState(false);
  const [busy, setBusy] = useState(false);
  // Schließen oder Neustart: eine spät eintreffende Antwort verwerfen.
  const epochRef = useRef(0);

  const generate = useCallback(async () => {
    const epoch = ++epochRef.current;
    setPhase("loading");
    setError(null);
    setStatus(null);
    setCopyHighlight(false);
    let result: Awaited<ReturnType<typeof commands.meetingFollowupDraft>>;
    try {
      result = await commands.meetingFollowupDraft(meetingId);
    } catch (e) {
      result = { status: "error", error: String(e) };
    }
    if (epoch !== epochRef.current) return;
    if (result.status === "error") {
      setError(errorText(t, result.error));
      setPhase("error");
      return;
    }
    setTo((result.data.to ?? []).join(", "));
    setSubject(result.data.subject);
    setBody(result.data.body_text);
    setPhase("ready");
  }, [meetingId, t]);
  const generateRef = useRef(generate);
  generateRef.current = generate;

  useEffect(() => {
    if (!open) {
      epochRef.current++;
      return;
    }
    setPhase("idle");
    setTo("");
    setSubject("");
    setBody("");
    setError(null);
    setStatus(null);
    setCopyHighlight(false);
    if (remoteId === null) void generateRef.current();
  }, [open, remoteId, meetingId]);

  const currentDraft = (): MailDraft => ({
    to: parseAddresses(to),
    subject,
    body_text: body,
    body_html: "",
  });

  const output = async (
    mode: FollowupMode,
    path: string | null,
    onOk: (clipped: boolean) => Status,
  ) => {
    setBusy(true);
    setStatus(null);
    let result: Awaited<ReturnType<typeof commands.meetingFollowupOpen>>;
    try {
      result = await commands.meetingFollowupOpen(currentDraft(), mode, path);
    } catch (e) {
      result = { status: "error", error: String(e) };
    }
    setBusy(false);
    if (result.status === "error") {
      // Mailprogramm nicht erreichbar: der Kopierknopf ist der Ausweg.
      setCopyHighlight(chatErrorCode(result.error) === "mailto_failed");
      setStatus({ kind: "error", text: errorText(t, result.error) });
      return;
    }
    setCopyHighlight(false);
    setStatus(onOk(result.data));
  };

  const onCopy = () =>
    output("copy", null, () => ({
      kind: "ok",
      text: t("meetings.followup.copied"),
    }));

  const onMailto = () =>
    output("mailto", null, (clipped) =>
      clipped
        ? { kind: "info", text: t("meetings.followup.mailtoClipped") }
        : { kind: "ok", text: t("meetings.followup.opened") },
    );

  const onEml = async () => {
    let target: string | null;
    try {
      target = await save({
        defaultPath: emlFileName(subject),
        filters: [{ name: "E-Mail", extensions: ["eml"] }],
      });
    } catch (e) {
      setStatus({ kind: "error", text: String(e) });
      return;
    }
    if (typeof target !== "string") return;
    await output("eml", target, () => ({
      kind: "ok",
      text: t("meetings.followup.emlSaved", { path: target }),
    }));
  };

  const ready = phase === "ready";
  const footer = (
    <div className="flex min-w-0 flex-wrap justify-end gap-2">
      {ready && (
        <Button
          variant="ghost"
          onClick={() => void generate()}
          disabled={busy}
          data-testid="followup-regenerate"
        >
          {t("meetings.followup.regenerate")}
        </Button>
      )}
      {ready && (
        <>
          <Button
            variant={copyHighlight ? "primary" : "secondary"}
            onClick={() => void onCopy()}
            disabled={busy}
            title={t("meetings.followup.copyTitle")}
            data-testid="followup-copy"
            data-highlighted={copyHighlight ? "true" : undefined}
          >
            {t("meetings.followup.copy")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => void onMailto()}
            disabled={busy}
            data-testid="followup-mailto"
          >
            {t("meetings.followup.mailto")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => void onEml()}
            disabled={busy}
            title={t("meetings.followup.emlTitle")}
            data-testid="followup-eml"
          >
            {t("meetings.followup.eml")}
          </Button>
        </>
      )}
      <Button variant="secondary" onClick={() => onOpenChange(false)}>
        {t("meetings.followup.close")}
      </Button>
    </div>
  );

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.followup.title")}
      closeLabel={t("meetings.followup.close")}
      footer={footer}
      className="max-w-3xl"
    >
      <div className="space-y-3" data-testid="followup-dialog">
        {remote && (
          <p
            data-testid="followup-remote-bar"
            className="flex items-start gap-1.5 rounded-md border border-yellow-500/40 bg-yellow-500/10 px-2 py-1.5 text-xs text-yellow-700 dark:text-yellow-300"
          >
            <AlertTriangle
              width={12}
              height={12}
              className="mt-0.5 shrink-0"
              aria-hidden="true"
            />
            <span>
              {t("meetings.followup.remote", { provider: remote.label })}
            </span>
          </p>
        )}

        {phase === "idle" && (
          <Button
            onClick={() => void generate()}
            data-testid="followup-generate"
          >
            {t("meetings.followup.generate")}
          </Button>
        )}

        {phase === "loading" && (
          <p
            role="status"
            className="text-sm text-text/70"
            data-testid="followup-loading"
          >
            {t("meetings.followup.generating")}
          </p>
        )}

        {phase === "error" && (
          <div className="space-y-2">
            <p
              role="alert"
              className="text-sm text-red-400"
              data-testid="followup-error"
            >
              {error}
            </p>
            <Button variant="secondary" onClick={() => void generate()}>
              {t("meetings.followup.retry")}
            </Button>
          </div>
        )}

        {ready && (
          <div className="space-y-3">
            <div className="space-y-1">
              <label
                htmlFor="followup-to"
                className="text-xs font-medium text-text/70"
              >
                {t("meetings.followup.to")}
              </label>
              <Input
                id="followup-to"
                className="w-full"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                placeholder="name@firma.de"
                aria-describedby="followup-to-hint"
              />
              <p id="followup-to-hint" className="text-xs text-text/60">
                {parseAddresses(to).length === 0
                  ? t("meetings.followup.noRecipients")
                  : t("meetings.followup.toHint")}
              </p>
            </div>
            <div className="space-y-1">
              <label
                htmlFor="followup-subject"
                className="text-xs font-medium text-text/70"
              >
                {t("meetings.followup.subject")}
              </label>
              <Input
                id="followup-subject"
                className="w-full"
                value={subject}
                onChange={(e) => setSubject(e.target.value)}
              />
            </div>
            <div className="space-y-1">
              <label
                htmlFor="followup-body"
                className="text-xs font-medium text-text/70"
              >
                {t("meetings.followup.body")}
              </label>
              <Textarea
                id="followup-body"
                className="w-full font-normal"
                rows={12}
                value={body}
                onChange={(e) => setBody(e.target.value)}
              />
            </div>
          </div>
        )}

        <div aria-live="polite" data-testid="followup-status">
          {status && (
            <p
              className={`text-sm ${
                status.kind === "error"
                  ? "text-red-400"
                  : status.kind === "info"
                    ? "text-yellow-700 dark:text-yellow-300"
                    : "text-text/80"
              }`}
            >
              {status.text}
            </p>
          )}
        </div>
      </div>
    </Dialog>
  );
};
