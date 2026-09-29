import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, events, type CalendarSource } from "@/bindings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";
import { CalendarConnectDialog } from "./CalendarConnectDialog";

/** Uhrzeit des letzten erfolgreichen Abrufs, in der Sprache der Oberflaeche. */
const timeOf = (ms: number, language: string): string =>
  new Date(ms).toLocaleTimeString(language, {
    hour: "2-digit",
    minute: "2-digit",
  });

const REMINDER_CHOICES = [
  { value: "0", key: "off" },
  { value: "60", key: "minute1" },
  { value: "120", key: "minutes2" },
  { value: "300", key: "minutes5" },
] as const;

/**
 * Kalender und Erinnerung (M5-P5b) als Zeilen der Gruppe "Besprechungen":
 * die Liste der verbundenen ICS-Quellen mit Stand je Quelle, "Kalender
 * verbinden", "Jetzt aktualisieren", und die Erinnerung vor Terminen.
 */
export const MeetingCalendarSettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [sources, setSources] = useState<CalendarSource[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await commands.calendarSourcesList();
    if (result.status === "ok") {
      setSources(result.data ?? []);
      setLoadFailed(false);
    } else {
      setLoadFailed(true);
    }
  }, []);

  useEffect(() => {
    void load();
    const un = events.calendarSyncEvent.listen(() => void load());
    return () => {
      void un.then((f) => f());
    };
  }, [load]);

  const refresh = async () => {
    setSyncing(true);
    const result = await commands.calendarSyncNow(null);
    setSyncing(false);
    if (result.status === "ok") setSources(result.data ?? []);
    else void load();
  };

  const remove = async (id: string) => {
    setRemoving(null);
    // Ein Microsoft-Konto wird abgemeldet (loescht auch das Token), eine
    // ICS-Quelle entfernt (loescht die Adresse).
    const isGraph = sources.some((s) => s.id === id && s.kind === "graph");
    const result = isGraph
      ? await commands.calendarGraphSignOut(id)
      : await commands.calendarSourceRemove(id);
    if (result.status === "ok") {
      setSources((list) => list.filter((s) => s.id !== id));
    }
    void load();
  };

  const status = (source: CalendarSource) => {
    if (source.last_error) {
      return (
        <span
          className="text-red-500 break-words"
          data-testid="calendar-source-error"
        >
          {source.last_error}
        </span>
      );
    }
    if (source.last_ok_at) {
      return (
        <span className="text-text/70" data-testid="calendar-source-status">
          {t("meetings.calendar.syncOk", {
            count: source.event_count,
            time: timeOf(source.last_ok_at, i18n.language),
          })}
        </span>
      );
    }
    return (
      <span className="text-text/70" data-testid="calendar-source-status">
        {t("meetings.calendar.neverSynced")}
      </span>
    );
  };

  const lead = String(getSetting("meeting_reminder_lead_s") ?? 60);
  const reminderOptions: { value: string; label: string }[] =
    REMINDER_CHOICES.map((c) => ({
      value: c.value,
      label: t(`meetings.calendar.reminder.${c.key}`),
    }));
  // Ein eigener Wert aus der Einstellungsdatei (z. B. 90 s) bleibt sichtbar.
  if (!reminderOptions.some((o) => o.value === lead)) {
    reminderOptions.push({
      value: lead,
      label: `${Math.round(Number(lead) / 60)} min`,
    });
  }

  return (
    <>
      <SettingContainer
        title={t("meetings.calendar.title")}
        description={t("meetings.calendar.description")}
        grouped={true}
        layout="stacked"
        descriptionMode="tooltip"
      >
        <div className="space-y-2" data-testid="calendar-settings">
          {loadFailed && (
            <p className="text-sm text-red-500">
              {t("meetings.calendar.loadError")}
            </p>
          )}
          {sources.length === 0 && !loadFailed && (
            <p className="text-sm text-text/70" data-testid="calendar-empty">
              {t("meetings.calendar.empty")}
            </p>
          )}
          {sources.length > 0 && (
            <ul className="space-y-2">
              {sources.map((source) => (
                <li
                  key={source.id}
                  className="rounded-lg border border-mid-gray/20 px-3 py-2 text-sm"
                  data-testid="calendar-source"
                  data-source-id={source.id}
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <span className="font-medium">{source.label}</span>
                      {source.account_hint && (
                        <span
                          className="text-text/60"
                          data-testid="calendar-source-hint"
                        >
                          {source.kind === "graph"
                            ? ` · ${source.account_hint}`
                            : ` · ${source.account_hint}/…`}
                        </span>
                      )}
                    </div>
                    {removing === source.id ? null : (
                      <Button
                        variant="danger-ghost"
                        size="sm"
                        onClick={() => setRemoving(source.id)}
                        data-testid="calendar-remove"
                      >
                        {source.kind === "graph"
                          ? t("meetings.calendar.graph.signOut")
                          : t("meetings.calendar.remove")}
                      </Button>
                    )}
                  </div>
                  <div>{status(source)}</div>
                  {source.kind === "graph" && source.last_error && (
                    <Button
                      variant="secondary"
                      size="sm"
                      className="mt-1"
                      onClick={() => setDialogOpen(true)}
                      data-testid="calendar-graph-reauth"
                    >
                      {t("meetings.calendar.graph.reauth")}
                    </Button>
                  )}
                  {source.last_ok_at && !source.has_attendee_data && (
                    <p className="text-xs text-text/60">
                      {t("meetings.calendar.attendeeDataMissing")}
                    </p>
                  )}
                  {removing === source.id && (
                    <div
                      className="mt-2 space-y-2"
                      data-testid="calendar-remove-confirm"
                    >
                      <p className="text-text/80">
                        {source.kind === "graph"
                          ? t("meetings.calendar.graph.signOutConfirm", {
                              label: source.account_hint ?? source.label,
                            })
                          : t("meetings.calendar.removeConfirm", {
                              label: source.label,
                            })}
                      </p>
                      <div className="flex gap-2">
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => void remove(source.id)}
                          data-testid="calendar-remove-yes"
                        >
                          {source.kind === "graph"
                            ? t("meetings.calendar.graph.signOutConfirmYes")
                            : t("meetings.calendar.removeConfirmYes")}
                        </Button>
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => setRemoving(null)}
                        >
                          {t("meetings.calendar.cancel")}
                        </Button>
                      </div>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              onClick={() => setDialogOpen(true)}
              data-testid="calendar-connect"
            >
              {t("meetings.calendar.connect")}
            </Button>
            {sources.length > 0 && (
              <Button
                variant="secondary"
                onClick={() => void refresh()}
                disabled={syncing}
                data-testid="calendar-refresh"
              >
                {syncing
                  ? t("meetings.calendar.refreshing")
                  : t("meetings.calendar.refresh")}
              </Button>
            )}
          </div>
        </div>
      </SettingContainer>

      <SettingContainer
        title={t("meetings.calendar.reminder.title")}
        description={t("meetings.calendar.reminder.description")}
        grouped={true}
      >
        <Dropdown
          options={reminderOptions}
          selectedValue={lead}
          onSelect={(value) =>
            void updateSetting("meeting_reminder_lead_s", Number(value))
          }
          disabled={isUpdating("meeting_reminder_lead_s")}
        />
      </SettingContainer>
      <ToggleSwitch
        checked={getSetting("meeting_reminder_all_events") ?? false}
        onChange={(v) => void updateSetting("meeting_reminder_all_events", v)}
        isUpdating={isUpdating("meeting_reminder_all_events")}
        disabled={lead === "0"}
        label={t("meetings.calendar.reminder.allEvents")}
        description={t("meetings.calendar.reminder.allEventsDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />

      <CalendarConnectDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onAdded={(source) =>
          setSources((list) => [
            ...list.filter((s) => s.id !== source.id),
            source,
          ])
        }
      />
    </>
  );
};
