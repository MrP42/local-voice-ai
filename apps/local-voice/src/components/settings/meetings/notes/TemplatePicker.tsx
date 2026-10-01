import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LayoutTemplate } from "lucide-react";
import {
  commands,
  events,
  type AutoTemplateInfo,
  type TemplateInfo,
} from "@/bindings";
import { AUTO_TEMPLATE_ID, DEFAULT_TEMPLATE_ID } from "@/lib/meetingNotes";
import { Select } from "../../../ui/Select";
import { Button } from "../../../ui/Button";
import { TemplateManagerDialog } from "./TemplateManagerDialog";

interface TemplatePickerProps {
  /** Gewaehlte Vorlage; `null` = Standardvorlage. */
  value: string | null;
  onChange: (templateId: string) => void;
  disabled?: boolean;
  className?: string;
  /** P1k: "Automatisch (nach Inhalt)" als Auswahl anbieten. */
  allowAuto?: boolean;
  /** Was die Automatik zuletzt gewaehlt hat (nur bei "Automatisch" gezeigt). */
  autoInfo?: AutoTemplateInfo | null;
  /** Auswahlliste an `body` haengen (in Dialogen, sonst schneidet sie der Dialog ab). */
  menuPortal?: boolean;
}

/**
 * Vorlagenwahl mit Zugang zur Vorlagenverwaltung. Kontrolliert: der Aufrufer
 * entscheidet, wohin die Wahl geht (laufende Besprechung, Aufnahmeformular).
 * Eine geloeschte oder unbekannte Vorlage faellt auf die Standardvorlage zurueck.
 * Mit `allowAuto` kommt "Automatisch (nach Inhalt)" dazu: das Backend waehlt
 * beim Erzeugen die passende Vorlage, die Wahl des Nutzers hat immer Vorrang.
 */
export const TemplatePicker: React.FC<TemplatePickerProps> = ({
  value,
  onChange,
  disabled,
  className = "",
  allowAuto = false,
  autoInfo = null,
  menuPortal = false,
}) => {
  const { t } = useTranslation();
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [managing, setManaging] = useState(false);

  const load = useCallback(async () => {
    const result = await commands.meetingTemplatesList();
    if (result.status === "ok") setTemplates(result.data ?? []);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const options = useMemo(() => {
    const list = templates.map((info) => ({
      value: info.id,
      label: info.title,
    }));
    return allowAuto
      ? [
          { value: AUTO_TEMPLATE_ID, label: t("meetings.templates.auto") },
          ...list,
        ]
      : list;
  }, [templates, allowAuto, t]);
  const isAuto = allowAuto && value === AUTO_TEMPLATE_ID;
  const known = isAuto || templates.some((info) => info.id === value);
  const shown = known
    ? value
    : templates.length > 0
      ? DEFAULT_TEMPLATE_ID
      : null;

  return (
    <div className={`flex flex-wrap items-center gap-2 ${className}`}>
      <span className="text-xs text-text/60">
        {t("meetings.templates.label")}
      </span>
      <div className="min-w-48">
        <Select
          ariaLabel={t("meetings.templates.label")}
          value={shown}
          options={options}
          isClearable={false}
          disabled={disabled}
          menuPortal={menuPortal}
          placeholder={t("meetings.templates.label")}
          onChange={(id) => {
            if (id) onChange(id);
          }}
        />
      </div>
      {isAuto && <AutoNote info={autoInfo} />}
      <Button
        size="sm"
        variant="ghost"
        onClick={() => setManaging(true)}
        title={t("meetings.templates.manage")}
      >
        <LayoutTemplate width={14} height={14} />
        {t("meetings.templates.manage")}
      </Button>
      <TemplateManagerDialog
        open={managing}
        onOpenChange={setManaging}
        onChanged={() => void load()}
      />
    </div>
  );
};

/**
 * Was "Automatisch" gewaehlt hat: "Automatisch: Kundengespraech / Vertrieb", mit
 * der Begruendung des Modells als Tooltip. Vor der ersten Wahl steht, wann sie
 * getroffen wird; bei unklarer oder gescheiterter Wahl, dass die
 * Standardvorlage gilt.
 */
const AutoNote: React.FC<{ info: AutoTemplateInfo | null }> = ({ info }) => {
  const { t } = useTranslation();
  if (!info) {
    return (
      <span
        className="text-xs text-text/60"
        data-testid="template-auto-note"
        data-state="pending"
      >
        {t("meetings.templates.autoPending")}
      </span>
    );
  }
  const detail =
    info.outcome === "model"
      ? info.reason
        ? t("meetings.templates.autoReason", { reason: info.reason })
        : undefined
      : t(
          info.outcome === "failed"
            ? "meetings.templates.autoFailed"
            : "meetings.templates.autoUncertain",
        );
  return (
    <span
      className="text-xs text-text/70"
      data-testid="template-auto-note"
      data-state={info.outcome}
      data-template-id={info.template_id}
      title={detail}
    >
      {t("meetings.templates.autoChosen", { title: info.title })}
      {info.outcome !== "model" && detail ? ` – ${detail}` : ""}
    </span>
  );
};

/**
 * Vorlagenwahl einer bestimmten Besprechung (vor, waehrend oder nach der
 * Aufnahme). Notizen und Protokoll teilen sich diese eine Wahl, damit beide
 * zusammenpassen. "Automatisch" zeigt, was zuletzt nach Inhalt gewaehlt wurde
 * (laedt neu, sobald Notizen oder Protokoll fertig sind).
 */
export const MeetingTemplatePicker: React.FC<{
  meetingId: string;
  className?: string;
  disabled?: boolean;
  /** Meldet die gemerkte Wahl (beim Laden und bei jeder Aenderung). */
  onValueChange?: (value: string | null) => void;
  /** Auswahlliste an `body` haengen (in Dialogen). */
  menuPortal?: boolean;
}> = ({ meetingId, className, disabled, onValueChange, menuPortal }) => {
  const [value, setValue] = useState<string | null>(null);
  const [autoInfo, setAutoInfo] = useState<AutoTemplateInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingsGetTemplate(meetingId).then((result) => {
      if (!cancelled && result.status === "ok") {
        setValue(result.data);
        onValueChange?.(result.data);
      }
    });
    return () => {
      cancelled = true;
    };
    // `onValueChange` ist ein Melder, kein Eingabewert: nur der Wechsel der Besprechung laedt neu.
  }, [meetingId]);

  const loadAuto = useCallback(async () => {
    const result = await commands.meetingsGetAutoTemplate(meetingId);
    if (result.status === "ok") setAutoInfo(result.data ?? null);
  }, [meetingId]);

  useEffect(() => {
    void loadAuto();
  }, [loadAuto, value]);

  // Die Wahl nach Inhalt faellt beim Erzeugen von Notizen oder Protokoll.
  useEffect(() => {
    const unNotes = events.meetingNotesEvent.listen((event) => {
      if (
        event.payload.meeting_id === meetingId &&
        event.payload.kind === "done"
      ) {
        void loadAuto();
      }
    });
    const unMinutes = events.minutesEvent.listen((event) => {
      if (
        event.payload.meeting_id === meetingId &&
        event.payload.kind === "done"
      ) {
        void loadAuto();
      }
    });
    return () => {
      void unNotes.then((f) => f());
      void unMinutes.then((f) => f());
    };
  }, [meetingId, loadAuto]);

  return (
    <TemplatePicker
      className={className}
      value={value}
      disabled={disabled}
      allowAuto
      autoInfo={autoInfo}
      menuPortal={menuPortal}
      onChange={(id) => {
        setValue(id);
        onValueChange?.(id);
        void commands.meetingsSetTemplate(meetingId, id);
      }}
    />
  );
};
