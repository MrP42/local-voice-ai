import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  ClipboardPaste,
  Copy,
  Megaphone,
  UserCheck,
} from "lucide-react";
import {
  commands,
  type NoticeApp,
  type NoticeLang,
  type NoticeOutcome,
  type NoticeRecord,
} from "@/bindings";
import {
  NOTICE_APPS,
  NOTICE_LANGS,
  PASTE_COUNTDOWN_SECS,
  latestRecord,
  noticeErrorCode,
  noticeIsRequired,
  noticeSettingsOf,
  noticeTime,
} from "@/lib/meetingNotice";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Dropdown } from "../../ui/Dropdown";
import { IconAction } from "../../ui/IconAction";
import { MeetingChatNotice } from "./MeetingChatNotice";

interface ParticipantNoticeProps {
  /** Besprechung, an der der Nachweis haftet; `null` = vor dem Start (Startdialog). */
  meetingId: string | null;
  /** Präfix für `data-testid` ("consent", "recording"). */
  testId: string;
  /** Meldet, wie viele Nachweise vorliegen (der Startdialog sperrt damit bei Pflicht). */
  onRecordsChange?: (count: number) => void;
}

type Message = { kind: "info" | "error"; text: string };

/**
 * Hinweis an Teilnehmende (G2, #497): ein vorbereiteter Text für den
 * Besprechungs-Chat (Teams, Webex oder neutral, Deutsch oder Englisch). Er lässt
 * sich kopieren oder auf Knopfdruck in das Chatfeld der Besprechungs-App
 * einfügen; gesendet wird nie von hier. Dazu die Bestätigung "mündlich
 * informiert". Jede Aktion hält das Backend als Nachweis an der Besprechung
 * fest (wann, welcher Text, auf welchem Weg); die Statuszeile zeigt den
 * jüngsten. Ist die Funktion abgeschaltet, bleibt die einfache Kopierzeile.
 */
export const ParticipantNotice: React.FC<ParticipantNoticeProps> = ({
  meetingId,
  testId,
  onRecordsChange,
}) => {
  const { getSetting } = useSettings();
  const cfg = noticeSettingsOf(getSetting("meeting_notice"));
  if (!cfg.enabled) return <MeetingChatNotice testId={testId} />;
  return (
    <NoticePanel
      meetingId={meetingId}
      testId={testId}
      onRecordsChange={onRecordsChange}
    />
  );
};

const NoticePanel: React.FC<ParticipantNoticeProps> = ({
  meetingId,
  testId,
  onRecordsChange,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting } = useSettings();
  const cfg = noticeSettingsOf(getSetting("meeting_notice"));
  const [app, setApp] = useState<NoticeApp>(cfg.default_app);
  const [lang, setLang] = useState<NoticeLang>(cfg.default_lang);
  const [text, setText] = useState("");
  const [records, setRecords] = useState<NoticeRecord[]>([]);
  const [pasteAvailable, setPasteAvailable] = useState(false);
  const [message, setMessage] = useState<Message | null>(null);
  const [busy, setBusy] = useState(false);
  const [countdown, setCountdown] = useState<number | null>(null);
  const onChangeRef = useRef(onRecordsChange);
  onChangeRef.current = onRecordsChange;
  // Wirkt sich auf den Text aus: Vorlagen, Zweck, Aufbewahrung und der Anbieter
  // der Notizen (lokal oder extern).
  const textKey = JSON.stringify([
    cfg.templates,
    cfg.purpose,
    cfg.retention,
    getSetting("post_process_provider_id"),
  ]);

  const refresh = useCallback(async () => {
    try {
      const r = await commands.meetingNoticeStatus(meetingId);
      if (r.status !== "ok" || !r.data) return;
      setRecords(r.data.records ?? []);
      setPasteAvailable(r.data.paste_available === true);
    } catch {
      // Ohne Antwort bleibt der Stand, wie er war.
    }
  }, [meetingId]);

  // Vor dem Start beginnt der Dialog von vorn: Nachweise eines früheren,
  // abgebrochenen Dialogs gelten nicht für diese Aufnahme.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        if (meetingId === null) await commands.meetingNoticeResetPending();
      } catch {
        // egal: der Stand wird gleich neu gelesen
      }
      if (!cancelled) await refresh();
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingId, refresh]);

  useEffect(() => {
    onChangeRef.current?.(records.length);
  }, [records.length]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const r = await commands.meetingNoticePreview(app, lang);
        if (!cancelled && r.status === "ok") setText(r.data ?? "");
      } catch {
        if (!cancelled) setText("");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app, lang, textKey]);

  const errorText = (error: string): string => {
    const code = noticeErrorCode(error);
    const key = `meetings.notice.errors.${code}`;
    return i18n.exists(key) ? t(key) : error;
  };

  const settle = async (
    action: () => Promise<
      { status: "ok"; data: NoticeOutcome } | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    setMessage(null);
    try {
      const r = await action();
      if (r.status === "error") {
        const code = noticeErrorCode(r.error);
        setMessage({
          kind: code === "notice_cancelled" ? "info" : "error",
          text: errorText(r.error),
        });
      } else if (r.data?.fallback) {
        setMessage({
          kind: "info",
          text: t(`meetings.notice.fallback.${r.data.fallback}`),
        });
      }
    } catch (e) {
      setMessage({ kind: "error", text: String(e) });
    } finally {
      await refresh();
      setBusy(false);
    }
  };

  const copy = () =>
    settle(() => commands.meetingNoticeCopy(meetingId, app, lang));
  const oral = () =>
    settle(() => commands.meetingNoticeOral(meetingId, app, lang));
  const paste = async () => {
    setCountdown(PASTE_COUNTDOWN_SECS);
    const timer = setInterval(
      () => setCountdown((c) => (c !== null && c > 1 ? c - 1 : c)),
      1000,
    );
    try {
      await settle(() =>
        commands.meetingNoticePaste(meetingId, app, lang, PASTE_COUNTDOWN_SECS),
      );
    } finally {
      clearInterval(timer);
      setCountdown(null);
    }
  };

  const last = latestRecord(records);
  const appLabel = (a: NoticeApp) => t(`meetings.notice.apps.${a}`);
  const status = last
    ? t(`meetings.notice.status.${last.way}`, {
        time: noticeTime(last.at, i18n.language),
        app: appLabel(last.app),
      })
    : t("meetings.notice.status.none");
  const required = noticeIsRequired(cfg);
  const waiting = countdown !== null;

  return (
    <section
      className="space-y-2 rounded-lg border border-mid-gray/20 p-3"
      aria-labelledby={`${testId}-notice-title`}
      data-testid={`${testId}-notice`}
    >
      <div className="space-y-0.5">
        <h3
          id={`${testId}-notice-title`}
          className="flex items-center gap-1.5 text-sm font-semibold"
        >
          <Megaphone width={14} height={14} aria-hidden="true" />
          {t("meetings.notice.title")}
        </h3>
        <p className="text-xs text-text/70">
          {t("meetings.notice.description")}
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Dropdown
          className="w-44"
          ariaLabel={t("meetings.notice.app")}
          selectedValue={app}
          options={NOTICE_APPS.map((a) => ({ value: a, label: appLabel(a) }))}
          onSelect={(v) => setApp(v as NoticeApp)}
          disabled={busy}
        />
        <Dropdown
          className="w-36"
          ariaLabel={t("meetings.notice.lang")}
          selectedValue={lang}
          options={NOTICE_LANGS.map((l) => ({
            value: l,
            label: t(`meetings.notice.langs.${l}`),
          }))}
          onSelect={(v) => setLang(v as NoticeLang)}
          disabled={busy}
        />
      </div>

      <p
        className="whitespace-pre-wrap text-sm text-text/80 select-text"
        data-testid={`${testId}-notice-text`}
      >
        {text}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          onClick={() => void copy()}
          disabled={busy || text === ""}
          data-testid={`${testId}-notice-copy`}
        >
          <Copy width={14} height={14} aria-hidden="true" />
          {t("meetings.notice.copy")}
        </Button>
        {pasteAvailable && (
          <Button
            type="button"
            size="sm"
            onClick={() => void paste()}
            disabled={busy || text === ""}
            data-testid={`${testId}-notice-paste`}
          >
            <ClipboardPaste width={14} height={14} aria-hidden="true" />
            {t("meetings.notice.paste")}
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={() => void oral()}
          disabled={busy}
          data-testid={`${testId}-notice-oral`}
        >
          <Check width={14} height={14} aria-hidden="true" />
          {t("meetings.notice.oral")}
        </Button>
      </div>

      {waiting && (
        <div
          className="flex flex-wrap items-center gap-2 rounded-md bg-logo-primary/10 p-2 text-sm"
          data-testid={`${testId}-notice-wait`}
        >
          <span role="status" aria-live="polite">
            {t("meetings.notice.waiting")}
          </span>
          <span
            className="tabular-nums font-semibold"
            aria-hidden="true"
            data-testid={`${testId}-notice-countdown`}
          >
            {countdown}
          </span>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            onClick={() => void commands.meetingNoticeCancelPaste()}
            data-testid={`${testId}-notice-cancel`}
          >
            {t("meetings.notice.cancel")}
          </Button>
        </div>
      )}

      <p
        role="status"
        aria-live="polite"
        className="text-xs text-text/80"
        data-testid={`${testId}-notice-status`}
        data-state={last?.way ?? "none"}
      >
        {status}
      </p>
      <p
        role="status"
        aria-live="polite"
        className={`text-xs empty:hidden ${
          message?.kind === "error" ? "text-red-500" : "text-text/80"
        }`}
        data-testid={`${testId}-notice-message`}
      >
        {message?.text ?? ""}
      </p>
      {required && (
        <p
          id={`${testId}-notice-required-text`}
          className="text-xs text-text/70"
          data-testid={`${testId}-notice-required`}
        >
          {t("meetings.notice.required")}
        </p>
      )}
    </section>
  );
};

interface ParticipantNoticeButtonProps {
  meetingId: string;
}

/**
 * Während der Aufnahme: ein Symbolknopf in der Bedienspalte, der den Hinweis
 * in einem Dialog öffnet (kopieren, einfügen, "mündlich informiert") und mit
 * einem Häkchen zeigt, dass ein Nachweis vorliegt. Ohne die Funktion bleibt der
 * einfache Kopierknopf.
 */
export const ParticipantNoticeButton: React.FC<
  ParticipantNoticeButtonProps
> = ({ meetingId }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const cfg = noticeSettingsOf(getSetting("meeting_notice"));
  const [open, setOpen] = useState(false);
  const [count, setCount] = useState(0);

  useEffect(() => {
    if (!cfg.enabled) return;
    let cancelled = false;
    void (async () => {
      try {
        const r = await commands.meetingNoticeStatus(meetingId);
        if (!cancelled && r.status === "ok" && r.data) {
          setCount(r.data.records?.length ?? 0);
        }
      } catch {
        // kein Häkchen
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [meetingId, cfg.enabled]);

  if (!cfg.enabled) return <MeetingChatNotice testId="recording" compact />;
  return (
    <>
      <IconAction
        icon={count > 0 ? UserCheck : Megaphone}
        label={t("meetings.recorder.noticeOpen")}
        description={
          count > 0
            ? t("meetings.recorder.noticeOpenDone")
            : t("meetings.recorder.noticeOpenHint")
        }
        testId="recording-notice-open"
        onClick={() => setOpen(true)}
      />
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title={t("meetings.notice.title")}
        closeLabel={t("meetings.notice.close")}
      >
        <ParticipantNotice
          meetingId={meetingId}
          testId="recording"
          onRecordsChange={setCount}
        />
      </Dialog>
    </>
  );
};
