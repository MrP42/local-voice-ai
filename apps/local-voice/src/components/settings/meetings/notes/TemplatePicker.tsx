import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LayoutTemplate } from "lucide-react";
import { commands, type TemplateInfo } from "@/bindings";
import { DEFAULT_TEMPLATE_ID } from "@/lib/meetingNotes";
import { Select } from "../../../ui/Select";
import { Button } from "../../../ui/Button";
import { TemplateManagerDialog } from "./TemplateManagerDialog";

interface TemplatePickerProps {
  /** Gewaehlte Vorlage; `null` = Standardvorlage. */
  value: string | null;
  onChange: (templateId: string) => void;
  disabled?: boolean;
  className?: string;
}

/**
 * Vorlagenwahl mit Zugang zur Vorlagenverwaltung. Kontrolliert: der Aufrufer
 * entscheidet, wohin die Wahl geht (laufende Besprechung, Aufnahmeformular).
 * Eine geloeschte oder unbekannte Vorlage faellt auf die Standardvorlage zurueck.
 */
export const TemplatePicker: React.FC<TemplatePickerProps> = ({
  value,
  onChange,
  disabled,
  className = "",
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

  const options = useMemo(
    () => templates.map((info) => ({ value: info.id, label: info.title })),
    [templates],
  );
  const known = templates.some((info) => info.id === value);
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
      <div className="min-w-48" aria-label={t("meetings.templates.label")}>
        <Select
          value={shown}
          options={options}
          isClearable={false}
          disabled={disabled}
          placeholder={t("meetings.templates.label")}
          onChange={(id) => {
            if (id) onChange(id);
          }}
        />
      </div>
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

/** Vorlagenwahl einer bestimmten Besprechung (vor, waehrend oder nach der Aufnahme). */
export const MeetingTemplatePicker: React.FC<{
  meetingId: string;
  className?: string;
}> = ({ meetingId, className }) => {
  const [value, setValue] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingsGetTemplate(meetingId).then((result) => {
      if (!cancelled && result.status === "ok") setValue(result.data);
    });
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  return (
    <TemplatePicker
      className={className}
      value={value}
      onChange={(id) => {
        setValue(id);
        void commands.meetingsSetTemplate(meetingId, id);
      }}
    />
  );
};
