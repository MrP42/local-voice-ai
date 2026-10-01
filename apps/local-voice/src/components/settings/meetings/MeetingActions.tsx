import React from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Copy,
  Download,
  FileText,
  FolderInput,
  Info,
  LayoutTemplate,
  Languages,
  Library,
  Mail,
  Menu,
  MessageSquare,
  Pencil,
  Presentation,
  RefreshCw,
  Sparkles,
  Trash2,
  UserPen,
  Users,
} from "lucide-react";
import { ActionMenu, type ActionMenuItem } from "../../ui/ActionMenu";
import { IconAction } from "../../ui/IconAction";

/** Tastenkürzel für "Umbenennen" (siehe MeetingDetail). */
const RENAME_KEY = "F2";

export interface MeetingActionHandlers {
  onExport: () => void;
  onFollowup: () => void;
  onCopy: () => void;
  onPeople: () => void;
  onChatToggle: () => void;
  onRetranscribe: () => void;
  /** G5: "Übersetzen nach ..." (neue Fassung, das Original bleibt). */
  onTranslate: () => void;
  onRegenNotes: () => void;
  onRegenMinutes: () => void;
  onTemplate: () => void;
  /** G4: "Vorlagen verwalten ..." (Verwaltung der Vorlagen direkt). */
  onManageTemplates: () => void;
  /** G4: "Neu erzeugen mit Vorlage ..." (Vorlage waehlen, Protokoll oder KI-Notizen erzeugen). */
  onRegenWithTemplate: () => void;
  /** U8: "Sprecher benennen ..." (Dialog mit allen Sprechern). */
  onSpeakers: () => void;
  /** D4: "Folien erkennen" (nur bei Besprechungen mit Videodatei, siehe `canDetectSlides`). */
  onDetectSlides?: () => void;
  onRename: () => void;
  onMove: () => void;
  onCopyPlain: () => void;
  onExportTranscript: () => void;
  onDetails: () => void;
  onDelete: () => void;
}

interface MeetingActionsProps extends MeetingActionHandlers {
  hasSegments: boolean;
  /** Gibt es erkannte Sprecher, die man benennen kann? */
  hasSpeakers: boolean;
  chatOpen: boolean;
  /** Transkript wurde gerade kopiert (Haken statt Symbol, kurz). */
  copied: boolean;
  /** Läuft eine Verarbeitung? Dann sind Neu-Transkription und Neu-Erzeugen gesperrt. */
  busy: boolean;
  hasAudio: boolean;
  /** D4: Quelle ist ein Video; nur dann steht "Folien erkennen" im Menü. */
  canDetectSlides?: boolean;
  /** Die Besprechung wird gerade aufgenommen: Löschen ist gesperrt. */
  live?: boolean;
  /**
   * `toolbar`: Symbolzeile der Bedienspalte; `menu`: das Menü "☰" mit den
   * selteneren Aktionen (steht im Kopf neben "Details"); `menu-all`: das Menü
   * trägt zusätzlich die Aktionen der Symbolzeile (schmales Fenster).
   */
  mode?: "toolbar" | "menu" | "menu-all";
}

/**
 * Symbolzeile der Bedienspalte (Exportieren, Follow-up-Mail, Kopieren,
 * Personen, Fragen) und das Menü "☰" mit den selteneren Aktionen. Alle
 * Knöpfe sind `IconAction`: gleich groß, Name und Kurzerklärung im Tooltip (Maus
 * und Tastatur), Name im aria-label.
 */
export const MeetingActions: React.FC<MeetingActionsProps> = ({
  hasSegments,
  hasSpeakers,
  chatOpen,
  copied,
  busy,
  hasAudio,
  canDetectSlides = false,
  live = false,
  mode = "toolbar",
  onExport,
  onFollowup,
  onCopy,
  onPeople,
  onChatToggle,
  onRetranscribe,
  onTranslate,
  onRegenNotes,
  onRegenMinutes,
  onTemplate,
  onManageTemplates,
  onRegenWithTemplate,
  onSpeakers,
  onDetectSlides,
  onRename,
  onMove,
  onCopyPlain,
  onExportTranscript,
  onDetails,
  onDelete,
}) => {
  const { t } = useTranslation();

  const retranscribeTitle = !hasAudio
    ? t("meetings.actions.retranscribeNoAudio")
    : busy
      ? t("meetings.actions.retranscribeBusy")
      : undefined;

  const items: ActionMenuItem[] = [
    {
      id: "retranscribe",
      label: t("meetings.actions.retranscribe"),
      icon: RefreshCw,
      onSelect: onRetranscribe,
      disabled: !hasAudio || busy,
      title: retranscribeTitle,
      testId: "menu-retranscribe",
    },
    {
      id: "translate",
      label: t("meetings.actions.translate"),
      icon: Languages,
      onSelect: onTranslate,
      disabled: busy || !hasSegments,
      title: t("meetings.actions.translateHint"),
      testId: "menu-translate",
    },
    {
      id: "regen-notes",
      label: t("meetings.actions.regenNotes"),
      icon: Sparkles,
      onSelect: onRegenNotes,
      disabled: busy || !hasSegments,
      testId: "menu-regen-notes",
    },
    {
      id: "regen-minutes",
      label: t("meetings.actions.regenMinutes"),
      icon: FileText,
      onSelect: onRegenMinutes,
      disabled: busy || !hasSegments,
      testId: "menu-regen-minutes",
    },
    {
      id: "regen-template",
      label: t("meetings.actions.regenWithTemplate"),
      icon: Sparkles,
      onSelect: onRegenWithTemplate,
      disabled: busy || !hasSegments,
      testId: "menu-regen-template",
    },
    {
      id: "template",
      label: t("meetings.actions.template"),
      icon: LayoutTemplate,
      onSelect: onTemplate,
      testId: "menu-template",
    },
    {
      id: "template-manage",
      label: t("meetings.templates.manage"),
      icon: Library,
      onSelect: onManageTemplates,
      testId: "menu-template-manage",
    },
    {
      id: "speakers",
      label: t("meetings.actions.speakers"),
      icon: UserPen,
      onSelect: onSpeakers,
      disabled: !hasSpeakers,
      title: hasSpeakers ? undefined : t("meetings.actions.speakersNone"),
      testId: "menu-speakers",
    },
    ...(canDetectSlides && onDetectSlides
      ? [
          {
            id: "detect-slides",
            label: t("meetings.slides.detect"),
            icon: Presentation,
            onSelect: onDetectSlides,
            disabled: busy || live,
            title: busy
              ? t("meetings.slides.detectBusy")
              : t("meetings.slides.detectHint"),
            testId: "menu-detect-slides",
          } satisfies ActionMenuItem,
        ]
      : []),
    {
      id: "rename",
      label: t("meetings.actions.rename"),
      icon: Pencil,
      onSelect: onRename,
      trailing: <kbd className="text-xs text-text/50">{RENAME_KEY}</kbd>,
      testId: "menu-rename",
    },
    {
      id: "move",
      label: t("meetings.actions.move"),
      icon: FolderInput,
      onSelect: onMove,
      testId: "menu-move",
    },
    {
      id: "copy-plain",
      label: t("meetings.actions.copyPlain"),
      icon: Copy,
      onSelect: onCopyPlain,
      disabled: !hasSegments,
      title: t("meetings.detail.copyPlainHint"),
      testId: "menu-copy-plain",
    },
    {
      id: "export-transcript",
      label: t("meetings.actions.exportTranscript"),
      icon: Download,
      onSelect: onExportTranscript,
      disabled: !hasSegments,
      title: t("meetings.detail.exportTranscript"),
      testId: "menu-export-transcript",
    },
    {
      id: "details",
      label: t("meetings.actions.details"),
      icon: Info,
      onSelect: onDetails,
      testId: "menu-details",
    },
    {
      id: "delete",
      label: t("meetings.actions.delete"),
      icon: Trash2,
      iconClassName: "text-red-500",
      onSelect: onDelete,
      disabled: live,
      title: live ? t("meetings.actions.deleteLive") : undefined,
      testId: "menu-delete",
    },
  ];

  // Schmales Fenster: die Symbolzeile fällt weg, ihre Aktionen stehen vorn im Menü.
  const primaryItems: ActionMenuItem[] = [
    {
      id: "export",
      label: t("meetings.export.button"),
      icon: Download,
      onSelect: onExport,
      testId: "menu-export",
    },
    {
      id: "followup",
      label: t("meetings.followup.button"),
      icon: Mail,
      onSelect: onFollowup,
      disabled: !hasSegments,
      testId: "menu-followup",
    },
    {
      id: "copy",
      label: t("meetings.actions.copyName"),
      icon: Copy,
      onSelect: onCopy,
      disabled: !hasSegments,
      testId: "menu-copy",
    },
    {
      id: "people",
      label: t("meetings.actions.peopleName"),
      icon: Users,
      onSelect: onPeople,
      testId: "menu-people",
    },
    {
      id: "chat",
      label: t("meetings.chat.ask"),
      icon: MessageSquare,
      onSelect: onChatToggle,
      testId: "menu-chat",
    },
  ];

  if (mode !== "toolbar") {
    return (
      <ActionMenu
        trigger={{
          icon: Menu,
          label: t("meetings.actions.menuName"),
          description: t("meetings.actions.menuHint"),
          testId: "meeting-menu",
        }}
        menuLabel={t("meetings.actions.menuLabel")}
        align="end"
        widthClass="w-72"
        items={mode === "menu-all" ? [...primaryItems, ...items] : items}
      />
    );
  }

  return (
    <div
      className="flex items-center gap-2"
      role="toolbar"
      aria-label={t("meetings.actions.group")}
      data-testid="rec-actions"
    >
      <IconAction
        icon={Download}
        label={t("meetings.export.button")}
        description={t("meetings.export.buttonTitle")}
        testId="export-open"
        onClick={onExport}
      />
      <IconAction
        icon={Mail}
        label={t("meetings.followup.button")}
        description={t("meetings.followup.buttonTitle")}
        testId="followup-open"
        disabled={!hasSegments}
        onClick={onFollowup}
      />
      <IconAction
        icon={copied ? Check : Copy}
        label={
          copied ? t("meetings.actions.copied") : t("meetings.actions.copyName")
        }
        description={t("meetings.actions.copyHint")}
        testId="copy-transcript"
        disabled={!hasSegments}
        onClick={onCopy}
      />
      <IconAction
        icon={Users}
        label={t("meetings.actions.peopleName")}
        description={t("meetings.actions.peopleHint")}
        testId="people-open"
        onClick={onPeople}
      />
      <IconAction
        icon={MessageSquare}
        label={t("meetings.chat.ask")}
        description={t("meetings.chat.askTitle")}
        testId="chat-toggle"
        aria-pressed={chatOpen}
        aria-keyshortcuts="Control+J"
        className={chatOpen ? "border-logo-primary bg-logo-primary/20" : ""}
        onClick={onChatToggle}
      />
    </div>
  );
};
