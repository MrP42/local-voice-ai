import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, ChevronUp, Pause, Play } from "lucide-react";
import { Button } from "../../../ui/Button";
import {
  LISTEN_WPM,
  READ_WPM,
  minutes,
  sectionAt,
  trackSentence,
  type Kurzbuch,
  type KurzbuchSection,
} from "@/lib/kurzbuch/kurzbuch";
import { loadHeard, saveHeard } from "@/lib/kurzbuch/heard";
import { kiLangLabel } from "./targetLang";

/**
 * Überblick eines Kurzbuchs (lva#205) neben dem Lesetext: Dauer, worum es
 * geht, für wen, Kernaussagen mit Abspielen und Gehört-Zustand. Der Text
 * selbst bleibt im Reader; ein Klick auf eine Kernaussage liest ab ihrer
 * Überschrift vor.
 */
export function KurzbuchPanel({
  kb,
  pageId,
  docId,
  language = null,
  title,
  current,
  playing,
  canResume,
  onPlayFrom,
  onResume,
  onStop,
}: {
  kb: Kurzbuch;
  pageId: string;
  docId: string;
  /** Sprache des Kurzbuchs (lva#217), fehlt bei älteren. */
  language?: string | null;
  title: string;
  /** Gerade vorgelesener Satz in diesem Dokument. */
  current: { start: number; end: number } | null;
  playing: boolean;
  canResume: boolean;
  onPlayFrom: (offset: number) => void;
  onResume: () => void;
  onStop: () => void;
}) {
  const { t } = useTranslation();
  const [heard, setHeard] = useState(() => loadHeard(pageId, docId));
  const entered = useRef(new Set<string>());
  const [open, setOpen] = useState(false);
  const moreId = useId();
  useEffect(() => {
    setHeard(loadHeard(pageId, docId));
    entered.current = new Set();
  }, [pageId, docId]);
  useEffect(() => {
    if (!playing || !current) return;
    const id = trackSentence(kb, entered.current, current.start, current.end);
    if (!id || heard.has(id)) return;
    const next = new Set(heard).add(id);
    setHeard(next);
    saveHeard(pageId, docId, next);
  }, [current?.start, playing]);

  const active = playing && current ? sectionAt(kb, current.start) : null;
  const sections: KurzbuchSection[] = [...kb.ideas, kb.conclusion];
  const nextUp =
    kb.ideas.findIndex((s) => !heard.has(s.id)) + 1 || kb.ideas.length;

  return (
    <aside
      className={`kurzbuch__panel${open ? " kurzbuch__panel--open" : ""}`}
      aria-label={t("tts.kurzbuch.overview")}
    >
      <div className="kidocs__meta">{t("tts.kurzbuch.kind")}</div>
      <h2 className="kurzbuch__title">{title}</h2>
      <div className="kurzbuch__facts">
        <span className="kurzbuch__chip">
          {t("tts.kurzbuch.ideas", { count: kb.ideas.length })}
        </span>
        <span className="kurzbuch__chip">
          {t("tts.kurzbuch.readMin", { min: minutes(kb.words, READ_WPM) })}
        </span>
        <span className="kurzbuch__chip">
          {t("tts.kurzbuch.listenMin", { min: minutes(kb.words, LISTEN_WPM) })}
        </span>
        {language && (
          <span className="kurzbuch__chip" data-testid="kurzbuch-lang">
            {t("tts.kiDocs.lang.chip", { lang: kiLangLabel(language) })}
          </span>
        )}
      </div>
      <Button
        variant="primary"
        className="kurzbuch__main"
        icon={playing ? <Pause size={16} /> : <Play size={16} />}
        onClick={playing ? onStop : canResume ? onResume : () => onPlayFrom(0)}
      >
        {playing
          ? t("tts.kurzbuch.pause")
          : canResume
            ? t("tts.kurzbuch.resumeAt", { n: nextUp })
            : t("tts.kurzbuch.start")}
      </Button>
      {/* Schmal (über dem Text) zugeklappt, damit der Lesetext Platz behält. */}
      <Button
        variant="ghost"
        size="sm"
        className="kurzbuch__toggle"
        aria-expanded={open}
        aria-controls={moreId}
        icon={open ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
        onClick={() => setOpen((o) => !o)}
      >
        {open ? t("tts.kurzbuch.less") : t("tts.kurzbuch.more")}
      </Button>
      <div className="kurzbuch__more" id={moreId}>
        {kb.about && (
          <p className="kurzbuch__about">
            <b>{t("tts.kurzbuch.about")}</b> {kb.about}
          </p>
        )}
        {kb.audience.length > 0 && (
          <>
            <div className="kurzbuch__label">{t("tts.kurzbuch.audience")}</div>
            <ul className="kurzbuch__who">
              {kb.audience.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          </>
        )}
        <ol className="kurzbuch__toc" aria-label={t("tts.kurzbuch.list")}>
          {sections.map((s, i) => {
            const isIdea = i < kb.ideas.length;
            const on = active?.id === s.id;
            const done = heard.has(s.id);
            const label = isIdea
              ? t("tts.kurzbuch.ideaN", { n: i + 1, total: kb.ideas.length })
              : s.title;
            return (
              <li
                key={s.id + i}
                className={`kurzbuch__item${on ? " kurzbuch__item--on" : ""}`}
                aria-current={on ? "true" : undefined}
              >
                <span className="kurzbuch__num" aria-hidden="true">
                  {isIdea ? i + 1 : ""}
                  {done && <Check size={12} className="kurzbuch__done" />}
                </span>
                <span className="kurzbuch__text">
                  {done && (
                    <span className="sr-only">{t("tts.kurzbuch.heard")} </span>
                  )}
                  {s.keyword ? (
                    <>
                      <b>{s.keyword}:</b> {s.statement}
                    </>
                  ) : (
                    <b>{s.statement}</b>
                  )}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  icon={on ? <Pause size={14} /> : <Play size={14} />}
                  aria-label={
                    on
                      ? t("tts.kurzbuch.pauseItem", { item: label })
                      : t("tts.kurzbuch.playItem", { item: label })
                  }
                  title={on ? t("tts.kurzbuch.pause") : t("tts.kurzbuch.play")}
                  onClick={() => (on ? onStop() : onPlayFrom(s.start))}
                />
              </li>
            );
          })}
        </ol>
      </div>
    </aside>
  );
}
