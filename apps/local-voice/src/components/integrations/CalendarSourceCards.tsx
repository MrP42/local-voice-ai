import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarDays } from "lucide-react";
import { commands, events, type CalendarSource } from "@/bindings";
import { Button } from "../ui/Button";

/** Uhrzeit des letzten erfolgreichen Abrufs, in der Sprache der Oberflaeche. */
const timeOf = (ms: number, language: string): string =>
  new Date(ms).toLocaleTimeString(language, {
    hour: "2-digit",
    minute: "2-digit",
  });

/**
 * Kalenderquellen (M5-P5b, P5f), vom Kalender gefuehrt: die Liste, „Jetzt
 * aktualisieren“, Entfernen/Abmelden. Seit A4 wohnt das hier auf der Seite
 * „Integrationen“ (vorher in den Einstellungen); das Register spiegelt jede
 * Quelle unter gleicher Kennung und haelt Richtung und Rechte.
 */
export function useCalendarSources() {
  const [sources, setSources] = useState<CalendarSource[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);
  const [syncing, setSyncing] = useState(false);

  const load = useCallback(async () => {
    try {
      const result = await commands.calendarSourcesList();
      if (result.status === "ok") {
        setSources(result.data ?? []);
        setLoadFailed(false);
      } else {
        setLoadFailed(true);
      }
    } catch {
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

  const refresh = useCallback(async () => {
    setSyncing(true);
    const result = await commands.calendarSyncNow(null);
    setSyncing(false);
    if (result.status === "ok") setSources(result.data ?? []);
    else void load();
  }, [load]);

  const add = useCallback((source: CalendarSource) => {
    setSources((list) => [...list.filter((s) => s.id !== source.id), source]);
  }, []);

  /** Ein Microsoft-Konto wird abgemeldet (loescht auch das Token), eine
   *  ICS-Quelle entfernt (loescht die Adresse). */
  const remove = useCallback(
    async (id: string) => {
      const isGraph = sources.some((s) => s.id === id && s.kind === "graph");
      const result = isGraph
        ? await commands.calendarGraphSignOut(id)
        : await commands.calendarSourceRemove(id);
      if (result.status === "ok") {
        setSources((list) => list.filter((s) => s.id !== id));
      }
      void load();
    },
    [load, sources],
  );

  return { sources, loadFailed, syncing, load, refresh, add, remove };
}

interface CalendarSourceCardsProps {
  calendar: ReturnType<typeof useCalendarSources>;
  onConnect: () => void;
  /** Hat das Register einen Eintrag mit dieser Kennung (Rechte-Matrix)? */
  hasRights: (id: string) => boolean;
  onOpenRights: (id: string) => void;
  /** Nach Entfernen/Abmelden: das Register hat sich geaendert. */
  onChanged: () => void;
}

export const CalendarSourceCards: React.FC<CalendarSourceCardsProps> = ({
  calendar,
  onConnect,
  hasRights,
  onOpenRights,
  onChanged,
}) => {
  const { t, i18n } = useTranslation();
  const { sources, loadFailed, syncing, refresh, remove } = calendar;
  const [removing, setRemoving] = useState<string | null>(null);

  const status = (source: CalendarSource) => {
    if (source.last_error) {
      return (
        <span
          className="text-status-red break-words"
          data-testid="calendar-source-error"
        >
          {source.last_error}
        </span>
      );
    }
    if (source.last_ok_at) {
      return (
        <span className="text-text-muted" data-testid="calendar-source-status">
          {t("meetings.calendar.syncOk", {
            count: source.event_count,
            time: timeOf(source.last_ok_at, i18n.language),
          })}
        </span>
      );
    }
    return (
      <span className="text-text-muted" data-testid="calendar-source-status">
        {t("meetings.calendar.neverSynced")}
      </span>
    );
  };

  return (
    <div className="space-y-3" data-testid="calendar-settings">
      {loadFailed && (
        <p className="text-sm text-status-red">
          {t("meetings.calendar.loadError")}
        </p>
      )}
      {sources.length === 0 && !loadFailed && (
        <p className="text-sm text-text-muted" data-testid="calendar-empty">
          {t("meetings.calendar.empty")}
        </p>
      )}
      {sources.length > 0 && (
        <ul className="grid gap-3 sm:grid-cols-2">
          {sources.map((source) => (
            <li
              key={source.id}
              className="flex flex-col gap-1 rounded-lg border border-mid-gray/20 px-3 py-3 text-sm"
              data-testid="calendar-source"
              data-source-id={source.id}
            >
              <div className="flex items-start gap-2">
                <CalendarDays
                  size={18}
                  className="mt-0.5 shrink-0 text-text-muted"
                  aria-hidden="true"
                />
                <div className="min-w-0 flex-1">
                  <span className="font-medium break-words">
                    {source.label}
                  </span>
                  {source.account_hint && (
                    <span
                      className="text-text-muted break-all"
                      data-testid="calendar-source-hint"
                    >
                      {source.kind === "graph"
                        ? ` · ${source.account_hint}`
                        : ` · ${source.account_hint}/…`}
                    </span>
                  )}
                </div>
              </div>
              <div className="ps-6">{status(source)}</div>
              {source.kind === "graph" && source.last_error && (
                <div className="ps-6">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={onConnect}
                    data-testid="calendar-graph-reauth"
                  >
                    {t("meetings.calendar.graph.reauth")}
                  </Button>
                </div>
              )}
              {source.last_ok_at && !source.has_attendee_data && (
                <p className="ps-6 text-xs text-text-muted">
                  {t("meetings.calendar.attendeeDataMissing")}
                </p>
              )}
              {removing === source.id ? (
                <div
                  className="mt-1 space-y-2 ps-6"
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
                      onClick={() => {
                        setRemoving(null);
                        void remove(source.id).then(onChanged);
                      }}
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
              ) : (
                <div className="mt-1 flex flex-wrap gap-2 ps-6">
                  {hasRights(source.id) && (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => onOpenRights(source.id)}
                      data-testid="calendar-rights"
                    >
                      {t("integrations.calendar.rights")}
                    </Button>
                  )}
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
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap gap-2">
        <Button onClick={onConnect} data-testid="calendar-connect">
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
  );
};
