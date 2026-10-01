import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, Eye, EyeOff, Play } from "lucide-react";
import type { MeetingSlide } from "@/bindings";
import { formatClock } from "@/lib/meetingJobs";
import { slideStartMs } from "@/lib/meetingSlides";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";

interface SlideViewerProps {
  /** Die Folien, durch die die Pfeile blaettern (nach Nummer). */
  slides: MeetingSlide[];
  /** Wie viele Folien die Besprechung hat (auch ausgeblendete): "Folie 3 von 6". */
  total: number;
  /** ID der gezeigten Folie; `null` = geschlossen. */
  slideId: string | null;
  imageUrl: (relative: string | null) => string | null;
  canSeek: boolean;
  onClose: () => void;
  onNavigate: (slideId: string) => void;
  onSeek: (ms: number) => void;
  onHide: (slideId: string, hidden: boolean) => void;
}

/**
 * Grossansicht einer Folie (D4) im Dialog: Pfeiltasten und Knoepfe blaettern, "Ab
 * hier abspielen" springt im Audio an die erste Sichtung, Ausblenden gilt auch hier.
 * Der Text der Folie (Texterkennung, D2) steht darunter, sobald es ihn gibt.
 */
export const SlideViewer: React.FC<SlideViewerProps> = ({
  slides,
  total,
  slideId,
  imageUrl,
  canSeek,
  onClose,
  onNavigate,
  onSeek,
  onHide,
}) => {
  const { t } = useTranslation();
  const index = slideId ? slides.findIndex((s) => s.id === slideId) : -1;
  const slide = index >= 0 ? slides[index] : null;
  const previous = index > 0 ? slides[index - 1] : null;
  const next =
    index >= 0 && index < slides.length - 1 ? slides[index + 1] : null;

  useEffect(() => {
    if (!slide) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      if (e.key === "ArrowLeft" && previous) {
        e.preventDefault();
        onNavigate(previous.id);
      } else if (e.key === "ArrowRight" && next) {
        e.preventDefault();
        onNavigate(next.id);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [slide, previous, next, onNavigate]);

  if (!slide) return null;
  const url = imageUrl(slide.image_path);
  const start = slideStartMs(slide);
  const ranges = slide.occurrences
    .map((o) => `${formatClock(o.start_ms)}–${formatClock(o.end_ms)}`)
    .join(", ");

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t("meetings.slides.viewerTitle", {
        number: slide.number,
        total,
      })}
      closeLabel={t("meetings.slides.close")}
      className="max-w-4xl"
      contentFades={false}
      footer={
        <>
          <Button
            variant="secondary"
            size="sm"
            data-testid="slide-prev"
            disabled={!previous}
            onClick={() => previous && onNavigate(previous.id)}
          >
            <ChevronLeft width={14} height={14} aria-hidden="true" />
            {t("meetings.slides.previous")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            data-testid="slide-next"
            disabled={!next}
            onClick={() => next && onNavigate(next.id)}
          >
            {t("meetings.slides.next")}
            <ChevronRight width={14} height={14} aria-hidden="true" />
          </Button>
          <Button
            variant="secondary"
            size="sm"
            data-testid="slide-viewer-hide"
            onClick={() => onHide(slide.id, !slide.hidden)}
          >
            {slide.hidden ? (
              <Eye width={14} height={14} aria-hidden="true" />
            ) : (
              <EyeOff width={14} height={14} aria-hidden="true" />
            )}
            {slide.hidden
              ? t("meetings.slides.unhide")
              : t("meetings.slides.hide")}
          </Button>
          {canSeek && (
            <Button
              size="sm"
              data-testid="slide-viewer-play"
              onClick={() => {
                onSeek(start);
                onClose();
              }}
            >
              <Play width={14} height={14} aria-hidden="true" />
              {t("meetings.slides.playFrom", { time: formatClock(start) })}
            </Button>
          )}
        </>
      }
    >
      <div data-testid="slide-viewer" data-slide-number={slide.number}>
        <div className="flex items-center justify-center rounded-md bg-mid-gray/10">
          {url && (
            <img
              src={url}
              alt={t("meetings.slides.alt", { number: slide.number })}
              data-testid="slide-viewer-image"
              className="max-h-[60vh] w-full object-contain"
            />
          )}
        </div>
        <p className="mt-2 text-xs tabular-nums text-text/60">
          {t("meetings.slides.visibleAt", { ranges })}
          {slide.hidden ? ` · ${t("meetings.slides.hiddenNote")}` : ""}
        </p>
        {slide.ocr_text && (
          <details className="mt-2 text-sm" data-testid="slide-viewer-text">
            <summary className="cursor-pointer text-xs text-text/70">
              {t("meetings.slides.text")}
            </summary>
            <p className="mt-1 whitespace-pre-wrap text-text/90">
              {slide.ocr_text}
            </p>
          </details>
        )}
      </div>
    </Dialog>
  );
};
