import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDownToLine, X } from "lucide-react";
import { IconButton } from "../ui/IconButton";
import type { TextParagraph } from "@/lib/library/follow";

/** So lange pausiert das Mitscrollen nach einem Scrollen des Nutzers. */
export const FOLLOW_PAUSE_MS = 5000;

const Paragraph = React.memo(
  ({
    p,
    index,
    active,
    onPick,
  }: {
    p: TextParagraph;
    index: number;
    active: boolean;
    onPick(index: number): void;
  }) => (
    <p
      className={`library-follow__p${p.heading ? " library-follow__p--head" : ""}${active ? " library-follow__p--now" : ""}`}
      data-index={index}
      data-active={active ? "true" : undefined}
      aria-current={active ? "location" : undefined}
      onClick={() => onPick(index)}
    >
      {p.text}
    </p>
  ),
);
Paragraph.displayName = "Paragraph";

/**
 * Lesefläche zum Mitlesen (lva#204): der Text des Buchs, der gerade gehörte
 * Absatz hervorgehoben und mitgescrollt. Ein Scrollen des Nutzers hält das
 * Mitscrollen {@link FOLLOW_PAUSE_MS} an; danach erscheint „Zur aktuellen
 * Stelle“. Ein Klick auf einen Absatz springt im Hörbuch dorthin.
 */
export const FollowPane: React.FC<{
  title: string;
  paragraphs: TextParagraph[];
  active: number;
  /** Stelle nur geschätzt (Text und Hörbuch ohne gemeinsame Zeitachse). */
  approx: boolean;
  onPick(index: number): void;
  onClose(): void;
}> = ({ title, paragraphs, active, approx, onPick, onClose }) => {
  const { t } = useTranslation();
  const box = useRef<HTMLDivElement>(null);
  const [away, setAway] = useState(false);
  const pausedUntil = useRef(0);
  const timer = useRef<number | null>(null);
  const lastActive = useRef(-1);

  const scrollToActive = useCallback((smooth: boolean) => {
    const el = box.current?.querySelector<HTMLElement>('[data-active="true"]');
    const container = box.current;
    if (!el || !container) return;
    const top = el.offsetTop - container.clientHeight / 2 + el.offsetHeight / 2;
    container.scrollTo({
      top: Math.max(0, top),
      behavior: smooth ? "smooth" : "auto",
    });
  }, []);

  // Mitscrollen, solange der Nutzer nicht gerade selbst scrollt.
  useEffect(() => {
    if (active === lastActive.current && !away) return;
    lastActive.current = active;
    if (Date.now() < pausedUntil.current || away) return;
    scrollToActive(true);
  }, [active, away, scrollToActive]);

  // Beim Öffnen sofort an die Stelle, ohne Fahrt durch den ganzen Text.
  useEffect(() => {
    scrollToActive(false);
  }, [scrollToActive]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const userScrolled = () => {
    pausedUntil.current = Date.now() + FOLLOW_PAUSE_MS;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setAway(true);
    }, FOLLOW_PAUSE_MS);
  };

  const backToNow = () => {
    pausedUntil.current = 0;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    setAway(false);
    scrollToActive(true);
  };

  const pick = (index: number) => {
    pausedUntil.current = 0;
    setAway(false);
    onPick(index);
  };

  return (
    <div
      className="library-follow"
      role="region"
      aria-label={t("library.follow.region", { title })}
      data-testid="library-follow"
    >
      <div className="library-follow__head">
        <strong className="library-follow__title">{title}</strong>
        {approx && (
          <span className="library-meta" data-testid="library-follow-approx">
            {t("library.follow.approx")}
          </span>
        )}
        <span className="library-bar__spacer" />
        <IconButton
          icon={<X size={18} aria-hidden="true" />}
          label={t("library.follow.close")}
          onClick={onClose}
          data-testid="library-follow-close"
        />
      </div>
      <div
        className="library-follow__text"
        ref={box}
        // Nur echte Eingaben des Nutzers zählen, nicht das eigene Mitscrollen.
        onWheel={userScrolled}
        onTouchMove={userScrolled}
        onPointerDown={(e) => {
          // Auf die Bildlaufleiste getippt (Ziel ist der Behälter selbst).
          if (e.target === e.currentTarget) userScrolled();
        }}
        onKeyDown={(e) => {
          if (
            [
              "ArrowUp",
              "ArrowDown",
              "PageUp",
              "PageDown",
              "Home",
              "End",
              " ",
            ].includes(e.key)
          )
            userScrolled();
        }}
        tabIndex={0}
        lang="de"
        data-testid="library-follow-text"
      >
        {paragraphs.map((p, i) => (
          <Paragraph
            key={p.start}
            p={p}
            index={i}
            active={i === active}
            onPick={pick}
          />
        ))}
      </div>
      {away && (
        <button
          type="button"
          className="library-follow__now"
          onClick={backToNow}
          data-testid="library-follow-now"
        >
          <ArrowDownToLine size={16} aria-hidden="true" />
          {t("library.follow.current")}
        </button>
      )}
    </div>
  );
};
