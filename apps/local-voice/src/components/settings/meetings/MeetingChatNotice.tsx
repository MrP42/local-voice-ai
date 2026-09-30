import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";
import type { PostProcessProvider } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { IconAction } from "../../ui/IconAction";

/** Platzhalter-Adresse des lokalen Servers (`managers::llm::LOCAL_PLACEHOLDER_URL`). */
const LOCAL_PLACEHOLDER_URL = "http://127.0.0.1:0/v1";

/**
 * Ist dieser Anbieter der lokale Server? Wie im Backend (`llm::is_local`)
 * erkannt an der festen ID oder der Platzhalter-Adresse, nicht am Namen.
 */
export const isLocalProvider = (
  provider: Pick<PostProcessProvider, "id" | "base_url">,
): boolean =>
  provider.id === "local" ||
  provider.base_url.replace(/\/+$/, "") ===
    LOCAL_PLACEHOLDER_URL.replace(/\/+$/, "");

/**
 * Der Hinweistext fuer den Meeting-Chat (F26). Ohne aktiven Anbieter gibt es
 * keine Notizen und nichts wird uebermittelt: es gilt die lokale Fassung.
 */
export const useChatNoticeText = (): string => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const providerId = getSetting("post_process_provider_id");
  const providers = (getSetting("post_process_providers") ??
    []) as PostProcessProvider[];
  const active = providers.find((p) => p.id === providerId);
  if (!active || isLocalProvider(active)) {
    return t("meetings.record.chatNotice.local");
  }
  return t("meetings.record.chatNotice.remote", { provider: active.label });
};

interface MeetingChatNoticeProps {
  /** Praefix fuer `data-testid` (Aufnahmekarte und Einwilligungsdialog). */
  testId: string;
  /** Nur ein Symbolknopf (laufende Aufnahme): der Text steht im Tooltip. */
  compact?: boolean;
}

/**
 * Hinweis zum Kopieren in den Chat der Besprechung: wer mitliest, erfaehrt,
 * dass mitgeschrieben wird und wohin der Text geht. Der Kopierknopf meldet
 * Erfolg und Misserfolg sichtbar (und per `aria-live`).
 */
export const MeetingChatNotice: React.FC<MeetingChatNoticeProps> = ({
  testId,
  compact = false,
}) => {
  const { t } = useTranslation();
  const text = useChatNoticeText();
  const [state, setState] = useState<"idle" | "copied" | "error">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("error");
      // Der Symbolknopf hat keine Textzeile: Misserfolg als Meldung zeigen.
      if (compact) toast.error(t("meetings.record.chatNotice.copyError"));
    }
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setState("idle"), 2500);
  };

  if (compact) {
    return (
      <span className="inline-flex" data-testid={`${testId}-chat-notice`}>
        <span
          className="sr-only select-text"
          data-testid={`${testId}-chat-notice-text`}
        >
          {text}
        </span>
        <IconAction
          icon={state === "copied" ? Check : Copy}
          label={
            state === "copied"
              ? t("meetings.recorder.noticeCopied")
              : t("meetings.recorder.noticeName")
          }
          description={text}
          testId={`${testId}-chat-notice-copy`}
          onClick={() => void copy()}
        />
        <span role="status" aria-live="polite" className="sr-only">
          {state === "error" ? t("meetings.record.chatNotice.copyError") : ""}
        </span>
      </span>
    );
  }

  return (
    <div
      className="space-y-1.5 rounded-lg border border-mid-gray/20 p-3"
      data-testid={`${testId}-chat-notice`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-mid-gray uppercase tracking-wide">
          {t("meetings.record.chatNotice.title")}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void copy()}
          data-testid={`${testId}-chat-notice-copy`}
        >
          {state === "copied" ? (
            <Check width={14} height={14} />
          ) : (
            <Copy width={14} height={14} />
          )}
          {state === "copied"
            ? t("meetings.record.chatNotice.copied")
            : t("meetings.record.chatNotice.copy")}
        </Button>
      </div>
      <p
        className="text-sm text-text/80 select-text"
        data-testid={`${testId}-chat-notice-text`}
      >
        {text}
      </p>
      <p
        role="status"
        aria-live="polite"
        className="text-xs text-red-500 empty:hidden"
      >
        {state === "error" ? t("meetings.record.chatNotice.copyError") : ""}
      </p>
    </div>
  );
};
