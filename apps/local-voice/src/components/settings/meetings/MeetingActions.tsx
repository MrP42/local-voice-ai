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
  Mail,
  Menu,
  MessageSquare,
  Pencil,
  RefreshCw,
  Sparkles,
  Trash2,
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
  onRegenNotes: () => void;
  onRegenMinutes: () => void;
  onTemplate: () => void;
  onRename: () => void;
  onMove: () => void;
  onCopyPlain: () => void;
  onExportTranscript: () => void;
  onDetails: () => void;
  onDelete: () => void;
}

interface MeetingActionsProps extends MeetingActionHandlers {
  hasSegments: boolean;
  chatOpen: boolean;
  /** Transkript wurde gerade kopiert (Haken statt Symbol, kurz). */
  copied: boolean;
  /** Läuft eine Verarbeitung? Dann sind Neu-Transkription und Neu-Erzeugen gesperrt. */
  busy: boolean;
  hasAudio: boolean;
}

/**
 * Symbolzeile der Bedienspalte (Exportieren, Follow-up-Mail, Kopieren,
 * Personen, Fragen) und rechts das Menü "☰" mit den selteneren Aktionen. Alle
 * Knöpfe sind `IconAction`: gleich groß, Name und Kurzerklärung im Tooltip (Maus
 * und Tastatur), Name im aria-label.
 */
export const MeetingActions: React.FC<MeetingActionsProps> = ({
  hasSegments,
  chatOpen,
  copied,
  busy,
  hasAudio,
  onExport,
  onFollowup,
  onCopy,
  onPeople,
  onChatToggle,
  onRetranscribe,
  onRegenNotes,
  onRegenMinutes,
  onTemplate,
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
      id: "template",
      label: t("meetings.actions.template"),
      icon: LayoutTemplate,
      onSelect: onTemplate,
      testId: "menu-template",
    },
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
      testId: "menu-delete",
    },
  ];

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
      <span className="flex-1" aria-hidden="true" />
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
        items={items}
      />
    </div>
  );
};
