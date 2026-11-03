import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  BookOpen,
  Download,
  FilePlus2,
  Languages,
  Link,
  Menu,
  Mic,
  Plus,
  Share2,
  Sparkles,
  SpellCheck,
  Tags,
  Upload,
  Users,
  WandSparkles,
  Zap,
  AudioLines,
} from "lucide-react";
import { ActionMenu } from "../../ui/ActionMenu";
import type { ActionMenuItem } from "../../ui/ActionMenu";
import { IconAction } from "../../ui/IconAction";
import { TooltipTrigger } from "../../ui/TooltipTrigger";
import { Glyph } from "../../ui/AudioPlayer";
import { Select } from "../../ui/Select";
import { TTS_TARGET_LANGS } from "../../../lib/constants/languages";
import { AutoTagBar, type AutoTagBarProps } from "./tags/AutoTagBar";
import { counterWidthCh } from "./sentenceCounter";
import { Button } from "../../ui/Button";
import {
  describeTranslateError,
  progressParts,
  progressRatio,
  sectionChars,
  workerStatus,
  type TransformProgress,
} from "../../../lib/reader/translationStream";

export type TtsTab = "original" | "translation" | "summary";

interface Progress {
  position: number;
  total: number;
}

export interface ReadAloudActionsProps {
  /** Einzeilig in der Spielerleiste (rechte Spalte ausgeblendet): kein
   *  Trennstrich, kein Satzzähler (steht in der Leiste), Fortschritt daneben. */
  compact?: boolean;
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

  /** Zaehler "Satz n/m": `position` = der Satz, der zu hoeren ist (1-basiert). */
  sentenceCounter: Progress | null;

  onOpenWorkshop: () => void;
  /** lva#236: Fassungen exportieren (EPUB, PDF, Word, Audio …). */
  onExport?: () => void;
  tidying: boolean;
  onTidy: () => void;
  /** Sprechfassung läuft (KI setzt Satzzeichen, Satzbau, Zahlen fürs Vorlesen). */
  speeching: boolean;
  onSpeech: () => void;
  errorCount: number;
  onCheckScript: () => void;
  /** Warum Auto-Tagging gerade nichts bringt (Engine ohne Stil-Tags, z. B. Phonon);
   *  der Eintrag ist dann ausgegraut und nennt den Grund. */
  autoTagBlockedReason?: string;
  /** Props der Auto-Tagging-Leiste ohne Dialogsteuerung — die liegt hier. */
  autoTag: Omit<
    AutoTagBarProps,
    "open" | "onOpenChange" | "hideTrigger" | "onLoadingChange" | "showSettings"
  >;

  targetLang: string;
  onTargetLangChange: (value: string) => void;
  translating: boolean;
  onTranslate: () => void;

  /** Ein KI-Dokument wird erstellt (lva#183); es läuft nur eines. */
  summarizing: boolean;
  /** Öffnet „Neues KI-Dokument“. */
  onSummarize: () => void;
  /** Fortschritt von Übersetzung/Zusammenfassung (Block x von y). */
  transformProgress: TransformProgress | null;
  /** Übersetzung angehalten (Fehler/Abbruch), Teilergebnis steht. */
  translatePartial?: { kind: "error" | "cancelled"; message: string } | null;
  /** Die Wiedergabe wartet auf neuen Text der laufenden Übersetzung. */
  waitingForTranslation?: boolean;
  /** Hinweise zu Abschnitten, deren Marken nicht stimmen. */
  translateWarnings?: string[];
  /** „Fortsetzen“ nach Fehler/Abbruch. */
  onResumeTranslate?: () => void;
  onCancelTransform: (kind: "translation" | "summary") => void;
  /** Sprecher der Übersetzung gegen Stimmen der Zielsprache tauschen. */
  onSwapSpeakers: () => void;
}

/** Fortschrittsbalken mit Text und Abbrechen — die Zeilen unter den Knöpfen. */
const ProgressLine: React.FC<{
  ratio: number;
  text: string;
  cancelLabel: string;
  onCancel: () => void;
  tabular?: boolean;
}> = ({ ratio, text, cancelLabel, onCancel, tabular }) => (
  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
    <div
      className="h-1.5 w-full shrink-0 overflow-hidden rounded-full bg-mid-gray/20 sm:w-32"
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(ratio * 100)}
    >
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
      className="mbtn mbtn--sm min-h-[44px] min-w-[44px] sm:min-h-0 sm:min-w-0"
      onClick={onCancel}
      aria-label={cancelLabel}
    >
      <Glyph name="stop" />
    </button>
  </div>
);

/** Balken mit Textwert; Beschriftung und Wert stehen als Text, nicht nur als Farbe. */
const Bar: React.FC<{
  ratio: number;
  ariaLabel: string;
  valueText: string;
  className?: string;
  thin?: boolean;
}> = ({ ratio, ariaLabel, valueText, className = "", thin }) => (
  <div
    role="progressbar"
    aria-label={ariaLabel}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(ratio * 100)}
    aria-valuetext={valueText}
    className={`${thin ? "h-1.5" : "h-2"} min-w-0 overflow-hidden rounded-full bg-mid-gray/30 ${className}`.trim()}
  >
    <div
      className="h-full rounded-full bg-logo-primary transition-[width] duration-200"
      style={{ width: `${Math.min(1, Math.max(0, ratio)) * 100}%` }}
    />
  </div>
);

/** Fortschritt der Buchübersetzung: Gesamt, Abschnitte/Kapitel, Worker. */
const TranslateProgress: React.FC<{
  progress: TransformProgress | null;
  waiting: boolean;
  onCancel: () => void;
}> = ({ progress, waiting, onCancel }) => {
  const { t, i18n } = useTranslation();
  const started = !!progress && progress.total > 1;
  const ratio = progress ? progressRatio(progress) : 0;
  const parts = progress ? progressParts(progress) : null;
  const percent = Math.round(ratio * 100);
  const chars = progress ? sectionChars(progress) : null;
  const workers = progress?.workers ?? [];
  const totalValue = t("tts.transform.totalValue", { percent });
  return (
    <div
      className="flex flex-col gap-2 rounded-lg border border-mid-gray/30 p-3"
      data-testid="tts-translate-progress"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-semibold">
          {t("tts.transform.running")}
        </span>
        <button
          type="button"
          className="mbtn mbtn--sm min-h-[44px] min-w-[44px] sm:min-h-0 sm:min-w-0"
          onClick={onCancel}
          aria-label={t("tts.transform.cancel")}
          title={t("tts.transform.cancel")}
        >
          <Glyph name="stop" />
        </button>
      </div>
      {!started ? (
        <p className="text-xs text-text/70">
          {waiting
            ? t("tts.transform.waiting")
            : t("tts.transform.translateStarting")}
        </p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="shrink-0 text-xs font-medium">
              {t("tts.transform.total")}
            </span>
            <Bar
              ratio={ratio}
              ariaLabel={t("tts.transform.totalAria")}
              valueText={totalValue}
              className="min-w-[5rem] flex-1"
            />
            <span className="shrink-0 text-xs tabular-nums text-text/80">
              {totalValue}
            </span>
          </div>
          <p className="text-xs tabular-nums text-text/80">
            {parts?.chapter != null
              ? t("tts.transform.sectionsDoneChapter", {
                  done: Math.min(progress!.done, progress!.total),
                  total: progress!.total,
                  chapter: parts.chapter,
                  chapters: parts.chapters,
                })
              : t("tts.transform.sectionsDone", {
                  done: Math.min(progress!.done, progress!.total),
                  total: progress!.total,
                })}
          </p>
          <p className="text-[11px] leading-snug text-text/60">
            {chars
              ? t("tts.transform.sizeNote", {
                  chars: chars.toLocaleString(i18n.language),
                })
              : t("tts.transform.sizeNoteGeneric")}
          </p>
          {parts?.title ? (
            <p
              className="line-clamp-2 break-words text-xs text-text/80"
              title={progress!.chapterTitle ?? parts.title}
              data-testid="tts-translate-chapter"
            >
              <span className="text-text/60">
                {t("tts.transform.currentChapter")}
              </span>{" "}
              {(progress!.chapterTitle ?? parts.title)
                .replace(/\s+/g, " ")
                .trim()}
            </p>
          ) : null}
          {workers.length > 1 ? (
            <div
              className="flex flex-col gap-1.5"
              data-testid="tts-worker-list"
            >
              <p className="text-xs font-medium">
                {t("tts.transform.workersTitle")}
              </p>
              <ul className="flex flex-col gap-1.5">
                {workers.map((w, i) => {
                  const status = workerStatus(w, progress!);
                  const value = t("tts.transform.workerValue", {
                    done: w.done,
                    total: w.total,
                  });
                  const statusText = t(
                    status === "done"
                      ? "tts.transform.workerDone"
                      : status === "running"
                        ? "tts.transform.workerRunning"
                        : "tts.transform.workerWaiting",
                  );
                  return (
                    <li
                      key={i}
                      className="flex flex-col gap-0.5"
                      data-testid="tts-worker-row"
                    >
                      <div className="flex flex-wrap items-baseline justify-between gap-x-2 text-xs tabular-nums">
                        <span className="font-medium">
                          {t("tts.transform.worker", { n: i + 1 })}
                        </span>
                        <span className="text-text/80">
                          {value} · {statusText}
                        </span>
                      </div>
                      <Bar
                        thin
                        ratio={w.total > 0 ? w.done / w.total : 0}
                        ariaLabel={t("tts.transform.workerAria", {
                          n: i + 1,
                          done: w.done,
                          total: w.total,
                        })}
                        valueText={`${value} · ${statusText}`}
                      />
                    </li>
                  );
                })}
              </ul>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
};

/** Fehler- oder Abbruchzustand der Übersetzung: eigener Block samt „Fortsetzen“. */
const TranslateInterrupted: React.FC<{
  partial: { kind: "error" | "cancelled"; message: string };
  onResume?: () => void;
}> = ({ partial, onResume }) => {
  const { t } = useTranslation();
  const cancelled = partial.kind === "cancelled";
  const info = cancelled ? null : describeTranslateError(partial.message);
  const where =
    info && info.chapter != null
      ? info.section != null
        ? info.sections != null
          ? t("tts.transform.errorWhere", {
              chapter: info.chapter,
              section: info.section,
              sections: info.sections,
            })
          : t("tts.transform.errorWhereSection", {
              chapter: info.chapter,
              section: info.section,
            })
        : t("tts.transform.errorWhereChapter", { chapter: info.chapter })
      : null;
  return (
    <div
      role="status"
      data-testid="tts-translate-partial"
      className="flex w-full flex-col gap-2 rounded-lg border border-mid-gray/40 bg-mid-gray/10 p-3 text-sm"
    >
      <p className="font-semibold">
        {cancelled
          ? t("tts.transform.cancelledTitle")
          : t("tts.transform.errorTitle")}
      </p>
      {info ? (
        <div className="flex flex-col gap-1" data-testid="tts-translate-error">
          <p className="break-words">{t(`tts.transform.cause.${info.kind}`)}</p>
          {where ? (
            <p
              className="text-xs text-text/80"
              data-testid="tts-translate-where"
            >
              {where.charAt(0).toUpperCase() + where.slice(1)}
            </p>
          ) : null}
          {info.detail ? (
            <details className="text-xs text-text/70">
              <summary className="min-h-[44px] cursor-pointer py-3 sm:min-h-0 sm:py-0">
                {t("tts.transform.errorDetails")}
              </summary>
              <p className="mt-1 break-words rounded bg-mid-gray/15 p-2 font-mono">
                {partial.message}
              </p>
            </details>
          ) : null}
        </div>
      ) : null}
      <p className="text-xs text-text/80">{t("tts.transform.errorKept")}</p>
      <Button
        type="button"
        variant="primary"
        size="lg"
        className="min-h-[44px] max-sm:w-full sm:self-start"
        data-testid="tts-translate-resume"
        onClick={onResume}
      >
        {t("tts.transform.resume")}
      </Button>
    </div>
  );
};

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
    sentenceCounter,
    tidying,
    speeching,
    errorCount,
  } = props;
  const [autoTagOpen, setAutoTagOpen] = useState(false);
  // In der Spielerleiste (compact) stehen Fortschritt, Warnungen und
  // Fortsetzen nicht in der einzeiligen Zeile (sie sprengten die Breite und
  // schoben die Leseansicht nach links), sondern als Fenster ÜBER der ganzen
  // Leiste, rechtsbündig (Patrick 02.11.).
  const rootRef = useRef<HTMLDivElement>(null);
  const [player, setPlayer] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setPlayer(
      props.compact
        ? (rootRef.current?.closest<HTMLElement>(
            '[data-testid="reader-player"]',
          ) ?? null)
        : null,
    );
  }, [props.compact]);
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
  const exportItem: ActionMenuItem[] = props.onExport
    ? [
        {
          id: "export",
          label: t("library.export.menu"),
          icon: Share2,
          onSelect: props.onExport,
          testId: "tts-export-open",
          title: t("library.export.readerHint"),
        },
      ]
    : [];
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
      disabled: tidying || speeching || !hasText,
      testId: "tidy-run",
      title: tidying ? t("tts.tidying") : t("tts.tidyHint"),
    },
    {
      id: "speech",
      label: speeching ? t("tts.speeching") : t("tts.speech"),
      icon: AudioLines,
      iconClassName: speeching ? "animate-pulse" : undefined,
      onSelect: props.onSpeech,
      disabled: speeching || tidying || !hasText,
      testId: "speech-run",
      title: speeching ? t("tts.speeching") : t("tts.speechHint"),
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
      disabled: autoTagLoading || !hasText || !!props.autoTagBlockedReason,
      testId: "autotag-open",
      title: props.autoTagBlockedReason,
    },
    ...exportItem,
  ];

  // Übersetzung und Zusammenfassung: dasselbe Menü, aber nur, was auf den
  // Text DIESES Reiters wirkt. Aufbereiten und Auto-Tagging arbeiten am
  // Original und bleiben dort.
  const otherTabItems: ActionMenuItem[] = [
    ...menuItems.filter(
      (item) =>
        item.id === "workshop" ||
        item.id === "script-check" ||
        item.id === "export",
    ),
    ...(tab === "translation"
      ? [
          {
            id: "speaker-swap",
            label: t("tts.speakerSwap.open"),
            icon: Users,
            onSelect: props.onSwapSpeakers,
            disabled: !hasSpokenText,
            testId: "speaker-swap-open",
            title: t("tts.speakerSwap.hint"),
          },
        ]
      : []),
  ];
  const otherTabMenu = (
    <ActionMenu
      trigger={{
        icon: Menu,
        label: t("tts.actions.menu"),
        description: t("tts.actions.menuHint"),
        testId: `tts-action-menu-${tab}`,
        badge: errorCount > 0 ? errorCount : undefined,
      }}
      menuLabel={t("tts.actions.menu")}
      items={otherTabItems}
      align="end"
    />
  );
  const running =
    props.transformProgress && props.transformProgress.kind === tab
      ? props.transformProgress
      : null;
  // KI-Dokumente zeigen ihren Fortschritt auf der Karte (lva#183).
  const transformBusy = tab === "translation" && props.translating;

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

  /** Übersetzung: Fortschritt, Warnungen, Unterbrechung. */
  const showWarnings =
    tab === "translation" &&
    !props.translating &&
    (props.translateWarnings?.length ?? 0) > 0;
  const showPartial =
    tab === "translation" && !props.translating && !!props.translatePartial;
  const translateNotes: React.ReactElement[] = [];
  if (transformBusy)
    translateNotes.push(
      <TranslateProgress
        key="progress"
        progress={running}
        waiting={!!props.waitingForTranslation}
        onCancel={() => props.onCancelTransform("translation")}
      />,
    );
  if (showWarnings)
    translateNotes.push(
      <details
        key="warnings"
        className={`text-xs text-text/70 ${props.compact ? "px-3 py-1" : ""}`.trim()}
        data-testid="tts-translate-warnings"
      >
        <summary className="min-h-[44px] cursor-pointer py-3 sm:min-h-0 sm:py-0">
          {t("tts.transform.warnings", {
            count: props.translateWarnings!.length,
          })}
        </summary>
        <ul className="list-disc ps-5">
          {props.translateWarnings!.map((w, i) => (
            <li key={i}>{w}</li>
          ))}
        </ul>
      </details>,
    );
  if (showPartial)
    translateNotes.push(
      <TranslateInterrupted
        key="partial"
        partial={props.translatePartial!}
        onResume={props.onResumeTranslate}
      />,
    );

  return (
    <div
      ref={rootRef}
      className={
        props.compact
          ? "flex min-w-0 items-center gap-2"
          : "flex flex-col gap-2 border-t border-mid-gray/20 pt-3"
      }
    >
      <div
        className="tts-actions relative flex items-center gap-1.5"
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
            {otherTabMenu}
          </>
        )}
        {tab === "summary" && (
          <>
            <IconAction
              icon={Sparkles}
              label={
                props.summarizing
                  ? t("tts.kiDocs.creating")
                  : t("tts.kiDocs.new")
              }
              description={t("tts.kiDocs.newHint")}
              testId="tts-action-summarize"
              onClick={props.onSummarize}
              disabled={props.summarizing || !hasText}
            />
            <span className="flex-1" />
            {saveButton}
            {prewarmButton}
            {otherTabMenu}
          </>
        )}
      </div>

      {props.compact
        ? translateNotes.length > 0 &&
          (player
            ? createPortal(
                <div
                  className="absolute bottom-full end-0 z-30 mb-2 flex max-h-[min(70vh,32rem)] w-[24rem] max-w-[calc(100%-1rem)] flex-col gap-2 overflow-auto rounded-lg bg-background shadow-lg"
                  data-testid="tts-translate-overlay"
                >
                  {translateNotes}
                </div>,
                player,
              )
            : null)
        : translateNotes}
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
      {sentenceCounter && !props.compact && (
        // Feste Breite (tabular-nums + Mindestbreite nach der Stellenzahl der Gesamtzahl):
        // die Zeile springt nicht, wenn aus "Satz 9/12" "Satz 10/12" wird.
        <span
          className="inline-block text-xs tabular-nums text-text/60"
          style={{
            minWidth: `${counterWidthCh(
              sentenceCounter.total,
              (position, total) =>
                t("tts.sentenceProgress", { position, total }),
            )}ch`,
          }}
          data-testid="tts-sentence-counter"
        >
          {t("tts.sentenceProgress", {
            position: sentenceCounter.position,
            total: sentenceCounter.total,
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
