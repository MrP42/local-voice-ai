import React from "react";
import {
  BookOpen,
  Bot,
  CalendarDays,
  Cloud,
  Folder,
  Library,
  Mail,
  Play,
  Webhook,
  type LucideIcon,
} from "lucide-react";

/** Symbol je Art; Kalender-, Katalog- und Detailansicht benutzen dasselbe. */
const ICONS: Record<string, LucideIcon> = {
  youtube: Play,
  ics: CalendarDays,
  calendar_ics: CalendarDays,
  graph: CalendarDays,
  calendar_graph: CalendarDays,
  m365: Cloud,
  smtp: Mail,
  folder: Folder,
  obsidian: BookOpen,
  wissen: Library,
  agent: Bot,
  mcp: Bot,
  webhook: Webhook,
};

export const KindIcon: React.FC<{ kind: string; size?: number }> = ({
  kind,
  size = 22,
}) => {
  const Icon = ICONS[kind] ?? Folder;
  return (
    <span
      className="inline-flex shrink-0 items-center justify-center rounded-lg bg-mid-gray/15 p-2 text-text"
      aria-hidden="true"
    >
      <Icon size={size} />
    </span>
  );
};
