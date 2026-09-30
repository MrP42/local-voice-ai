import React from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Copy, MoveRight } from "lucide-react";
import type { MeetingDrag } from "./useMeetingDrag";

/**
 * Die Karte, die beim Ziehen am Zeiger haengt: Titel der Besprechung und ob
 * sie verschoben (Pfeil) oder mit Strg dazugelegt wird (Kopie).
 */
export const DragGhost: React.FC<{ drag: MeetingDrag | null }> = ({ drag }) => {
  const { t } = useTranslation();
  if (!drag) return null;
  const Icon = drag.additive ? Copy : MoveRight;
  return createPortal(
    <div
      aria-hidden="true"
      data-testid="drag-ghost"
      data-additive={drag.additive ? "true" : "false"}
      className="pointer-events-none fixed z-50 flex max-w-[16rem] items-center gap-1.5 rounded-lg border border-logo-primary bg-background px-2 py-1 text-sm shadow-lg"
      style={{ left: drag.x + 12, top: drag.y + 12 }}
    >
      <Icon width={14} height={14} className="shrink-0 text-logo-primary" />
      <span className="truncate">{drag.meeting.title}</span>
      <span className="shrink-0 text-xs text-text/60">
        {drag.additive
          ? t("meetings.projects.dragAdd")
          : t("meetings.projects.dragMove")}
      </span>
    </div>,
    document.body,
  );
};
