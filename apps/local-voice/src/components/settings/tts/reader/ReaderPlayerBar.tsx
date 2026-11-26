import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSectionActive } from "../../../shell/KeepAlive";
import {
  ChevronUp,
  Globe,
  Moon,
  MoreHorizontal,
  Square,
  UserRound,
} from "lucide-react";
import { Glyph } from "../../../ui/AudioPlayer";
import type { ReaderPageInfo } from "./ReaderView";
import { useSentenceClock, type SentenceClock } from "./useSentenceClock";
import { FlowControl } from "./FlowControl";

/** Pfeiltasten, Pos1 und Ende bewegen den Fokus in einer Auswahl
 *  (Radiogruppe, Menü), wie es WAI-ARIA erwartet (Codex-Review #788). */
function arrowFocus(
  e: React.KeyboardEvent<HTMLElement>,
  selector: string,
): number | null {
  const keys = [
    "ArrowDown",
    "ArrowRight",
    "ArrowUp",
    "ArrowLeft",
    "Home",
    "End",
  ];
  if (!keys.includes(e.key)) return null;
  const items = Array.from(
    e.currentTarget.querySelectorAll<HTMLElement>(selector),
  );
  if (items.length === 0) return null;
  e.preventDefault();
  const at = items.indexOf(document.activeElement as HTMLElement);
  const next =
    e.key === "Home"
      ? 0
      : e.key === "End"
        ? items.length - 1
        : (Math.max(at, 0) +
            (e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : -1) +
            items.length) %
          items.length;
  items[next].focus();
  return next;
}

export const SLEEP_CHOICES = [0, 15, 30, 45, 60] as const;

interface ReaderPlayerBarProps {
  speaking: boolean;
  /** Abspielen möglich (Text vorhanden). */
  canPlay: boolean;
  /** Satzsprünge möglich (laufende oder pausierte Sitzung). */
  canSeek: boolean;
  onPlayPause(): void;
  onSeek(delta: number): void;
  /** Absatz zurück/vor (lva#781). Ohne Handler entfallen die Knöpfe. */
  onParagraph?(delta: -1 | 1): void;
  onStop(): void;
  speed: number;
  speeds: readonly number[];
  speedLabel(value: number): string;
  onSpeed(value: number): void;
  /** Sprachfluss 0 (zügig) bis 1 (ruhig): Pausen an Komma, Satz- und Absatzende. */
  flow: number;
  onFlow(value: number): void;
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
  /** Alle Kapitel mit Seite (0-basiert, nach dem Umbruch gemessen): ein Klick
   *  auf das Kapitel öffnet die Liste, ein Eintrag springt hin (Patrick 01.11.). */
  chapters?: { title: string; level: number; page: number | null }[];
  /** Index des aktuellen Kapitels in `chapters` (-1: davor). */
  chapterIndex?: number;
  onChapter?(index: number): void;
  /** Seite der gemerkten Stelle (1-basiert), wenn bekannt. */
  resumePage?: number | null;
  /** Gemerkte Stelle verwerfen: Play beginnt wieder vorn. */
  onRestart?(): void;
  /** Name der Stimme, die liest („Stimme: Annika“). */
  voiceName?: string;
  /** Stimmenwahl im Fenster des Knopfs „Stimme“. Fehlt sie, ruft der Knopf
   *  `onVoice` (die Stimme steht dann in der rechten Spalte). */
  voice?: React.ReactNode;
  onVoice?(): void;
  /** Inhalt des Fensters „Sprache“ (Fassungen, Übersetzungen). */
  language?: React.ReactNode | ((close: () => void) => React.ReactNode);
  /**
   * Weitere Werkzeuge (Speichern, Diktat, Export …). Sie stehen kompakt im
   * Fenster „Mehr“ (`inPopover` = false: Fortschritt und Hinweise legen sich
   * dann als eigenes Fenster über die Leiste und bleiben sichtbar, auch wenn
   * „Mehr“ zu ist).
   */
  dock?: (inPopover: boolean) => React.ReactNode;
}

type Popup = "speed" | "voice" | "lang" | "sleep" | "more" | "chapters";

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
 * Wiedergabeleiste unter der Buchseite. Seit lva#781 wie am iPhone: oben
 * Stand und Stimme, darunter Absatz/Satz/Play/Satz/Absatz mit Beschriftung,
 * dann Tempo, Stimme, Sprache, Schlaftimer und „Mehr“. Breit steht alles in
 * einer Zeile, schmal untereinander (Container-Query in reader.css).
 */
export const ReaderPlayerBar: React.FC<ReaderPlayerBarProps> = ({
  speaking,
  canPlay,
  canSeek,
  onPlayPause,
  onSeek,
  onParagraph,
  onStop,
  speed,
  speeds,
  speedLabel,
  onSpeed,
  flow,
  onFlow,
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
  chapters = [],
  chapterIndex = -1,
  onChapter,
  onRestart,
  voiceName,
  voice,
  onVoice,
  language,
  dock,
}) => {
  const { t } = useTranslation();
  const [popup, setPopup] = useState<Popup | null>(null);
  const toggle = (kind: Popup) => setPopup((p) => (p === kind ? null : kind));
  const btnRefs = useRef<Partial<Record<Popup, HTMLButtonElement | null>>>({});
  const panelRefs = useRef<Partial<Record<Popup, HTMLDivElement | null>>>({});
  const chaptersId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const [sleepUntil, setSleepUntil] = useState<number | null>(null);
  /** Gewählte Timer-Dauer in Minuten (0 = aus) — für aria-checked. */
  const [sleepChoice, setSleepChoice] = useState(0);
  const flowHeadId = useId();
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

  const close = (focusBack = false) => {
    if (focusBack && popup) btnRefs.current[popup]?.focus();
    setPopup(null);
  };

  // Escape und Klick daneben schließen das offene Fenster.
  useEffect(() => {
    if (!popup) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      btnRefs.current[popup]?.focus();
      setPopup(null);
    };
    // Kein fixed-Hintergrund: die Leiste ist ein Größen-Container
    // (Container-Query) und damit Bezugsrahmen für fixed.
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element | null;
      if (target?.closest?.("[data-reader-popup]")) return;
      // Menüs aus den Fenstern (Stimmenliste, Aktionsmenüs) liegen als Portal
      // außerhalb der Leiste: ein Klick dort schließt sie nicht.
      if (
        !rootRef.current?.contains(target) &&
        target?.closest?.(
          '[role="listbox"], [role="option"], [role="menu"], [role="menuitem"], [id^="react-select"], [class*="__menu"]',
        )
      )
        return;
      setPopup(null);
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onDown, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onDown, true);
    };
  }, [popup]);

  // Geöffnetes Fenster: Fokus auf den gewählten bzw. ersten Eintrag.
  useEffect(() => {
    if (!popup) return;
    const panel = panelRefs.current[popup];
    if (!panel) return;
    if (popup === "chapters") {
      const active =
        panel.querySelector<HTMLElement>('[aria-current="location"]') ??
        panel.querySelector<HTMLElement>("button");
      active?.scrollIntoView({ block: "center" });
      active?.focus();
      return;
    }
    const first =
      panel.querySelector<HTMLElement>(
        '[aria-checked="true"], [aria-pressed="true"]',
      ) ??
      panel.querySelector<HTMLElement>(
        'input, button:not([disabled]), [tabindex]:not([tabindex="-1"]), [role="menuitem"], [role="menuitemradio"]',
      );
    first?.focus();
  }, [popup]);

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
  // „Text: 62 % gelesen“: im Lauf nach Sätzen, sonst nach der Seite.
  const readPercent = Math.round(
    (progress ? fraction : pages > 1 ? (shownPage - 1) / (pages - 1) : 0) * 100,
  );

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

  const canListChapters = chapters.length > 0 && !!onChapter;
  const chapterNode = canListChapters ? (
    <span className="reader-player__chapter-wrap" data-reader-popup="chapters">
      <button
        type="button"
        ref={(el) => {
          btnRefs.current.chapters = el;
        }}
        className="reader-player__chapter reader-player__chapter--button"
        onClick={() => toggle("chapters")}
        aria-haspopup="true"
        aria-expanded={popup === "chapters"}
        aria-controls={popup === "chapters" ? chaptersId : undefined}
        title={t("tts.reader.chapterListHint")}
        data-testid="reader-chapter"
      >
        <span className="reader-player__chapter-text">
          {chapter ?? t("tts.reader.chapterList")}
        </span>
        <ChevronUp width={14} height={14} aria-hidden="true" />
      </button>
    </span>
  ) : chapter ? (
    <span
      className="reader-player__chapter"
      title={chapter}
      data-testid="reader-chapter"
    >
      {chapter}
    </span>
  ) : null;

  const sectionActive = useSectionActive();
  // Schmal steht die Leiste fest unten (reader.css): Höhe an den
  // Arbeitsbereich melden (Platz frei halten), Abstand = Statusleiste.
  useEffect(() => {
    const root = rootRef.current;
    const workspace = root?.closest<HTMLElement>(".tts-workspace");
    if (!root || !workspace) return;
    const report = () => {
      const fixed = getComputedStyle(root).position === "fixed";
      const status = document.querySelector(".lv-statusbar");
      const bottom = status
        ? Math.max(0, window.innerHeight - status.getBoundingClientRect().top)
        : 0;
      root.style.setProperty("--reader-player-bottom", `${bottom}px`);
      const h = Math.ceil(root.getBoundingClientRect().height);
      workspace.style.setProperty(
        "--reader-player-h",
        fixed ? `${h + 12}px` : "0px",
      );
      // App-weit: Meldungen (Toaster) und Bildlauf-Ziele halten Abstand zur
      // festen Leiste – nur solange Vorlesen sichtbar ist.
      document.documentElement.style.setProperty(
        "--lv-player-h",
        fixed && sectionActive ? `${h}px` : "0px",
      );
      // Toasts weichen der festen Leiste aus, damit ein Ziel (Satz, Fundstelle)
      // beim Hinscrollen über ihr landet und nicht darunter (lva#217/#365).
      document.documentElement.style.setProperty(
        "--lv-toast-zone",
        fixed && sectionActive ? "128px" : "0px",
      );
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(root);
    window.addEventListener("resize", report);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", report);
      workspace.style.removeProperty("--reader-player-h");
      document.documentElement.style.removeProperty("--lv-player-h");
      document.documentElement.style.removeProperty("--lv-toast-zone");
    };
  }, [sectionActive]);

  /** Beschrifteter Knopf der Werkzeugzeile (Symbol oben, Wort darunter). */
  const toolButton = (
    kind: Popup,
    icon: React.ReactNode,
    label: string,
    extra: {
      title?: string;
      testId: string;
      onClick?: () => void;
      haspopup?: "menu" | "dialog" | "listbox";
      ariaLabel?: string;
      active?: boolean;
    },
  ) => (
    <button
      type="button"
      ref={(el) => {
        btnRefs.current[kind] = el;
      }}
      className={`reader-tool${extra.active ? " reader-tool--active" : ""}`}
      onClick={extra.onClick ?? (() => toggle(kind))}
      aria-haspopup={extra.onClick ? undefined : (extra.haspopup ?? "dialog")}
      aria-expanded={extra.onClick ? undefined : popup === kind}
      aria-label={extra.ariaLabel}
      title={extra.title ?? label}
      data-testid={extra.testId}
    >
      <span className="reader-tool__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="reader-tool__label">{label}</span>
    </button>
  );

  /** Fenster über einem Werkzeugknopf. */
  const panel = (
    kind: Popup,
    label: string,
    children: React.ReactNode,
    opts: {
      role?: "dialog" | "menu";
      testId?: string;
      wide?: boolean;
      /** Inhalt nur bei offenem Fenster einhängen (Stimmenliste, Sprachen). */
      lazy?: boolean;
    } = {},
  ) => (
    <div
      ref={(el) => {
        panelRefs.current[kind] = el;
      }}
      role={opts.role ?? "dialog"}
      aria-label={label}
      hidden={popup !== kind}
      className={`reader-popover${opts.wide ? " reader-popover--wide" : ""}`}
      data-testid={opts.testId}
    >
      {opts.lazy && popup !== kind ? null : children}
    </div>
  );

  const sleepLabel =
    sleepLeft !== null
      ? t("tts.reader.sleepLeft", { minutes: sleepLeft })
      : t("tts.reader.sleepTimer");

  return (
    <div
      ref={rootRef}
      className="reader-player"
      role="region"
      aria-label={t("tts.reader.player")}
      data-testid="reader-player"
    >
      <div className="reader-player__meta">
        <span
          className={`reader-player__where${
            !progress && resumable
              ? " reader-player__where--resume"
              : chapter || canListChapters
                ? " reader-player__where--chapter"
                : ""
          }`}
          data-testid="reader-where"
        >
          {progress ? (
            <>
              {chapterNode}
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
            <>
              {chapterNode}
              <span className="reader-player__sentence">
                {t("tts.reader.pageOf", { page: shownPage, count: pages })}
              </span>
            </>
          )}
        </span>
        <span className="reader-player__status" data-testid="reader-status">
          <span data-testid="reader-percent">
            {t("tts.reader.percentRead", { percent: readPercent })}
          </span>
          {voiceName && (
            <span
              className="reader-player__voice-name"
              title={`${t("tts.voices.title")}: ${voiceName}`}
              data-testid="reader-voice-name"
            >
              {t("tts.reader.voiceIs", { name: voiceName })}
            </span>
          )}
        </span>
        {popup === "chapters" && (
          <div
            id={chaptersId}
            ref={(el) => {
              panelRefs.current.chapters = el;
            }}
            role="navigation"
            aria-label={t("tts.reader.chapterList")}
            className="reader-player__chapters"
            data-reader-popup="chapters"
            data-testid="reader-chapter-list"
            onKeyDown={(e) => {
              if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
              e.preventDefault();
              const items = Array.from(
                e.currentTarget.querySelectorAll<HTMLElement>("button"),
              );
              const i = items.indexOf(document.activeElement as HTMLElement);
              const next =
                (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) %
                items.length;
              items[next]?.focus();
            }}
          >
            <ol>
              {chapters.map((c, i) => (
                <li key={`${i}-${c.title}`}>
                  <button
                    type="button"
                    aria-current={i === chapterIndex ? "location" : undefined}
                    className="reader-player__chapters-item"
                    style={{
                      paddingInlineStart: `${10 + (c.level - 1) * 12}px`,
                    }}
                    onClick={() => {
                      onChapter?.(i);
                      close(true);
                    }}
                    data-testid="reader-chapter-entry"
                  >
                    <span
                      className="reader-player__chapters-title"
                      title={c.title}
                    >
                      {c.title}
                    </span>
                    {c.page !== null && (
                      <span className="reader-player__chapters-page">
                        {c.page + 1}
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ol>
          </div>
        )}
      </div>
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
        <div
          className="reader-transport"
          role="group"
          aria-label={t("tts.reader.transport")}
        >
          {onParagraph && (
            <button
              type="button"
              className="reader-transport__btn"
              onClick={() => onParagraph(-1)}
              disabled={!canPlay}
              aria-label={t("tts.reader.previousParagraph")}
              title={t("tts.reader.previousParagraph")}
              data-testid="reader-paragraph-prev"
            >
              <Glyph name="prev" />
              <span className="reader-transport__label">
                {t("tts.reader.paragraph")}
              </span>
            </button>
          )}
          <button
            type="button"
            className="reader-transport__btn"
            onClick={() => onSeek(-1)}
            disabled={!canSeek}
            aria-label={t("tts.previousSentence")}
            title={t("tts.previousSentence")}
            data-testid="reader-sentence-prev"
          >
            <Glyph name="rewind" />
            <span className="reader-transport__label">
              {t("tts.reader.sentence")}
            </span>
          </button>
          <button
            type="button"
            className="reader-transport__play"
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
            className="reader-transport__btn"
            onClick={() => onSeek(1)}
            disabled={!canSeek}
            aria-label={t("tts.nextSentence")}
            title={t("tts.nextSentence")}
            data-testid="reader-sentence-next"
          >
            <Glyph name="forward" />
            <span className="reader-transport__label">
              {t("tts.reader.sentence")}
            </span>
          </button>
          {onParagraph && (
            <button
              type="button"
              className="reader-transport__btn"
              onClick={() => onParagraph(1)}
              disabled={!canPlay}
              aria-label={t("tts.reader.nextParagraph")}
              title={t("tts.reader.nextParagraph")}
              data-testid="reader-paragraph-next"
            >
              <Glyph name="next" />
              <span className="reader-transport__label">
                {t("tts.reader.paragraph")}
              </span>
            </button>
          )}
        </div>
        <div className="reader-player__tools">
          <div className="reader-tool-wrap" data-reader-popup="speed">
            {toolButton(
              "speed",
              <span className="reader-tool__speed">{speedLabel(speed)}</span>,
              t("tts.reader.tempo"),
              {
                testId: "reader-speed",
                haspopup: "dialog",
                title: t("tts.settings.speedDescription"),
                ariaLabel: `${t("tts.settings.speed")}: ${speedLabel(speed)}`,
              },
            )}
            {panel(
              "speed",
              t("tts.settings.speed"),
              <>
                <div
                  role="radiogroup"
                  aria-label={t("tts.settings.speed")}
                  className="reader-player__chips"
                  onKeyDown={(e) => {
                    // Radiogruppe: Pfeile wählen zugleich (WAI-ARIA), ohne
                    // das Fenster zu schließen.
                    const next = arrowFocus(e, '[role="radio"]');
                    if (next !== null) onSpeed(speeds[next]);
                  }}
                >
                  {speeds.map((value) => (
                    <button
                      key={`s${value}`}
                      type="button"
                      role="radio"
                      aria-checked={speed === value}
                      tabIndex={speed === value ? 0 : -1}
                      onClick={() => {
                        onSpeed(value);
                        close(true);
                      }}
                      className="reader-player__chip tabular-nums"
                    >
                      {speedLabel(value)}
                    </button>
                  ))}
                </div>
                <div
                  id={flowHeadId}
                  className="reader-popover__head"
                  title={t("tts.settings.flowHint")}
                >
                  {t("tts.settings.flow")}
                </div>
                <div
                  className="px-2 pb-1.5"
                  role="group"
                  aria-labelledby={flowHeadId}
                >
                  <FlowControl value={flow} onChange={onFlow} />
                </div>
              </>,
              { testId: "reader-speed-panel" },
            )}
          </div>
          <div className="reader-tool-wrap" data-reader-popup="voice">
            {toolButton(
              "voice",
              <UserRound width={20} height={20} />,
              t("tts.reader.voice"),
              {
                testId: "reader-voice",
                onClick: voice ? undefined : onVoice,
                title: voiceName
                  ? `${t("tts.voices.title")}: ${voiceName}`
                  : t("tts.voices.title"),
              },
            )}
            {voice &&
              panel("voice", t("tts.voices.title"), voice, {
                testId: "reader-voice-panel",
                wide: true,
                lazy: true,
              })}
          </div>
          <div className="reader-tool-wrap" data-reader-popup="lang">
            {toolButton(
              "lang",
              <Globe width={20} height={20} />,
              t("tts.reader.language"),
              {
                testId: "reader-language",
              },
            )}
            {panel(
              "lang",
              t("tts.reader.language"),
              (typeof language === "function"
                ? language(() => close(true))
                : language) ?? (
                <p className="reader-popover__hint">
                  {t("tts.reader.languageNone")}
                </p>
              ),
              { testId: "reader-language-panel", wide: true, lazy: true },
            )}
          </div>
          <div className="reader-tool-wrap" data-reader-popup="sleep">
            {toolButton("sleep", <Moon width={20} height={20} />, sleepLabel, {
              testId: "reader-sleep",
              haspopup: "menu",
              title: t("tts.reader.sleepTimer"),
              active: sleepLeft !== null,
            })}
            {panel(
              "sleep",
              t("tts.reader.sleepTimer"),
              <div
                role="none"
                className="reader-popover__list"
                onKeyDown={(e) => arrowFocus(e, '[role="menuitemradio"]')}
              >
                {SLEEP_CHOICES.map((minutes) => (
                  <button
                    key={minutes}
                    type="button"
                    role="menuitemradio"
                    aria-checked={minutes === sleepChoice}
                    onClick={() => {
                      chooseSleep(minutes);
                      close(true);
                    }}
                    className="reader-popover__item"
                  >
                    {minutes === 0
                      ? t("tts.reader.sleepOff")
                      : t("tts.reader.sleepMinutes", { minutes })}
                  </button>
                ))}
              </div>,
              { role: "menu" },
            )}
          </div>
          <div className="reader-tool-wrap" data-reader-popup="more">
            {toolButton(
              "more",
              <MoreHorizontal width={20} height={20} />,
              t("tts.reader.more"),
              { testId: "reader-more", title: t("tts.reader.moreOptions") },
            )}
            {panel(
              "more",
              t("tts.reader.moreOptions"),
              <>
                {(progress || chapter) && (
                  <div
                    className="reader-popover__where"
                    data-testid="reader-more-where"
                  >
                    {chapter && (
                      <div
                        className="font-semibold text-text [overflow-wrap:anywhere]"
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
                      : t("tts.reader.pageOf", {
                          page: shownPage,
                          count: pages,
                        })}
                  </div>
                )}
                {resumable && onRestart && (
                  <div role="group" aria-labelledby={resumeHeadId}>
                    <div
                      id={resumeHeadId}
                      className="reader-popover__head"
                      data-testid="reader-more-resume"
                    >
                      {resumeTitle}
                    </div>
                    <button
                      type="button"
                      onClick={() => {
                        onRestart();
                        close(true);
                      }}
                      className="reader-popover__item"
                      data-testid="reader-more-restart"
                    >
                      {t("tts.reader.restart")}
                    </button>
                  </div>
                )}
                <button
                  type="button"
                  onClick={() => {
                    onStop();
                    close(true);
                  }}
                  className="reader-popover__item"
                  data-testid="reader-stop"
                >
                  <Square width={14} height={14} aria-hidden="true" />
                  {t("tts.stop")}
                </button>
                {dock && (
                  // Bleibt eingehängt (nur verborgen): laufende Vorgänge der
                  // Werkzeuge (Auto-Tagging, Export) behalten ihren Zustand.
                  <div
                    className="reader-popover__dock"
                    data-testid="reader-dock-panel"
                  >
                    {dock(false)}
                  </div>
                )}
              </>,
              { testId: "reader-more-panel", wide: true },
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
