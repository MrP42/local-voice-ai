import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Button as FluentButton,
  Input,
  Switch,
} from "@fluentui/react-components";
import { commands } from "@/bindings";
import { GUIDE_SEEN_KEY, GUIDE_STEPS, storyPrompt } from "./steps";
import { loadSpeakers } from "./useSpeakers";
import "./chatGuide.css";

/** Systemeinstellung „Bewegung reduzieren“, auch wenn sie sich ändert. */
function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () =>
      typeof window !== "undefined" &&
      !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const q = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    if (!q) return;
    const on = () => setReduced(q.matches);
    q.addEventListener?.("change", on);
    return () => q.removeEventListener?.("change", on);
  }, []);
  return reduced;
}

export function guideSeen(): boolean {
  try {
    return localStorage.getItem(GUIDE_SEEN_KEY) === "1";
  } catch {
    return true;
  }
}

function markSeen() {
  try {
    localStorage.setItem(GUIDE_SEEN_KEY, "1");
  } catch {
    /* ohne Speicher erscheint die Führung beim nächsten Start erneut */
  }
}

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function rectOf(selector?: string): Rect | null {
  if (!selector) return null;
  const el = document.querySelector<HTMLElement>(selector);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return null;
  return { x: r.left, y: r.top, w: r.width, h: r.height };
}

export interface ChatGuideProps {
  open: boolean;
  onClose: () => void;
  /** Schickt die Bitte um die Geschichte als Nachricht in den Chat. */
  onStory: (prompt: string, display: string) => void;
}

/**
 * Führung beim ersten Start im Chat (lva#394 C2, Codex F4): die KI spricht
 * (Standardstimme, Text läuft mit), ein Zeiger fährt zum genannten Element und
 * rahmt es ein. Text und Markierung wechseln immer gemeinsam. Überspringen und
 * Esc beenden sofort; „Bewegung reduzieren“ zeigt nur den Rahmen, ohne Zeiger
 * und ohne Puls.
 */
export const ChatGuide: React.FC<ChatGuideProps> = ({
  open,
  onClose,
  onStory,
}) => {
  const { t, i18n } = useTranslation();
  const reduced = useReducedMotion();
  const [index, setIndex] = useState(0);
  const [voice, setVoice] = useState(true);
  const [topic, setTopic] = useState("");
  const [rect, setRect] = useState<Rect | null>(null);
  const titleId = useId();
  const textId = useId();
  const nextRef = useRef<HTMLButtonElement>(null);
  const step = GUIDE_STEPS[index];
  const last = index === GUIDE_STEPS.length - 1;
  const caption = t(`chatGuide.step.${step.id}`);

  // Markierung folgt dem Schritt (und Fenstergröße).
  useEffect(() => {
    if (!open) return;
    const update = () => setRect(rectOf(step.target));
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [open, step.target]);

  // Sprechen: jeder Schritt bricht den vorigen ab.
  useEffect(() => {
    if (!open || !voice) return;
    void commands.ttsSpeakText(caption).catch(() => undefined);
    return () => void commands.ttsCancel().catch(() => undefined);
  }, [open, voice, caption]);

  useEffect(() => {
    if (open) {
      setIndex(0);
      window.requestAnimationFrame(() => nextRef.current?.focus());
    }
  }, [open]);

  const finish = useCallback(() => {
    markSeen();
    void commands.ttsCancel().catch(() => undefined);
    onClose();
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      finish();
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [open, finish]);

  const tellStory = async () => {
    const theme = topic.trim() || t("chatGuide.story.defaultTopic");
    const speakers = await loadSpeakers();
    finish();
    onStory(
      storyPrompt({ topic: theme, speakers, language: i18n.language }),
      t("chatGuide.story.display", { topic: theme }),
    );
  };

  if (!open) return null;

  const overlay = createPortal(
    <>
      {rect && (
        <div
          className="lv-guide__ring"
          data-reduced={reduced || undefined}
          aria-hidden="true"
          style={{
            left: rect.x - 4,
            top: rect.y - 4,
            width: rect.w + 8,
            height: rect.h + 8,
          }}
        />
      )}
      {rect && !reduced && (
        <svg
          className="lv-guide__cursor"
          aria-hidden="true"
          viewBox="0 0 24 24"
          width="24"
          height="24"
          style={{
            transform: `translate(${rect.x + rect.w / 2}px, ${rect.y + rect.h / 2}px)`,
          }}
        >
          <path
            d="M4 2l16 10-7 1.5L9.5 21z"
            fill="#fff"
            stroke="#000"
            strokeWidth="1.4"
            strokeLinejoin="round"
          />
        </svg>
      )}
    </>,
    document.body,
  );

  return (
    <section
      className="lv-guide"
      role="region"
      aria-labelledby={titleId}
      aria-describedby={textId}
      data-testid="chat-guide"
      data-step={step.id}
    >
      {overlay}
      <div
        className="lv-guide__orb"
        data-reduced={reduced || undefined}
        aria-hidden="true"
      />
      <h2 id={titleId} className="sr-only">
        {t("chatGuide.title")}
      </h2>
      <p className="lv-guide__count" role="status">
        {t("chatGuide.count", { n: index + 1, total: GUIDE_STEPS.length })}
      </p>
      <p id={textId} className="lv-guide__text" aria-live="polite">
        {caption}
      </p>
      {last && (
        <Input
          className="lv-guide__topic"
          value={topic}
          onChange={(_e, d) => setTopic(d.value)}
          placeholder={t("chatGuide.story.defaultTopic")}
          aria-label={t("chatGuide.story.topic")}
          onKeyDown={(e) => {
            if (e.key === "Enter") void tellStory();
          }}
        />
      )}
      <div className="lv-guide__actions">
        <Switch
          checked={voice}
          onChange={(_e, d) => setVoice(d.checked)}
          label={t("chatGuide.voice")}
        />
        <span className="lv-guide__spacer" />
        <FluentButton appearance="subtle" onClick={finish}>
          {t("chatGuide.skip")}
        </FluentButton>
        {index > 0 && (
          <FluentButton
            appearance="secondary"
            onClick={() => setIndex((i) => i - 1)}
          >
            {t("chatGuide.back")}
          </FluentButton>
        )}
        <FluentButton
          ref={nextRef}
          appearance="primary"
          onClick={() => (last ? void tellStory() : setIndex((i) => i + 1))}
        >
          {last ? t("chatGuide.story.go") : t("chatGuide.next")}
        </FluentButton>
      </div>
    </section>
  );
};
