import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, EyeOff, Maximize2 } from "lucide-react";
import type { MeetingSlide } from "@/bindings";
import { formatClock } from "@/lib/meetingJobs";
import { slideStartMs } from "@/lib/meetingSlides";

interface SlidesPanelProps {
  /** Alle Folien der Besprechung, auch ausgeblendete. */
  slides: MeetingSlide[];
  /** Bild-URL zu einem relativen Pfad der Folie (`null`: noch kein Ordner). */
  imageUrl: (relative: string | null) => string | null;
  /** Die Folie, die zur Abspielzeit zu sehen ist. */
  activeId: string | null;
  /** Gibt es einen Player, in den ein Klick springen kann? */
  canSeek: boolean;
  onSeek: (ms: number) => void;
  onHide: (slideId: string, hidden: boolean) => void;
  onOpen: (slideId: string) => void;
}

/** Eine kleine Aktionsflaeche in der Fusszeile einer Folienkarte. */
const CardButton: React.FC<
  React.ButtonHTMLAttributes<HTMLButtonElement> & { label: string }
> = ({ label, children, ...props }) => (
  <button
    type="button"
    title={label}
    aria-label={label}
    className="inline-flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-text/60 hover:bg-mid-gray/15 hover:text-logo-primary focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary"
    {...props}
  >
    {children}
  </button>
);

/**
 * Reiter "Folien" in der Mitte der Besprechung (D4): die Folien als Vorschaubilder.
 * Ein Klick springt im Audio an die erste Sichtung; waehrend der Wiedergabe ist die
 * aktuelle Folie hervorgehoben und scrollt ins Bild. Ausblenden (Sprecherbild,
 * Dublette) nimmt eine Folie aus Liste, Marken und Auswahl, ohne sie zu loeschen.
 */
export const SlidesPanel: React.FC<SlidesPanelProps> = ({
  slides,
  imageUrl,
  activeId,
  canSeek,
  onSeek,
  onHide,
  onOpen,
}) => {
  const { t } = useTranslation();
  const [showHidden, setShowHidden] = useState(false);
  const hiddenCount = slides.filter((s) => s.hidden).length;
  const shown = slides.filter((s) => showHidden || !s.hidden);
  const listRef = useRef<HTMLDivElement>(null);

  // Die aktuelle Folie folgt der Abspielzeit: in den sichtbaren Bereich holen.
  useEffect(() => {
    if (!activeId) return;
    const el = listRef.current?.querySelector<HTMLElement>(
      `[data-slide-id="${CSS.escape(activeId)}"]`,
    );
    el?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
  }, [activeId]);

  return (
    <div
      data-testid="slides-panel"
      className="flex min-h-0 flex-1 flex-col gap-2"
    >
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <p className="text-xs text-text/60" data-testid="slides-summary">
          {t("meetings.slides.summary", { count: slides.length - hiddenCount })}
        </p>
        {hiddenCount > 0 && (
          <label className="flex cursor-pointer items-center gap-2 text-xs text-text/80">
            <input
              type="checkbox"
              data-testid="slides-show-hidden"
              checked={showHidden}
              onChange={(e) => setShowHidden(e.target.checked)}
            />
            {t("meetings.slides.showHidden", { count: hiddenCount })}
          </label>
        )}
      </div>
      {shown.length === 0 ? (
        <p className="text-sm text-text/60" data-testid="slides-all-hidden">
          {t("meetings.slides.allHidden")}
        </p>
      ) : (
        <div
          ref={listRef}
          data-testid="slides-list"
          className="grid min-h-0 flex-1 content-start gap-2 overflow-y-auto pe-1"
          style={{
            gridTemplateColumns: "repeat(auto-fill, minmax(9.5rem, 1fr))",
          }}
        >
          {shown.map((slide) => {
            const url = imageUrl(slide.thumb_path ?? slide.image_path);
            const start = slideStartMs(slide);
            const active = slide.id === activeId && !slide.hidden;
            const open = () => (canSeek ? onSeek(start) : onOpen(slide.id));
            return (
              <div
                key={slide.id}
                data-testid="slide-card"
                data-slide-id={slide.id}
                data-slide-number={slide.number}
                data-active={active ? "true" : undefined}
                data-hidden={slide.hidden ? "true" : undefined}
                className={`flex flex-col overflow-hidden rounded-md border bg-background transition-colors ${
                  active
                    ? "border-logo-primary ring-2 ring-logo-primary/60"
                    : "border-mid-gray/25"
                } ${slide.hidden ? "opacity-55" : ""}`}
              >
                <button
                  type="button"
                  data-testid="slide-thumb"
                  onClick={open}
                  title={
                    canSeek
                      ? t("meetings.slides.playFrom", {
                          time: formatClock(start),
                        })
                      : t("meetings.slides.open")
                  }
                  className="relative block aspect-video w-full cursor-pointer bg-mid-gray/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
                >
                  {url && (
                    <img
                      src={url}
                      alt={t("meetings.slides.alt", { number: slide.number })}
                      loading="lazy"
                      draggable={false}
                      className="h-full w-full object-contain"
                    />
                  )}
                </button>
                <div className="flex items-center gap-1 px-1.5 py-1">
                  <span className="min-w-0 flex-1 truncate text-xs tabular-nums text-text/80">
                    {t("meetings.slides.numberTime", {
                      number: slide.number,
                      time: formatClock(start),
                    })}
                  </span>
                  <CardButton
                    label={t("meetings.slides.enlarge")}
                    data-testid="slide-open"
                    onClick={() => onOpen(slide.id)}
                  >
                    <Maximize2 width={14} height={14} aria-hidden="true" />
                  </CardButton>
                  <CardButton
                    label={
                      slide.hidden
                        ? t("meetings.slides.unhide")
                        : t("meetings.slides.hide")
                    }
                    data-testid={slide.hidden ? "slide-unhide" : "slide-hide"}
                    onClick={() => onHide(slide.id, !slide.hidden)}
                  >
                    {slide.hidden ? (
                      <Eye width={14} height={14} aria-hidden="true" />
                    ) : (
                      <EyeOff width={14} height={14} aria-hidden="true" />
                    )}
                  </CardButton>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
