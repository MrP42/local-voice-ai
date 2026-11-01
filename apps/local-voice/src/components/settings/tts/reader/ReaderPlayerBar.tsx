import React, { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Moon, MoreHorizontal, SlidersHorizontal } from "lucide-react";
import { Glyph } from "../../../ui/AudioPlayer";
import type { ReaderPageInfo } from "./ReaderView";
import { useSentenceClock, type SentenceClock } from "./useSentenceClock";

export const SLEEP_CHOICES = [0, 15, 30, 45, 60] as const;

interface ReaderPlayerBarProps {
  speaking: boolean;
  /** Abspielen möglich (Text vorhanden). */
  canPlay: boolean;
  /** Satzsprünge möglich (laufende oder pausierte Sitzung). */
  canSeek: boolean;
  onPlayPause(): void;
  onSeek(delta: number): void;
  onStop(): void;
  speed: number;
  speeds: readonly number[];
  speedLabel(value: number): string;
  onSpeed(value: number): void;
  /** Satzfortschritt der Sitzung (0-basiert) — null ohne Sitzung. */
  progress: { position: number; total: number } | null;
  /** Geschätzte Gesamtdauer des Texts in Sekunden (bei Tempo 1). */
  estimateSeconds: number;
  /** Laufender Satz: die Zeit zählt damit sekundengenau weiter. */
  clock?: SentenceClock | null;
  page: ReaderPageInfo | null;
  onJumpToPage(page: number): void;
  /** Klick auf den Fortschrittsbalken: Anteil 0..1. */
  onSeekFraction(fraction: number): void;
  /** Der Schlaf-Timer ist abgelaufen. */
  onSleep(): void;
  /** Play setzt an der gemerkten Stelle fort (lva#148). */
  resumable?: boolean;
  /** Titel des aktuellen Kapitels, falls der Text Überschriften hat. */
  chapter?: string | null;
  /** Seite der gemerkten Stelle (1-basiert), wenn bekannt. */
  resumePage?: number | null;
  /** Gemerkte Stelle verwerfen: Play beginnt wieder vorn. */
  onRestart?(): void;
  /**
   * Rechte Spalte ausgeblendet (Variante A): Stimme und Werkzeuge stehen in
   * der Transportzeile. Ist die Leiste zu schmal, liegen sie hinter einem
   * Knopf (`inPopover`), damit die Leiste nie eine dritte Zeile bekommt.
   */
  dock?: (inPopover: boolean) => React.ReactNode;
}

/** Ab dieser Leistenbreite stehen Stimme und Werkzeuge in der Zeile. */
const DOCK_INLINE_MIN = 880;

const formatTime = (seconds: number): string => {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}`
    : `${m}:${String(r).padStart(2, "0")}`;
};

/**
 * Spielerleiste unter der Buchseite (lva#148, Codex: Fortschritt und Transport
 * bleiben zusammen, getrennt vom Bearbeiten). Ersetzt „Ausdruck & Sprechstil“
 * unter dem Text.
 */
export const ReaderPlayerBar: React.FC<ReaderPlayerBarProps> = ({
  speaking,
  canPlay,
  canSeek,
  onPlayPause,
  onSeek,
  onStop,
  speed,
  speeds,
  speedLabel,
  onSpeed,
  progress,
  estimateSeconds,
  clock = null,
  page,
  onJumpToPage,
  onSeekFraction,
  onSleep,
  resumable = false,
  resumePage = null,
  chapter = null,
  onRestart,
  dock,
}) => {
  const { t } = useTranslation();
  const [speedOpen, setSpeedOpen] = useState(false);
  const [sleepOpen, setSleepOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [dockOpen, setDockOpen] = useState(false);
  const [barWidth, setBarWidth] = useState(Number.POSITIVE_INFINITY);
  // Gelernter Platzbedarf der Werkzeuge in der Zeile: läuft die Ablage über,
  // wandern sie hinter den Knopf, bis die Leiste mindestens so breit ist.
  const [dockNeed, setDockNeed] = useState(DOCK_INLINE_MIN);
  const dockRef = useRef<HTMLDivElement>(null);
  const dockBtnRef = useRef<HTMLButtonElement>(null);
  const dockPanelRef = useRef<HTMLDivElement>(null);
  const [sleepUntil, setSleepUntil] = useState<number | null>(null);
  /** Gewählte Timer-Dauer in Minuten (0 = aus) — für aria-checked. */
  const [sleepChoice, setSleepChoice] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const moreBtnRef = useRef<HTMLButtonElement>(null);
  const moreMenuRef = useRef<HTMLDivElement>(null);
  const speedHeadId = useId();
  const sleepHeadId = useId();
  const resumeHeadId = useId();
  const chooseSleep = (minutes: number) => {
    setSleepChoice(minutes);
    setSleepUntil(minutes === 0 ? null : Date.now() + minutes * 60000);
    setNow(Date.now());
  };
  const [now, setNow] = useState(() => Date.now());
  const [jump, setJump] = useState("");
  const onSleepRef = useRef(onSleep);
  onSleepRef.current = onSleep;

  // Escape schließt die offenen Auswahlen (Tempo, Schlaf-Timer).
  useEffect(() => {
    if (!speedOpen && !sleepOpen && !moreOpen && !dockOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (moreOpen) moreBtnRef.current?.focus();
      if (dockOpen) dockBtnRef.current?.focus();
      setSpeedOpen(false);
      setSleepOpen(false);
      setMoreOpen(false);
      setDockOpen(false);
    };
    // Klick daneben schließt. Kein fixed-Hintergrund: die Leiste ist ein
    // Größen-Container (Container-Query) und damit Bezugsrahmen für fixed.
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (target?.closest?.("[data-reader-popup]")) return;
      // Menüs aus der Werkzeug-Ablage (Stimmenliste, Aktionsmenüs) liegen
      // als Portal außerhalb der Leiste: ein Klick dort schließt sie nicht.
      if (
        !rootRef.current?.contains(target) &&
        target?.closest?.(
          '[role="listbox"], [role="option"], [role="menu"], [role="menuitem"], [id^="react-select"], [class*="__menu"]',
        )
      )
        return;
      setSpeedOpen(false);
      setSleepOpen(false);
      setMoreOpen(false);
      setDockOpen(false);
    };
    // Wechselt die Breite die Bedienform, schließt die nun verborgene Auswahl.
    const ro = new ResizeObserver(() => {
      const root = rootRef.current;
      if (!root) return;
      root
        .querySelectorAll<HTMLElement>("[data-reader-popup]")
        .forEach((el) => {
          if (el.offsetParent !== null) return;
          const kind = el.dataset.readerPopup;
          if (kind === "more") setMoreOpen(false);
          else if (kind === "dock") setDockOpen(false);
          else if (kind === "speed") setSpeedOpen(false);
          else if (kind === "sleep") setSleepOpen(false);
        });
    });
    if (rootRef.current) ro.observe(rootRef.current);
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown, true);
      ro.disconnect();
    };
  }, [speedOpen, sleepOpen, moreOpen, dockOpen]);

  // Breite der Leiste: entscheidet, ob die Werkzeuge in der Zeile stehen.
  useEffect(() => {
    const root = rootRef.current;
    if (!root || !dock) return;
    const measure = () => setBarWidth(root.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(root);
    return () => ro.disconnect();
  }, [dock]);
  const dockInline = barWidth >= dockNeed;
  useLayoutEffect(() => {
    const el = dockRef.current;
    if (!dockInline || !el) return;
    const over = el.scrollWidth - el.clientWidth;
    if (over > 1) setDockNeed(barWidth + over + 16);
  });
  // Geöffnetes Fenster: Fokus hinein (erstes Bedienelement).
  useEffect(() => {
    if (!dockOpen) return;
    dockPanelRef.current
      ?.querySelector<HTMLElement>(
        'input, button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )
      ?.focus();
  }, [dockOpen]);
  useEffect(() => {
    if (dockInline || !dock) setDockOpen(false);
  }, [dockInline, dock]);

  // Geöffnetes Überlaufmenü: Fokus auf den gewählten Eintrag.
  useEffect(() => {
    if (!moreOpen) return;
    const menu = moreMenuRef.current;
    const first =
      menu?.querySelector<HTMLElement>('[aria-checked="true"]') ??
      menu?.querySelector<HTMLElement>('[role="menuitemradio"]');
    first?.focus();
  }, [moreOpen]);

  useEffect(() => {
    if (sleepUntil === null) return;
    const id = window.setInterval(() => {
      const n = Date.now();
      setNow(n);
      if (n >= sleepUntil) {
        setSleepUntil(null);
        setSleepChoice(0);
        onSleepRef.current();
      }
    }, 1000);
    return () => window.clearInterval(id);
  }, [sleepUntil]);

  // Innerhalb des hörbaren Satzes mitlaufen; nach einer Pause beginnt der
  // Satz wieder vorn, die Anzeige steht dann am Satzanfang.
  const within = useSentenceClock(
    clock && progress && clock.index === progress.position ? clock : null,
    4,
  );
  const fraction =
    progress && progress.total > 0
      ? Math.min(1, (progress.position + within) / progress.total)
      : 0;
  const total = estimateSeconds / Math.max(0.25, speed);
  const elapsed = total * fraction;
  const shownPage = page ? (page.currentPage ?? page.page) + 1 : 1;
  const pages = page?.pages ?? 1;

  const submitJump = () => {
    const n = parseInt(jump, 10);
    if (Number.isFinite(n) && n > 0) onJumpToPage(Math.min(pages, n) - 1);
    setJump("");
  };

  const resumeShort =
    resumePage !== null
      ? t("tts.reader.resumeFromPage", { page: resumePage })
      : t("tts.reader.resumeShort");
  const resumeTitle =
    resumePage !== null
      ? t("tts.reader.resumeHintPage", { page: resumePage })
      : t("tts.reader.resumeHint");

  const sleepLeft =
    sleepUntil !== null
      ? Math.max(0, Math.ceil((sleepUntil - now) / 60000))
      : null;

  return (
    <div
      ref={rootRef}
      className={`reader-player${dock ? " reader-player--docked" : ""}`}
      role="region"
      aria-label={t("tts.reader.player")}
      data-testid="reader-player"
    >
      <div className="reader-player__prog">
        <span data-testid="reader-elapsed">{formatTime(elapsed)}</span>
        <div
          className="reader-player__bar"
          role="slider"
          tabIndex={0}
          aria-label={t("tts.reader.progress")}
          aria-valuemin={1}
          aria-valuemax={pages}
          aria-valuenow={shownPage}
          aria-valuetext={t("tts.reader.pageOf", {
            page: shownPage,
            count: pages,
          })}
          data-testid="reader-progress"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onSeekFraction(
              Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)),
            );
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowRight" || e.key === "PageDown")
              onJumpToPage(Math.min(pages - 1, shownPage));
            else if (e.key === "ArrowLeft" || e.key === "PageUp")
              onJumpToPage(Math.max(0, shownPage - 2));
            else if (e.key === "Home") onJumpToPage(0);
            else if (e.key === "End") onJumpToPage(pages - 1);
            else return;
            e.preventDefault();
          }}
        >
          <span className="reader-player__track" />
          <span
            className="reader-player__fill"
            style={{ width: `${fraction * 100}%` }}
          />
          <span
            className="reader-player__knob"
            style={{ left: `${fraction * 100}%` }}
          />
        </div>
        <span data-testid="reader-remaining">
          −{formatTime(total - elapsed)}
        </span>
        <label className="reader-player__jump">
          <span className="reader-player__jump-label">
            {t("tts.reader.page")}
          </span>
          <input
            inputMode="numeric"
            value={jump}
            placeholder={String(shownPage)}
            onChange={(e) => setJump(e.target.value.replace(/\D/g, ""))}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitJump();
            }}
            onBlur={() => jump && submitJump()}
            aria-label={t("tts.reader.jumpToPage")}
            data-testid="reader-jump"
          />
          <span data-testid="reader-page-count">/ {pages}</span>
        </label>
      </div>
      <div className="reader-player__ctrls">
        <span
          className={`reader-player__where${
            !progress && resumable
              ? " reader-player__where--resume"
              : chapter
                ? " reader-player__where--chapter"
                : ""
          }`}
          data-testid="reader-where"
        >
          {progress ? (
            <>
              {chapter && (
                <span
                  className="reader-player__chapter"
                  title={chapter}
                  data-testid="reader-chapter"
                >
                  {chapter}
                </span>
              )}
              <span className="reader-player__sentence">
                {t("tts.reader.sentenceOf", {
                  position: Math.min(progress.total, progress.position + 1),
                  count: progress.total,
                })}
              </span>
            </>
          ) : resumable ? (
            <>
              <span className="reader-player__hint" title={resumeTitle}>
                {resumeShort}
              </span>
              {onRestart && (
                <>
                  <span aria-hidden="true">·</span>
                  <button
                    type="button"
                    className="reader-player__link"
                    onClick={onRestart}
                    data-testid="reader-restart"
                  >
                    {t("tts.reader.restart")}
                  </button>
                </>
              )}
            </>
          ) : (
            // Auch ohne laufende Sitzung: Kapitel über der Seitenangabe.
            <>
              {chapter && (
                <span
                  className="reader-player__chapter"
                  title={chapter}
                  data-testid="reader-chapter"
                >
                  {chapter}
                </span>
              )}
              <span className="reader-player__sentence">
                {t("tts.reader.pageOf", { page: shownPage, count: pages })}
              </span>
            </>
          )}
        </span>
        {dock &&
          (dockInline ? (
            <div ref={dockRef} className="reader-dock" data-testid="reader-dock">
              {dock(false)}
            </div>
          ) : (
            <div className="relative flex-none" data-reader-popup="dock">
              <button
                type="button"
                ref={dockBtnRef}
                className="reader-player__pill reader-player__pill--icon"
                onClick={() => setDockOpen((o) => !o)}
                aria-haspopup="dialog"
                aria-expanded={dockOpen}
                aria-label={t("tts.reader.dockTools")}
                title={t("tts.reader.dockTools")}
                data-testid="reader-dock-open"
              >
                <SlidersHorizontal width={16} height={16} aria-hidden="true" />
              </button>
              {/* Bleibt eingehängt (nur verborgen): laufende Vorgänge der
                  Werkzeuge (Auto-Tagging, Export) behalten ihren Zustand. */}
              <div
                ref={dockPanelRef}
                role="dialog"
                aria-label={t("tts.reader.dockTools")}
                hidden={!dockOpen}
                className="absolute left-0 bottom-full z-40 mb-1 w-80 max-w-[calc(100vw-2rem)] space-y-2 rounded-lg border border-mid-gray/40 bg-background p-3 shadow-lg"
                data-testid="reader-dock-panel"
              >
                {dock(true)}
              </div>
            </div>
          ))}
        <div className="mediabar">
          <button
            type="button"
            className="mbtn"
            onClick={() => onSeek(-1)}
            disabled={!canSeek}
            aria-label={t("tts.previousSentence")}
            title={t("tts.previousSentence")}
          >
            <Glyph name="prev" />
          </button>
          <button
            type="button"
            className="mbtn mbtn--primary mbtn--lg"
            onClick={onPlayPause}
            disabled={!speaking && !canPlay}
            aria-label={
              speaking
                ? t("tts.pause")
                : resumable
                  ? t("tts.reader.resume")
                  : t("tts.speak")
            }
            title={resumable && !speaking ? resumeTitle : undefined}
            data-testid="reader-play"
          >
            <Glyph name={speaking ? "pause" : "play"} />
          </button>
          <button
            type="button"
            className="mbtn"
            onClick={() => onSeek(1)}
            disabled={!canSeek}
            aria-label={t("tts.nextSentence")}
            title={t("tts.nextSentence")}
          >
            <Glyph name="next" />
          </button>
          <span className="mediabar__sep" />
          <button
            type="button"
            className="mbtn"
            onClick={onStop}
            aria-label={t("tts.stop")}
            title={t("tts.stop")}
          >
            <Glyph name="stop" />
          </button>
        </div>
        <div className="reader-player__more relative" data-reader-popup="more">
          <button
            type="button"
            ref={moreBtnRef}
            className="reader-player__pill reader-player__pill--icon"
            onClick={() => setMoreOpen((o) => !o)}
            aria-haspopup="menu"
            aria-expanded={moreOpen}
            aria-label={t("tts.reader.moreOptions")}
            title={t("tts.reader.moreOptions")}
            data-testid="reader-more"
          >
            {sleepLeft !== null && (
              <Moon width={14} height={14} aria-hidden="true" />
            )}
            <span className="tabular-nums">{speedLabel(speed)}</span>
            <MoreHorizontal width={16} height={16} aria-hidden="true" />
          </button>
          {moreOpen && (
            <>
              <div
                ref={moreMenuRef}
                role="menu"
                aria-label={t("tts.reader.moreOptions")}
                onKeyDown={(e) => {
                  if (
                    e.key !== "ArrowDown" &&
                    e.key !== "ArrowUp" &&
                    e.key !== "Home" &&
                    e.key !== "End"
                  )
                    return;
                  e.preventDefault();
                  const items = Array.from(
                    e.currentTarget.querySelectorAll<HTMLElement>(
                      '[role="menuitemradio"], [role="menuitem"]',
                    ),
                  );
                  const i = items.indexOf(
                    document.activeElement as HTMLElement,
                  );
                  const next =
                    e.key === "Home"
                      ? 0
                      : e.key === "End"
                        ? items.length - 1
                        : (i +
                            (e.key === "ArrowDown" ? 1 : -1) +
                            items.length) %
                          items.length;
                  items[next]?.focus();
                }}
                className="absolute right-0 bottom-full mb-1 w-64 rounded-lg border border-mid-gray/40 bg-background shadow-lg z-40 py-1"
              >
                {(progress || chapter) && (
                  <div
                    className="mb-1 border-b border-mid-gray/30 px-3 pt-1 pb-1.5 text-xs text-text-muted"
                    data-testid="reader-more-where"
                  >
                    {chapter && (
                      <div
                        className="truncate font-semibold text-text"
                        title={chapter}
                      >
                        {chapter}
                      </div>
                    )}
                    {progress
                      ? t("tts.reader.sentenceOf", {
                          position: Math.min(
                            progress.total,
                            progress.position + 1,
                          ),
                          count: progress.total,
                        })
                      : t("tts.reader.pageOf", { page: shownPage, count: pages })}
                  </div>
                )}
                {resumable && onRestart && (
                  <div
                    role="group"
                    aria-labelledby={resumeHeadId}
                    className="mb-1 border-b border-mid-gray/30 pb-1"
                  >
                    <div
                      id={resumeHeadId}
                      className="px-3 pt-1 pb-0.5 text-xs text-text-muted"
                      data-testid="reader-more-resume"
                    >
                      {resumeTitle}
                    </div>
                    <button
                      type="button"
                      role="menuitem"
                      tabIndex={-1}
                      onClick={() => {
                        onRestart();
                        setMoreOpen(false);
                        moreBtnRef.current?.focus();
                      }}
                      className="w-full px-3 py-1.5 text-sm text-start cursor-pointer hover:bg-mid-gray/15"
                      data-testid="reader-more-restart"
                    >
                      {t("tts.reader.restart")}
                    </button>
                  </div>
                )}
                <div role="group" aria-labelledby={speedHeadId}>
                  <div
                    id={speedHeadId}
                    className="px-3 pt-1 pb-0.5 text-xs text-text-muted"
                  >
                    {t("tts.settings.speed")}
                  </div>
                  <div role="none" className="reader-player__chips">
                    {speeds.map((value) => (
                      <button
                        key={`s${value}`}
                        type="button"
                        role="menuitemradio"
                        tabIndex={-1}
                        aria-checked={speed === value}
                        onClick={() => {
                          onSpeed(value);
                          setMoreOpen(false);
                          moreBtnRef.current?.focus();
                        }}
                        className="reader-player__chip tabular-nums"
                      >
                        {speedLabel(value)}
                      </button>
                    ))}
                  </div>
                </div>
                <div role="group" aria-labelledby={sleepHeadId}>
                  <div
                    id={sleepHeadId}
                    className="mt-1 border-t border-mid-gray/30 px-3 pt-1.5 pb-0.5 text-xs text-text-muted"
                  >
                    {sleepLeft !== null
                      ? `${t("tts.reader.sleepTimer")} · ${t("tts.reader.sleepLeft", { minutes: sleepLeft })}`
                      : t("tts.reader.sleepTimer")}
                  </div>
                  <div role="none" className="reader-player__chips">
                    {SLEEP_CHOICES.map((minutes) => (
                      <button
                        key={`t${minutes}`}
                        type="button"
                        role="menuitemradio"
                        tabIndex={-1}
                        aria-checked={minutes === sleepChoice}
                        onClick={() => {
                          chooseSleep(minutes);
                          setMoreOpen(false);
                          moreBtnRef.current?.focus();
                        }}
                        aria-label={
                          minutes === 0
                            ? t("tts.reader.sleepOff")
                            : t("tts.reader.sleepMinutes", { minutes })
                        }
                        className="reader-player__chip"
                      >
                        {minutes === 0
                          ? t("tts.reader.sleepOff")
                          : t("tts.reader.sleepMinutesShort", { minutes })}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
        <div className="reader-player__extra">
          <div className="relative" data-reader-popup="speed">
            <button
              type="button"
              className="reader-player__pill"
              onClick={() => setSpeedOpen((o) => !o)}
              aria-haspopup="listbox"
              aria-expanded={speedOpen}
              aria-label={t("tts.settings.speed")}
              title={t("tts.settings.speedDescription")}
            >
              {speedLabel(speed)}
            </button>
            {speedOpen && (
              <>
                <div
                  role="listbox"
                  aria-label={t("tts.settings.speed")}
                  className="absolute right-0 bottom-full mb-1 w-24 rounded-lg border border-mid-gray/40 bg-background shadow-lg z-40 py-1"
                >
                  {speeds.map((value) => (
                    <button
                      key={value}
                      type="button"
                      role="option"
                      aria-selected={speed === value}
                      onClick={() => {
                        onSpeed(value);
                        setSpeedOpen(false);
                      }}
                      className={`w-full px-3 py-1.5 text-sm text-start cursor-pointer tabular-nums hover:bg-mid-gray/15 ${
                        speed === value
                          ? "text-text font-medium"
                          : "text-text/80"
                      }`}
                    >
                      {speedLabel(value)}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
          <div className="relative" data-reader-popup="sleep">
            <button
              type="button"
              className="reader-player__pill"
              onClick={() => setSleepOpen((o) => !o)}
              aria-haspopup="menu"
              aria-expanded={sleepOpen}
              title={t("tts.reader.sleepTimer")}
              data-testid="reader-sleep"
            >
              <Moon width={14} height={14} aria-hidden="true" />
              <span>
                {sleepLeft !== null
                  ? t("tts.reader.sleepLeft", { minutes: sleepLeft })
                  : t("tts.reader.sleepTimer")}
              </span>
            </button>
            {sleepOpen && (
              <>
                <div
                  role="menu"
                  aria-label={t("tts.reader.sleepTimer")}
                  className="absolute right-0 bottom-full mb-1 w-40 rounded-lg border border-mid-gray/40 bg-background shadow-lg z-40 py-1"
                >
                  {SLEEP_CHOICES.map((minutes) => (
                    <button
                      key={minutes}
                      type="button"
                      role="menuitemradio"
                      aria-checked={minutes === sleepChoice}
                      onClick={() => {
                        chooseSleep(minutes);
                        setSleepOpen(false);
                      }}
                      className="w-full px-3 py-1.5 text-sm text-start cursor-pointer hover:bg-mid-gray/15"
                    >
                      {minutes === 0
                        ? t("tts.reader.sleepOff")
                        : t("tts.reader.sleepMinutes", { minutes })}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
