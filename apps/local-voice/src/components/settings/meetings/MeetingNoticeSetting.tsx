import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type MeetingNoticeSettings,
  type NoticeApp,
  type NoticeLang,
} from "@/bindings";
import {
  NOTICE_APPS,
  NOTICE_LANGS,
  noticeSettingsOf,
  withTemplate,
} from "@/lib/meetingNotice";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { Input } from "../../ui/Input";
import { SettingContainer } from "../../ui/SettingContainer";
import { Textarea } from "../../ui/Textarea";
import { ToggleSwitch } from "../../ui/ToggleSwitch";

/** Eine Zeile mit Eingabefeld: gespeichert wird beim Verlassen oder mit Enter. */
const TextRow: React.FC<{
  title: string;
  description: string;
  placeholder: string;
  value: string;
  maxLength: number;
  testId: string;
  onCommit: (value: string) => void;
  disabled: boolean;
}> = ({
  title,
  description,
  placeholder,
  value,
  maxLength,
  testId,
  onCommit,
  disabled,
}) => {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    const next = draft.trim();
    if (next !== value.trim()) onCommit(next);
  };
  return (
    <SettingContainer title={title} description={description} grouped={true}>
      <Input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
        placeholder={placeholder}
        aria-label={title}
        maxLength={maxLength}
        autoComplete="off"
        disabled={disabled}
        data-testid={testId}
        className="w-72"
      />
    </SettingContainer>
  );
};

/** Bearbeiten des Textbausteins je Besprechungs-App und Sprache. */
const TemplateEditor: React.FC<{
  cfg: MeetingNoticeSettings;
  save: (next: MeetingNoticeSettings) => void;
  disabled: boolean;
}> = ({ cfg, save, disabled }) => {
  const { t } = useTranslation();
  const [app, setApp] = useState<NoticeApp>(cfg.default_app);
  const [lang, setLang] = useState<NoticeLang>(cfg.default_lang);
  const [stored, setStored] = useState("");
  const [draft, setDraft] = useState("");
  const custom = cfg.templates.some((x) => x.app === app && x.lang === lang);
  const templatesKey = JSON.stringify(cfg.templates);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const r = await commands.meetingNoticeTemplate(app, lang);
        if (cancelled || r.status !== "ok") return;
        setStored(r.data ?? "");
        setDraft(r.data ?? "");
      } catch {
        // der Baustein bleibt leer
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app, lang, templatesKey]);

  return (
    <SettingContainer
      title={t("settings.meetings.notice.template")}
      description={t("settings.meetings.notice.templateDescription")}
      layout="stacked"
      descriptionMode="inline"
      grouped={true}
    >
      <div className="space-y-2" data-testid="notice-template-editor">
        <div className="flex flex-wrap items-center gap-2">
          <Dropdown
            className="w-44"
            ariaLabel={t("meetings.notice.app")}
            selectedValue={app}
            options={NOTICE_APPS.map((a) => ({
              value: a,
              label: t(`meetings.notice.apps.${a}`),
            }))}
            onSelect={(v) => setApp(v as NoticeApp)}
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
          />
          <span
            className="text-xs text-text/70"
            data-testid="notice-template-state"
          >
            {custom
              ? t("settings.meetings.notice.templateCustom")
              : t("settings.meetings.notice.templateBuiltin")}
          </span>
        </div>
        <Textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          rows={6}
          maxLength={2000}
          aria-label={t("settings.meetings.notice.template")}
          disabled={disabled}
          data-testid="notice-template-text"
          className="w-full"
        />
        <p className="text-xs text-text/70">
          {t("settings.meetings.notice.placeholders")}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            variant="primary"
            disabled={disabled || draft.trim() === "" || draft === stored}
            onClick={() => save(withTemplate(cfg, app, lang, draft))}
            data-testid="notice-template-save"
          >
            {t("settings.meetings.notice.save")}
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={disabled || !custom}
            onClick={() => save(withTemplate(cfg, app, lang, ""))}
            data-testid="notice-template-reset"
          >
            {t("settings.meetings.notice.restore")}
          </Button>
        </div>
      </div>
    </SettingContainer>
  );
};

/**
 * G2 (#497): Zeilen "Hinweis an Teilnehmende" in der Gruppe Besprechungen
 * (Einstellungen > Eingabe): Funktion ein/aus, Zeile im Protokollexport,
 * Vorgaben für App und Sprache, Zweck, Aufbewahrung und der Textbaustein. Die
 * Pflicht vor Aufnahmebeginn steht bei den Regeln (`ComplianceSettings`).
 */
export const MeetingNoticeSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const cfg = noticeSettingsOf(getSetting("meeting_notice"));
  const updating = isUpdating("meeting_notice");
  const save = (next: MeetingNoticeSettings) =>
    void updateSetting("meeting_notice", next);
  const patch = (p: Partial<MeetingNoticeSettings>) => save({ ...cfg, ...p });

  return (
    <>
      <ToggleSwitch
        checked={cfg.enabled}
        onChange={(v) => patch({ enabled: v })}
        isUpdating={updating}
        label={t("settings.meetings.notice.enabled")}
        description={t("settings.meetings.notice.enabledDescription")}
        descriptionMode="tooltip"
        grouped={true}
        testId="notice-enabled"
      />
      {cfg.enabled && (
        <>
          <ToggleSwitch
            checked={cfg.export_line}
            onChange={(v) => patch({ export_line: v })}
            isUpdating={updating}
            label={t("settings.meetings.notice.exportLine")}
            description={t("settings.meetings.notice.exportLineDescription")}
            descriptionMode="tooltip"
            grouped={true}
            testId="notice-export-line"
          />
          <SettingContainer
            title={t("settings.meetings.notice.defaultApp")}
            description={t("settings.meetings.notice.defaultAppDescription")}
            grouped={true}
          >
            <Dropdown
              className="w-44"
              selectedValue={cfg.default_app}
              options={NOTICE_APPS.map((a) => ({
                value: a,
                label: t(`meetings.notice.apps.${a}`),
              }))}
              onSelect={(v) => patch({ default_app: v as NoticeApp })}
              disabled={updating}
            />
          </SettingContainer>
          <SettingContainer
            title={t("settings.meetings.notice.defaultLang")}
            description={t("settings.meetings.notice.defaultLangDescription")}
            grouped={true}
          >
            <Dropdown
              className="w-36"
              selectedValue={cfg.default_lang}
              options={NOTICE_LANGS.map((l) => ({
                value: l,
                label: t(`meetings.notice.langs.${l}`),
              }))}
              onSelect={(v) => patch({ default_lang: v as NoticeLang })}
              disabled={updating}
            />
          </SettingContainer>
          <TextRow
            title={t("settings.meetings.notice.purpose")}
            description={t("settings.meetings.notice.purposeDescription")}
            placeholder={t("settings.meetings.notice.purposePlaceholder")}
            value={cfg.purpose}
            maxLength={200}
            testId="notice-purpose"
            onCommit={(purpose) => patch({ purpose })}
            disabled={updating}
          />
          <TextRow
            title={t("settings.meetings.notice.retention")}
            description={t("settings.meetings.notice.retentionDescription")}
            placeholder={t("settings.meetings.notice.retentionPlaceholder")}
            value={cfg.retention}
            maxLength={300}
            testId="notice-retention"
            onCommit={(retention) => patch({ retention })}
            disabled={updating}
          />
          <TemplateEditor cfg={cfg} save={save} disabled={updating} />
        </>
      )}
    </>
  );
};
