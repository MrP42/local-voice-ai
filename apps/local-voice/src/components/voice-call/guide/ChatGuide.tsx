import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Button as FluentButton,
  Input,
  Switch,
} from "@fluentui/react-components";
import { commands } from "@/bindings";
import { GUIDE_STEPS, storyPrompt } from "./steps";
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

  // Markierung folgt dem Schritt. Ziele können spät erscheinen oder sich durch
  // Layout, Scrollen oder Fenstergröße verschieben: regelmäßig nachmessen und nur
  // bei Änderung neu zeichnen.
  useEffect(() => {
    if (!open) return;
    let prev = "";
    const update = () => {
      const r = rectOf(step.target);
      const key = r ? `${r.x}|${r.y}|${r.w}|${r.h}` : "";
      if (key === prev) return;
      prev = key;
      setRect(r);
    };
    update();
    const timer = window.setInterval(update, 200);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open, step.target]);

  // Sprechen: ein neuer Text ersetzt die laufende Sitzung im Kern. Aufrufe laufen
  // nacheinander (Kette), abgebrochen wird nur, was die Führung selbst gestartet hat.
  const spoke = useRef(false);
  const chain = useRef<Promise<unknown>>(Promise.resolve());
  const enqueue = (job: () => Promise<unknown>) => {
    chain.current = chain.current.then(job).catch(() => undefined);
  };
  const stopOwnSpeech = useCallback(() => {
    if (!spoke.current) return;
    spoke.current = false;
    enqueue(() => commands.ttsCancel());
  }, []);
  useEffect(() => {
    if (!open) return;
    if (!voice) {
      stopOwnSpeech();
      return;
    }
    spoke.current = true;
    enqueue(() => commands.ttsSpeakText(caption));
  }, [open, voice, caption, stopOwnSpeech]);

  useEffect(() => {
    if (open) {
      setIndex(0);
      window.requestAnimationFrame(() => nextRef.current?.focus());
    }
  }, [open]);

  // Gültigkeit dieses Aufrufs: Esc, Überspringen und Aushängen verwerfen eine
  // noch ausstehende Geschichte (Codex lva#411 a).
  const alive = useRef(false);
  useEffect(() => {
    alive.current = open;
  }, [open]);

  const finish = useCallback(() => {
    alive.current = false;
    stopOwnSpeech();
    onClose();
    // Fokus zurück in die Eingabezeile (Codex lva#411 b).
    window.requestAnimationFrame(() =>
      document
        .querySelector<HTMLElement>("[data-testid='chat-input']")
        ?.focus(),
    );
  }, [onClose, stopOwnSpeech]);

  // Beim Aushängen (z. B. erste Nachricht im Chat) die eigene Ansage beenden.
  useEffect(
    () => () => {
      alive.current = false;
      stopOwnSpeech();
    },
    [stopOwnSpeech],
  );

  // Esc beendet die Führung – aber nicht, wenn gerade ein Menü, Popover oder
  // Dialog offen ist (die schließen zuerst) oder jemand die Taste schon behandelt hat.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (
        document.querySelector(
          '[role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"]',
        )
      )
        return;
      e.preventDefault();
      finish();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, finish]);

  const busy = useRef(false);
  const tellStory = async () => {
    if (busy.current) return;
    busy.current = true;
    const theme = topic.trim() || t("chatGuide.story.defaultTopic");
    const speakers = await loadSpeakers();
    if (!alive.current) return;
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
        <FluentButton appearance="subtle" onClick={() => finish()}>
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
