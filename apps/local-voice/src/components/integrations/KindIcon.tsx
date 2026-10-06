import React from "react";
import {
  BookOpen,
  Bot,
  CalendarDays,
  Cloud,
  FileText,
  Folder,
  Library,
  ListChecks,
  Mail,
  MessageSquare,
  Play,
  Plug,
  Table2,
  Users,
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
  // Dienste (Welle 1): Symbol nach Zweck, keine Markenlogos.
  service: Plug,
  slack: MessageSquare,
  teams: MessageSquare,
  discord: MessageSquare,
  notion: FileText,
  confluence: FileText,
  asana: ListChecks,
  clickup: ListChecks,
  jira: ListChecks,
  trello: ListChecks,
  todoist: ListChecks,
  monday: ListChecks,
  linear: ListChecks,
  github: ListChecks,
  hubspot: Users,
  pipedrive: Users,
  airtable: Table2,
  icloud: CalendarDays,
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
