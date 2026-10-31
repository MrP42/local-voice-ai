import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Button as FluentButton } from "@fluentui/react-components";
import {
  type LucideIcon,
  Check,
  Cog,
  History,
  Mic,
  Phone,
  Users,
  Volume2,
} from "lucide-react";
import { useIntroTourStore } from "@/stores/introTourStore";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import "./introTour.css";

type StepId =
  | "welcome"
  | "dictate"
  | "try"
  | "history"
  | "meetings"
  | "tts"
  | "voice"
  | "tools"
  | "end";

interface Step {
  id: StepId;
  /** Dialog in der Mitte oder Spotlight auf dem Element mit `data-tour`. */
  kind: "dialog" | "spot";
  target?: string;
  icon: LucideIcon;
}

export const TOUR_STEPS: Step[] = [
  { id: "welcome", kind: "dialog", icon: Mic },
  { id: "dictate", kind: "dialog", icon: Mic },
  { id: "try", kind: "dialog", icon: Mic },
  { id: "history", kind: "spot", target: "nav-history", icon: History },
  { id: "meetings", kind: "spot", target: "nav-meetings", icon: Users },
  { id: "tts", kind: "spot", target: "nav-tts", icon: Volume2 },
  { id: "voice", kind: "spot", target: "nav-voice", icon: Phone },
  { id: "tools", kind: "spot", target: "nav-utilities", icon: Cog },
  { id: "end", kind: "dialog", icon: Check },
];

/** Tastennamen wie auf deutschen Tastaturen beschriftet. */
const DE_KEYS: Record<string, string> = {
  Ctrl: "Strg",
  Space: "Leertaste",
  Shift: "Umschalt",
  Escape: "Esc",
};
const localizeKeys = (combo: string, language: string) =>
  language.startsWith("de")
    ? combo
        .split(" + ")
        .map((k) => DE_KEYS[k] ?? k)
        .join(" + ")
    : combo;

const prefersReducedMotion = () =>
  typeof window !== "undefined" &&
  !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/** Sichtbares Ziel des Spotlights, sonst `null` (dann zeigt der Schritt einen Dialog). */
const findTarget = (target?: string): HTMLElement | null => {
  if (!target) return null;
  const el = document.querySelector<HTMLElement>(`[data-tour="${target}"]`);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0 ? el : null;
};

const useDownloadState = (modelId: string | null) => {
  const model = useModelStore((s) =>
    modelId ? s.models.find((m) => m.id === modelId) : undefined,
  );
  const progress = useModelStore((s) =>
    modelId ? s.downloadProgress[modelId]?.percentage : undefined,
  );
  const busy = useModelStore((s) =>
    modelId
      ? modelId in s.downloadingModels ||
        modelId in s.verifyingModels ||
        modelId in s.extractingModels
      : false,
  );
  const current = useModelStore((s) => s.currentModel);
  return {
    name: model?.name ?? null,
    ready: !!model?.is_downloaded && !busy,
    selected: !!modelId && current === modelId,
    percent: Math.max(0, Math.min(100, Math.round(progress ?? 0))),
  };
};

/** Tasten, Pegel und Tippen in einer Endlosschleife — still bei reduzierter Bewegung. */
const DictationAnimation: React.FC<{ keys: string[]; sample: string }> = ({
  keys,
  sample,
}) => {
  const reduced = prefersReducedMotion();
  const [phase, setPhase] = useState<"idle" | "speak" | "type">(
    reduced ? "type" : "idle",
  );
  const [typed, setTyped] = useState(reduced ? sample : "");

  useEffect(() => {
    if (reduced) return;
    const timers: number[] = [];
    const run = () => {
      setPhase("idle");
      setTyped("");
      timers.push(window.setTimeout(() => setPhase("speak"), 600));
      timers.push(
        window.setTimeout(() => {
          setPhase("type");
          for (let i = 1; i <= sample.length; i++)
            timers.push(
              window.setTimeout(() => setTyped(sample.slice(0, i)), i * 28),
            );
        }, 2800),
      );
    };
    run();
    const loop = window.setInterval(run, 6800);
    return () => {
      window.clearInterval(loop);
      timers.forEach(window.clearTimeout);
    };
  }, [reduced, sample]);

  return (
    <div className="lv-tour-anim" data-phase={phase} aria-hidden="true">
      <div className="lv-tour-anim__keys">
        {keys.map((k) => (
          <span key={k} className="lv-tour-anim__key">
            {k}
          </span>
        ))}
      </div>
      <div className="lv-tour-anim__wave">
        {Array.from({ length: 18 }, (_, i) => (
          <i key={i} style={{ animationDelay: `${(i % 6) * 90}ms` }} />
        ))}
      </div>
      <div className="lv-tour-anim__field">
        {typed}
        <span className="lv-tour-anim__caret" />
      </div>
    </div>
  );
};

export const IntroTour: React.FC = () => {
  const { t, i18n } = useTranslation();
  const open = useIntroTourStore((s) => s.open);
  const close = useIntroTourStore((s) => s.close);
  const pendingModelId = useIntroTourStore((s) => s.pendingModelId);
  const download = useDownloadState(pendingModelId);
  const { getSetting } = useSettings();
  const binding = getSetting("bindings")?.transcribe?.current_binding ?? "";
  const pushToTalk = getSetting("push_to_talk") ?? true;
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef<HTMLElement | null>(null);

  const shortcut = binding
    ? localizeKeys(formatKeyCombination(binding, "unknown"), i18n.language)
    : "";
  const keys = shortcut ? shortcut.split(" + ") : [];

  const step = TOUR_STEPS[index];
  const target = step.kind === "spot" ? findTarget(step.target) : null;
  const mode = target ? "spot" : "dialog";
  const last = index === TOUR_STEPS.length - 1;

  useEffect(() => {
    if (open) {
      restoreFocus.current = document.activeElement as HTMLElement | null;
      setIndex(0);
    }
  }, [open]);

  const finish = useCallback(() => {
    close();
    restoreFocus.current?.focus?.();
  }, [close]);

  const next = useCallback(
    () => (last ? finish() : setIndex((i) => i + 1)),
    [last, finish],
  );
  const back = useCallback(() => setIndex((i) => Math.max(0, i - 1)), []);

  // Spotlight-Rahmen am Ziel ausrichten, auch bei Größenänderung.
  useLayoutEffect(() => {
    if (!open || !target) {
      setRect(null);
      return;
    }
    const update = () => setRect(target.getBoundingClientRect());
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [open, target]);

  // Fokus in den Schritt: Hauptknopf, damit Enter weiterführt.
  useEffect(() => {
    if (!open) return;
    const id = window.requestAnimationFrame(() => {
      boxRef.current
        ?.querySelector<HTMLElement>("[data-tour-primary]")
        ?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(id);
  }, [open, index]);

  // Tastatur am Dokument: Beim Schrittwechsel ist der Fokus kurz außerhalb
  // der Tour, schnelle Tastendrücke gingen sonst verloren.
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      const box = boxRef.current;
      // Im Übungsfeld gehören Esc (Diktat abbrechen) und Pfeile dem Text.
      const inField = !!(e.target as HTMLElement | null)?.closest?.("textarea");
      if (e.key === "Escape" && !inField) {
        e.preventDefault();
        finish();
      } else if (e.key === "ArrowRight" && !inField) {
        e.preventDefault();
        next();
      } else if (e.key === "ArrowLeft" && !inField) {
        e.preventDefault();
        back();
      } else if (e.key === "Tab" && box) {
        const items = Array.from(
          box.querySelectorAll<HTMLElement>(
            "button:not([disabled]), textarea, [tabindex='0']",
          ),
        );
        if (!items.length) return;
        const first = items[0];
        const lastItem = items[items.length - 1];
        const active = document.activeElement;
        if (!active || !box.contains(active)) {
          e.preventDefault();
          (e.shiftKey ? lastItem : first).focus();
        } else if (e.shiftKey && active === first) {
          e.preventDefault();
          lastItem.focus();
        } else if (!e.shiftKey && active === lastItem) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open, next, back, finish]);

  if (!open) return null;

  const progressLine = pendingModelId ? (
    <div
      className="lv-tour__progress"
      data-ready={download.ready || undefined}
      role="status"
    >
      <span>
        {download.ready
          ? t("introTour.download.ready", { model: download.name })
          : t("introTour.download.loading", {
              model: download.name ?? "Nemotron 3.5",
              percent: download.percent,
            })}
      </span>
      {!download.ready && (
        <span
          className="lv-tour__bar"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={download.percent}
          aria-label={t("introTour.download.label")}
        >
          <i style={{ width: `${download.percent}%` }} />
        </span>
      )}
    </div>
  ) : null;

  const body = (() => {
    switch (step.id) {
      case "welcome":
        return (
          <>
            <p>{t("introTour.welcome.body")}</p>
            {progressLine}
            {pendingModelId && (
              <p className="lv-tour__note">{t("introTour.welcome.note")}</p>
            )}
          </>
        );
      case "dictate":
        return (
          <>
            <ol className="lv-tour__list">
              <li>{t("introTour.dictate.one")}</li>
              <li>
                {pushToTalk
                  ? t("introTour.dictate.twoHold", { shortcut })
                  : t("introTour.dictate.twoToggle", { shortcut })}
              </li>
              <li>
                {pushToTalk
                  ? t("introTour.dictate.threeHold")
                  : t("introTour.dictate.threeToggle")}
              </li>
            </ol>
            <p className="lv-tour__note">{t("introTour.dictate.note")}</p>
          </>
        );
      case "try":
        return (
          <>
            <p>{t("introTour.try.body", { shortcut })}</p>
            <textarea
              className="lv-tour__try"
              rows={3}
              aria-label={t("introTour.try.field")}
              placeholder={t("introTour.try.placeholder")}
            />
            {pendingModelId && !download.ready ? progressLine : null}
          </>
        );
      case "tools":
        return (
          <ul className="lv-tour__list">
            <li>{t("introTour.tools.models")}</li>
            <li>{t("introTour.tools.integrations")}</li>
            <li>{t("introTour.tools.settings")}</li>
            <li>{t("introTour.tools.statusbar")}</li>
          </ul>
        );
      case "end":
        return (
          <>
            <p>{t("introTour.end.body")}</p>
            {pendingModelId && !download.ready ? (
              <p className="lv-tour__note">{t("introTour.end.stillLoading")}</p>
            ) : null}
          </>
        );
      default:
        return <p>{t(`introTour.${step.id}.body`)}</p>;
    }
  })();

  const Icon = step.icon;
  const art =
    step.id === "dictate" ? (
      <DictationAnimation keys={keys} sample={t("introTour.dictate.sample")} />
    ) : (
      <div className="lv-tour__icon" data-done={step.id === "end" || undefined}>
        <Icon size={40} aria-hidden={true} />
      </div>
    );

  const content = (
    <>
      <div className="lv-tour__eyebrow">
        {t(`introTour.${step.id}.eyebrow`)} ·{" "}
        {t("introTour.stepOf", { step: index + 1, total: TOUR_STEPS.length })}
      </div>
      <h2 className="lv-tour__title" id="lv-tour-title">
        {t(`introTour.${step.id}.title`)}
      </h2>
      <div className="lv-tour__body">{body}</div>
      <div className="lv-tour__nav">
        <span className="lv-tour__dots" aria-hidden="true">
          {TOUR_STEPS.map((s, i) => (
            <i key={s.id} data-on={i === index || undefined} />
          ))}
        </span>
        {!last && (
          <FluentButton appearance="transparent" onClick={finish}>
            {t("introTour.skip")}
          </FluentButton>
        )}
        {index > 0 && (
          <FluentButton appearance="secondary" onClick={back}>
            {t("introTour.back")}
          </FluentButton>
        )}
        <FluentButton appearance="primary" onClick={next} data-tour-primary="">
          {last
            ? t("introTour.finish")
            : index === 0
              ? t("introTour.start")
              : t("introTour.next")}
        </FluentButton>
      </div>
    </>
  );

  // Platz für die Sprechblase: rechts vom Ziel, sonst darunter. Ziele in der
  // unteren Hälfte (Werkzeuge) richten die Blase an ihrer Unterkante aus,
  // damit sie nicht aus dem Fenster ragt.
  const popStyle: React.CSSProperties & Record<string, string | number> = {};
  let arrow: "left" | "left-bottom" | "top" = "left";
  if (mode === "spot" && rect) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.min(380, vw - 32);
    const mid = rect.top + rect.height / 2;
    popStyle.width = width;
    if (rect.right + 16 + width <= vw - 16) {
      popStyle.left = rect.right + 16;
      if (mid > vh / 2) {
        arrow = "left-bottom";
        const bottom = Math.max(16, vh - rect.bottom);
        popStyle.bottom = bottom;
        popStyle["--lv-tour-arrow"] =
          `${Math.max(16, vh - mid - bottom - 6)}px`;
      } else {
        const top = Math.max(16, mid - 40);
        popStyle.top = top;
        popStyle["--lv-tour-arrow"] = `${Math.max(16, mid - top - 6)}px`;
      }
    } else {
      arrow = "top";
      const left = Math.max(16, Math.min(rect.left, vw - width - 16));
      popStyle.left = left;
      popStyle.top = rect.bottom + 16;
      popStyle["--lv-tour-arrow"] =
        `${Math.max(16, rect.left + rect.width / 2 - left - 6)}px`;
    }
  }

  return (
    <div className="lv-tour" data-mode={mode} data-testid="intro-tour">
      {mode === "dialog" ? (
        <div className="lv-tour__shade" />
      ) : rect ? (
        <div
          className="lv-tour__spot"
          style={{
            left: rect.left - 4,
            top: rect.top - 4,
            width: rect.width + 8,
            height: rect.height + 8,
          }}
        />
      ) : null}
      <div
        ref={boxRef}
        key={step.id}
        role="dialog"
        aria-modal="true"
        aria-labelledby="lv-tour-title"
        className={mode === "dialog" ? "lv-tour__dialog" : "lv-tour__pop"}
        data-arrow={mode === "spot" ? arrow : undefined}
        style={mode === "spot" ? popStyle : undefined}
      >
        {mode === "dialog" ? (
          <>
            <div className="lv-tour__art">{art}</div>
            <div className="lv-tour__text">{content}</div>
          </>
        ) : (
          content
        )}
      </div>
    </div>
  );
};

export default IntroTour;
