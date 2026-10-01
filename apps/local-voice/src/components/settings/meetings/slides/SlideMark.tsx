import React from "react";
import { useTranslation } from "react-i18next";
import { Presentation } from "lucide-react";
import type { SlideMark as SlideMarkData } from "@/lib/meetingSlides";

/**
 * Folienmarke im Transkript (D4): eine dezente Zeile ueber dem ersten Satz, der
 * nach dem Folienwechsel faellt. Ein Klick oeffnet die Grossansicht.
 */
export const SlideMarkRow: React.FC<{
  mark: SlideMarkData;
  active: boolean;
  onOpen: (slideId: string) => void;
}> = ({ mark, active, onOpen }) => {
  const { t } = useTranslation();
  return (
    <div
      data-testid="slide-mark"
      data-slide-number={mark.number}
      data-active={active ? "true" : undefined}
      className="flex items-center gap-2 py-0.5 text-[11px] text-text/45"
    >
      <button
        type="button"
        onClick={() => onOpen(mark.slideId)}
        title={t("meetings.slides.open")}
        className={`inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-md px-1 hover:text-logo-primary focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary ${
          active ? "text-logo-primary" : ""
        }`}
      >
        <Presentation width={12} height={12} aria-hidden="true" />
        {t("meetings.slides.mark", { number: mark.number })}
      </button>
      <span aria-hidden="true" className="h-px flex-1 bg-mid-gray/20" />
    </div>
  );
};
