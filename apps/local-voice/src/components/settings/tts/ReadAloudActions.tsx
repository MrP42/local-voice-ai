import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BookOpen,
  Download,
  FilePlus2,
  FileText,
  Languages,
  Link,
  Menu,
  Mic,
  Plus,
  SlidersHorizontal,
  SpellCheck,
  Tags,
  Upload,
  WandSparkles,
  Zap,
} from "lucide-react";
import { ActionMenu, ActionPopover } from "../../ui/ActionMenu";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { IconAction } from "../../ui/IconAction";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { Glyph } from "../../ui/AudioPlayer";
import { Select } from "../../ui/Select";
import { TTS_TARGET_LANGS } from "../../../lib/constants/languages";
import { AutoTagBar, type AutoTagBarProps } from "./tags/AutoTagBar";

export type TtsTab = "original" | "translation" | "summary";

interface Progress {
  position: number;
  total: number;
}

export interface ReadAloudActionsProps {
  tab: TtsTab;
  /** Der Text des aktiven Reiters ist nicht leer (Übersetzen, Aufbereiten …
   *  arbeiten immer auf dem Original). */
  hasText: boolean;
  /** Es gibt etwas zum Vorlesen (Speichern, Vorab erzeugen, Prüfen). */
  hasSpokenText: boolean;

  onLoadDocument: () => void;
  onLoadUrl: () => void;
  onAddProjectFile: () => void;

  dictating: boolean;
  onToggleDictation: () => void;

  saving: boolean;
  onSave: () => void;
  exportProgress: Progress | null;
  exportEta: number | null;
  formatEta: (seconds: number) => string;
  onCancelExport: () => void;

  prewarm: { done: number; total: number } | null;
  onPrewarm: () => void;
  onCancelPrewarm: () => void;

  speakProgress: Progress | null;

  onOpenWorkshop: () => void;
  tidying: boolean;
  onTidy: () => void;
  errorCount: number;
  onCheckScript: () => void;
  /** Props der Auto-Tagging-Leiste ohne Dialogsteuerung — die liegt hier. */
  autoTag: Omit<
    AutoTagBarProps,
    "open" | "onOpenChange" | "hideTrigger" | "onLoadingChange" | "showSettings"
  >;

  targetLang: string;
  onTargetLangChange: (value: string) => void;
  translating: boolean;
  onTranslate: () => void;

  summarizing: boolean;
  onSummarize: () => void;
  sumLength: string;
  onSumLengthChange: (value: string) => void;
  sumDetail: string;
  onSumDetailChange: (value: string) => void;
  sumAudience: string;
  onSumAudienceChange: (value: string) => void;
}

/** Fortschrittsbalken mit Text und Abbrechen — die Zeilen unter den Knöpfen. */
const ProgressLine: React.FC<{
  ratio: number;
  text: string;
  cancelLabel: string;
  onCancel: () => void;
  tabular?: boolean;
}> = ({ ratio, text, cancelLabel, onCancel, tabular }) => (
  <div className="flex items-center gap-2">
    <div className="h-1.5 w-32 shrink-0 overflow-hidden rounded-full bg-mid-gray/20">
      <div
        className="h-full bg-logo-primary transition-[width] duration-200"
        style={{ width: `${ratio * 100}%` }}
      />
    </div>
    <span
      className={`text-xs text-text/60 ${tabular ? "tabular-nums" : ""}`.trim()}
    >
      {text}
    </span>
    <button
      type="button"
      className="mbtn mbtn--sm"
      onClick={onCancel}
      aria-label={cancelLabel}
    >
      <Glyph name="stop" />
    </button>
  </div>
);

/**
 * Die Aktionen der Vorlesen-Bedienspalte als EINE Zeile gleich großer
 * Symbol-Knöpfe. Je Reiter nur, was er braucht; Seltenes wohnt hinter dem
 * Menü (☰). Namen und Erklärungen stehen im Tooltip — kein Knopf trägt Text.
 * Das Verhalten jeder Aktion liegt weiter in TtsSettings; hier sitzt nur die
 * Darstellung samt dem, was nur die Zeile selbst wissen muss (offene Menüs,
 * Dialog des Auto-Taggings).
 */
export const ReadAloudActions: React.FC<ReadAloudActionsProps> = (props) => {
  const { t } = useTranslation();
  const {
    tab,
    hasText,
    hasSpokenText,
    dictating,
    saving,
    exportProgress,
    exportEta,
    prewarm,
    speakProgress,
    tidying,
    errorCount,
  } = props;
  const [autoTagOpen, setAutoTagOpen] = useState(false);
  const [autoTagLoading, setAutoTagLoading] = useState(false);
  // Die Leiste hängt nur am Original-Reiter; mit ihr verschwindet auch ihr
  // Ladezustand, und der Menüeintrag darf nicht auf "läuft" hängen bleiben.
  useEffect(() => {
    if (tab !== "original") setAutoTagLoading(false);
  }, [tab]);

  const addItems: ActionMenuItem[] = [
    {
      id: "document",
      label: t("tts.add.document"),
      icon: Upload,
      onSelect: props.onLoadDocument,
    },
    {
      id: "url",
      label: t("tts.add.url"),
      icon: Link,
      onSelect: props.onLoadUrl,
    },
    {
      id: "project-file",
      label: t("tts.add.projectFile"),
      icon: FilePlus2,
      onSelect: props.onAddProjectFile,
    },
  ];

  const tidyLabel = tidying ? t("tts.tidying") : t("tts.tidy");
  const menuItems: ActionMenuItem[] = [
    {
      id: "workshop",
      label: t("tts.workshop.open"),
      icon: BookOpen,
      onSelect: props.onOpenWorkshop,
      testId: "workshop-open",
      title: t("tts.workshop.hint"),
    },
    {
      id: "tidy",
      label: tidyLabel,
      icon: WandSparkles,
      iconClassName: tidying ? "animate-pulse" : undefined,
      onSelect: props.onTidy,
      disabled: tidying || !hasText,
      testId: "tidy-run",
      title: tidying ? t("tts.tidying") : t("tts.tidyHint"),
    },
    {
      // Skript prüfen: jederzeit, nicht erst beim Vorlesen — auch bei
      // abgeschaltetem Automatik-Schalter.
      id: "script-check",
      label: t("tts.scriptCheck.run"),
      icon: SpellCheck,
      iconClassName: errorCount > 0 ? "text-red-500" : undefined,
      onSelect: props.onCheckScript,
      disabled: !hasSpokenText,
      testId: "script-check-run",
      title: t("tts.scriptCheck.runHint"),
      trailing:
        errorCount > 0 ? (
          <span
            data-testid="script-check-badge"
            className="rounded-full bg-red-500/20 px-1.5 text-xs text-red-500"
          >
            {errorCount}
          </span>
        ) : undefined,
    },
    {
      id: "autotag",
      label: t("tts.autotag.button"),
      icon: Tags,
      iconClassName: autoTagLoading ? "animate-pulse" : undefined,
      onSelect: () => setAutoTagOpen(true),
      disabled: autoTagLoading || !hasText,
      testId: "autotag-open",
    },
  ];

  // Speichern und Vorab-Erzeugen gehören in jedem Reiter dazu.
  const saveButton = (
    <IconAction
      icon={Download}
      label={saving ? t("tts.savingAudio") : t("tts.saveAudio")}
      description={t("tts.actions.saveHint")}
      testId="tts-action-save"
      onClick={props.onSave}
      disabled={saving || !hasSpokenText}
    />
  );
  const prewarmButton = (
    <IconAction
      icon={Zap}
      iconClassName={prewarm ? "animate-pulse" : undefined}
      label={prewarm ? t("tts.prewarm.running") : t("tts.prewarm.run")}
      description={t("tts.actions.prewarmHint")}
      testId="tts-action-prewarm"
      wrapperTestId="prewarm-run"
      onClick={props.onPrewarm}
      disabled={prewarm !== null || !hasSpokenText}
    />
  );

  return (
    <div className="flex flex-col gap-2 border-t border-mid-gray/20 pt-3">
      <div
        className="relative flex items-center gap-1.5"
        data-testid="tts-actions"
      >
        {tab === "original" && (
          <>
            <ActionMenu
              trigger={{
                icon: Plus,
                label: t("tts.add.short"),
                description: t("tts.actions.addHint"),
                testId: "tts-action-add",
              }}
              menuLabel={t("tts.add.title")}
              items={addItems}
            />
            <IconAction
              icon={Mic}
              iconClassName={
                dictating ? "text-red-400 animate-pulse" : undefined
              }
              label={dictating ? t("tts.dictateStop") : t("tts.dictate")}
              description={
                dictating
                  ? t("tts.actions.dictateStopHint")
                  : t("tts.actions.dictateHint")
              }
              testId="tts-action-dictate"
              onClick={props.onToggleDictation}
            />
            {saveButton}
            {prewarmButton}
            <span className="flex-1" />
            <ActionMenu
              trigger={{
                icon: Menu,
                label: t("tts.actions.menu"),
                description: t("tts.actions.menuHint"),
                testId: "tts-action-menu",
                // Der Zähler am Menü: Prüfbefunde sollen auffallen, auch
                // wenn das Menü zu ist.
                badge: errorCount > 0 ? errorCount : undefined,
                badgeTestId: "tts-action-menu-badge",
              }}
              menuLabel={t("tts.actions.menu")}
              items={menuItems}
              align="end"
            />
          </>
        )}
        {tab === "translation" && (
          <>
            <IconAction
              icon={Languages}
              label={
                props.translating
                  ? t("tts.translating")
                  : t("tts.translateShort")
              }
              description={t("tts.actions.translateHint")}
              testId="tts-action-translate"
              onClick={props.onTranslate}
              disabled={props.translating || !hasText}
            />
            <TooltipTrigger
              className="tts-actions__lang block min-w-0 flex-1"
              tooltipId="tts-target-lang-tip"
              content={
                <>
                  <div className="text-sm font-semibold">
                    <strong>{t("tts.actions.targetLang")}</strong>
                  </div>
                  <div className="text-xs text-text/70">
                    {t("tts.actions.targetLangHint")}
                  </div>
                </>
              }
            >
              <div
                role="group"
                aria-label={t("tts.actions.targetLang")}
                className="w-full"
              >
                <Select
                  menuPortal
                  value={props.targetLang}
                  options={TTS_TARGET_LANGS}
                  onChange={(value) => value && props.onTargetLangChange(value)}
                  isClearable={false}
                />
              </div>
            </TooltipTrigger>
            {saveButton}
            {prewarmButton}
          </>
        )}
        {tab === "summary" && (
          <>
            <IconAction
              icon={FileText}
              label={
                props.summarizing ? t("tts.summarizing") : t("tts.summarize")
              }
              description={t("tts.actions.summarizeHint")}
              testId="tts-action-summarize"
              onClick={props.onSummarize}
              disabled={props.summarizing || !hasText}
            />
            <ActionPopover
              trigger={{
                icon: SlidersHorizontal,
                label: t("tts.actions.summaryOptions"),
                description: t("tts.actions.summaryOptionsHint"),
                testId: "tts-action-summary-options",
              }}
              popoverLabel={t("tts.actions.summaryOptions")}
              popoverTestId="tts-summary-options"
            >
              <label className="flex flex-col gap-1 text-sm">
                {t("tts.summary.length")}
                <Select
                  menuPortal
                  value={props.sumLength}
                  isClearable={false}
                  options={[
                    { value: "kurz", label: t("tts.summary.lengths.short") },
                    { value: "mittel", label: t("tts.summary.lengths.medium") },
                    { value: "lang", label: t("tts.summary.lengths.long") },
                  ]}
                  onChange={(value) => value && props.onSumLengthChange(value)}
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                {t("tts.summary.detail")}
                <Select
                  menuPortal
                  value={props.sumDetail}
                  isClearable={false}
                  options={[
                    {
                      value: "ueberblick",
                      label: t("tts.summary.details.overview"),
                    },
                    {
                      value: "ausgewogen",
                      label: t("tts.summary.details.balanced"),
                    },
                    {
                      value: "detailliert",
                      label: t("tts.summary.details.deep"),
                    },
                  ]}
                  onChange={(value) => value && props.onSumDetailChange(value)}
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                {t("tts.summary.audience")}
                <Select
                  menuPortal
                  value={props.sumAudience}
                  isClearable={false}
                  options={[
                    {
                      value: "allgemein",
                      label: t("tts.summary.audiences.general"),
                    },
                    {
                      value: "fachpublikum",
                      label: t("tts.summary.audiences.expert"),
                    },
                    {
                      value: "management",
                      label: t("tts.summary.audiences.management"),
                    },
                  ]}
                  onChange={(value) =>
                    value && props.onSumAudienceChange(value)
                  }
                />
              </label>
            </ActionPopover>
            <span className="flex-1" />
            {saveButton}
            {prewarmButton}
          </>
        )}
      </div>

      {saving && (
        <ProgressLine
          tabular
          ratio={
            exportProgress?.total
              ? exportProgress.position / exportProgress.total
              : 0
          }
          text={
            exportProgress?.total
              ? `${t("tts.sentenceProgress", {
                  position: exportProgress.position,
                  total: exportProgress.total,
                })} · ${
                  exportEta === null
                    ? t("tts.exportEtaComputing")
                    : t("tts.exportEta", { time: props.formatEta(exportEta) })
                }`
              : t("tts.savingAudio")
          }
          cancelLabel={t("tts.cancelExport")}
          onCancel={props.onCancelExport}
        />
      )}
      {prewarm && (
        <ProgressLine
          ratio={prewarm.total ? prewarm.done / prewarm.total : 0}
          text={
            prewarm.total
              ? t("tts.prewarm.progress", {
                  done: prewarm.done,
                  total: prewarm.total,
                })
              : t("tts.prewarm.starting")
          }
          cancelLabel={t("tts.prewarm.cancel")}
          onCancel={props.onCancelPrewarm}
        />
      )}
      {speakProgress && (
        <span className="text-xs text-text/60">
          {t("tts.sentenceProgress", {
            position: speakProgress.position,
            total: speakProgress.total,
          })}
        </span>
      )}
      {/* Auto-Tagging: der Dialog öffnet aus dem Menü; die Leiste zeigt nur
          noch Ladezustand, Abbrechen, Vorschlagszähler und Fehler. Sie bleibt
          im Original-Reiter eingehängt, weil die Vorschläge an dessen Text
          und Editor-Chips hängen. */}
      {tab === "original" && (
        <AutoTagBar
          {...props.autoTag}
          showSettings={false}
          hideTrigger
          open={autoTagOpen}
          onOpenChange={setAutoTagOpen}
          onLoadingChange={setAutoTagLoading}
        />
      )}
    </div>
  );
};
