import React, {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Button as FluentButton } from "@fluentui/react-components";
import { type LucideIcon, Check, Cog, LayoutGrid, Mic } from "lucide-react";
import { useIntroTourStore } from "@/stores/introTourStore";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { commands } from "@/bindings";
import { retryFirstRunDownload } from "./firstRunSetup";
import "./introTour.css";

type StepId = "welcome" | "dictate" | "try" | "pages" | "tools" | "end";

interface Step {
  id: StepId;
  /** Dialog in der Mitte oder Spotlight auf dem Element mit `data-tour`. */
  kind: "dialog" | "spot";
  target?: string;
  icon: LucideIcon;
}

/** Schwerpunkt Diktieren und Üben; der Rest der App in zwei Spotlights. */
export const TOUR_STEPS: Step[] = [
  { id: "welcome", kind: "dialog", icon: Mic },
  { id: "dictate", kind: "dialog", icon: Mic },
  { id: "try", kind: "dialog", icon: Mic },
  { id: "pages", kind: "spot", target: "nav-primary", icon: LayoutGrid },
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

/** Systemeinstellung „Bewegung reduzieren“, auch wenn sie sich ändert. */
const useReducedMotion = () => {
  const query = useMemo(
    () =>
      typeof window !== "undefined" && window.matchMedia
        ? window.matchMedia("(prefers-reduced-motion: reduce)")
        : null,
    [],
  );
  const [reduced, setReduced] = useState(!!query?.matches);
  useEffect(() => {
    if (!query) return;
    const onChange = () => setReduced(query.matches);
    query.addEventListener?.("change", onChange);
    return () => query.removeEventListener?.("change", onChange);
  }, [query]);
  return reduced;
};

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
  return {
    name: model?.name ?? null,
    ready: !!model?.is_downloaded && !busy,
    percent: Math.max(0, Math.min(100, Math.round(progress ?? 0))),
  };
};

/** Drei Durchläufe Tasten, Pegel und Tippen, dann Endzustand. */
const DictationAnimation: React.FC<{
  keys: string[];
  sample: string;
  reduced: boolean;
}> = ({ keys, sample, reduced }) => {
  const [phase, setPhase] = useState<"idle" | "speak" | "type">("type");
  const [typed, setTyped] = useState(sample);

  useEffect(() => {
    if (reduced) {
      setPhase("type");
      setTyped(sample);
      return;
    }
    const timers: number[] = [];
    const run = (round: number) => {
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
          if (round < 3)
            timers.push(
              window.setTimeout(
                () => run(round + 1),
                sample.length * 28 + 2400,
              ),
            );
        }, 2800),
      );
    };
    run(1);
    return () => timers.forEach(window.clearTimeout);
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
        {phase !== "type" || typed.length < sample.length ? (
          <span className="lv-tour-anim__caret" />
        ) : null}
      </div>
    </div>
  );
};

/** Sinnvolles Ziel nach dem Schließen, wenn der Auslöser nicht mehr da ist. */
const FOCUS_FALLBACKS = [
  "[data-testid='workspace-start-tour']",
  "[data-titlebar-help]",
  "[data-tour='nav-home']",
];
const focusFallback = () =>
  FOCUS_FALLBACKS.map((q) => document.querySelector<HTMLElement>(q)).find(
    (el): el is HTMLElement => !!el,
  ) ?? null;

export const IntroTour: React.FC = () => {
  const { t, i18n } = useTranslation();
  const open = useIntroTourStore((s) => s.open);
  const close = useIntroTourStore((s) => s.close);
  const pendingModelId = useIntroTourStore((s) => s.pendingModelId);
  const failedModelId = useIntroTourStore((s) => s.failedModelId);
  const download = useDownloadState(pendingModelId);
  const currentModel = useModelStore((s) => s.currentModel);
  const { getSetting } = useSettings();
  const bindings = getSetting("bindings");
  const binding = bindings?.transcribe?.current_binding ?? "";
  const cancelBinding = bindings?.cancel?.current_binding ?? "escape";
  const pushToTalk = getSetting("push_to_talk") ?? true;
  const reduced = useReducedMotion();
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef<HTMLElement | null>(null);
  const titleId = useId();
  const bodyId = useId();

  const shortcut = binding
    ? localizeKeys(formatKeyCombination(binding, "unknown"), i18n.language)
    : "";
  const keys = shortcut ? shortcut.split(" + ") : [];
  const cancelKey = localizeKeys(
    formatKeyCombination(cancelBinding, "unknown"),
    i18n.language,
  );

  // Download beim ersten Start: Die Tour beginnt erst, wenn das Modell da ist.
  // Bereit heißt: geladen UND als Diktatmodell ausgewählt.
  const selected = !!pendingModelId && currentModel === pendingModelId;
  const loading = !!pendingModelId && !selected;
  const failed = !!failedModelId;
  const dictationReady = !loading && !failed && !!currentModel;

  const step = TOUR_STEPS[index];
  const target = step.kind === "spot" ? findTarget(step.target) : null;
  const mode = target ? "spot" : "dialog";
  const last = index === TOUR_STEPS.length - 1;
  const canAdvance = index !== 0 || (!loading && !failed);

  useEffect(() => {
    if (open) {
      restoreFocus.current = document.activeElement as HTMLElement | null;
      setIndex(0);
    }
  }, [open]);

  const finish = useCallback(() => {
    // Fertig eingerichtet: spätere Touren zeigen keinen Fortschritt mehr.
    if (selected) useIntroTourStore.getState().setPendingModel(null);
    close();
    const back = restoreFocus.current;
    window.requestAnimationFrame(() => {
      const target =
        back && back.isConnected && back !== document.body
          ? back
          : focusFallback();
      target?.focus?.();
    });
  }, [close, selected]);

  const next = useCallback(() => {
    if (!canAdvance) return;
    if (last) finish();
    else setIndex((i) => i + 1);
  }, [canAdvance, last, finish]);
  const back = useCallback(() => setIndex((i) => Math.max(0, i - 1)), []);

  // Spotlight-Rahmen am Ziel ausrichten, auch bei Scrollen und Größenänderung.
  useLayoutEffect(() => {
    if (!open || !target) {
      setRect(null);
      return;
    }
    target.scrollIntoView?.({ block: "nearest" });
    const update = () => setRect(target.getBoundingClientRect());
    update();
    const observer =
      typeof ResizeObserver !== "undefined" ? new ResizeObserver(update) : null;
    observer?.observe(target);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open, target]);

  // Fokus in den Schritt: Übungsfeld, sonst Hauptknopf, sonst „Wiederholen“,
  // sonst Dialog selbst. Wird ein Knopf erst später bedienbar (Modell bereit,
  // Download gescheitert), folgt der Fokus nur, wenn er gerade nirgends steht.
  const focusStep = useCallback((force: boolean) => {
    const box = boxRef.current;
    if (!box) return;
    const active = document.activeElement;
    const idle =
      !active ||
      active === document.body ||
      active === box ||
      !box.contains(active);
    if (!force && !idle) return;
    const pick = (selector: string) => box.querySelector<HTMLElement>(selector);
    (
      pick("textarea:not([disabled])") ??
      pick("[data-tour-primary]:not([disabled])") ??
      pick("[data-tour-retry]") ??
      box
    ).focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    if (!open) return;
    const id = window.requestAnimationFrame(() => focusStep(true));
    return () => window.cancelAnimationFrame(id);
  }, [open, index, focusStep]);

  useEffect(() => {
    if (!open) return;
    const id = window.requestAnimationFrame(() => focusStep(false));
    return () => window.cancelAnimationFrame(id);
  }, [open, canAdvance, dictationReady, failed, focusStep]);

  // Rest der App für Hilfsmittel und Tastatur stilllegen (echtes Modal).
  // Ausnahme: die Fensterknöpfe der Titelleiste (schließen, minimieren).
  useEffect(() => {
    if (!open) return;
    const tour = document.querySelector<HTMLElement>(".lv-tour");
    const muted: HTMLElement[] = [];
    const mute = (el: Element) => {
      if (!(el instanceof HTMLElement) || el === tour || el.inert) return;
      if (el.querySelector(".lv-window-controls")) {
        Array.from(el.children).forEach(mute);
        return;
      }
      if (el.classList.contains("lv-window-controls")) return;
      el.inert = true;
      muted.push(el);
    };
    Array.from(tour?.parentElement?.children ?? []).forEach(mute);
    return () => muted.forEach((el) => (el.inert = false));
  }, [open]);

  // Tastatur: Esc beendet die Tour (nicht im Übungsfeld, dort bricht Esc das
  // Diktat ab). Pfeile nur, wenn kein Bedienelement den Fokus hat. Tab bleibt
  // im Dialog.
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      const box = boxRef.current;
      const origin = e.target as Node | null;
      const onBody =
        !origin ||
        origin === document.body ||
        origin === document.documentElement;
      const onBox = !!box && origin === box;
      const inBox = !!box && !!origin && box.contains(origin);
      // Tab immer in der Tour halten: Fluents Tabster schiebt den Fokus beim
      // Drücken von Umschalt teils auf eigene Hilfselemente außerhalb.
      if (e.key === "Tab" && box) {
        const items = Array.from(
          box.querySelectorAll<HTMLElement>(
            "button:not([disabled]), textarea:not([disabled])",
          ),
        );
        e.preventDefault();
        e.stopPropagation();
        if (!items.length) {
          box.focus();
          return;
        }
        const active = document.activeElement as HTMLElement | null;
        const at = active ? items.indexOf(active) : -1;
        const nextIndex =
          at < 0
            ? e.shiftKey
              ? items.length - 1
              : 0
            : (at + (e.shiftKey ? -1 : 1) + items.length) % items.length;
        items[nextIndex].focus();
        return;
      }
      if (!onBody && !inBox) return;
      if (e.key === "Escape") {
        // Während einer Aufnahme bricht Esc zuerst die Aufnahme ab (globales
        // Kürzel im Backend); erst außerhalb einer Aufnahme schließt Esc die
        // Tour.
        e.preventDefault();
        void commands
          .isRecording()
          .catch(() => false)
          .then((recording) => {
            if (!recording) finish();
          });
      } else if (e.key === "ArrowRight" && (onBody || onBox)) {
        e.preventDefault();
        next();
      } else if (e.key === "ArrowLeft" && (onBody || onBox)) {
        e.preventDefault();
        back();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open, next, back, finish]);

  if (!open) return null;

  const modelName = download.name ?? t("introTour.download.fallbackName");
  const downloadBlock =
    pendingModelId || failed ? (
      <div
        className="lv-tour__progress"
        data-state={failed ? "failed" : loading ? "loading" : "ready"}
      >
        {failed ? (
          <>
            <span className="lv-tour__error">
              {t("introTour.download.failedShort")}
            </span>
            <span className="lv-tour__actions">
              <FluentButton
                appearance="secondary"
                size="small"
                onClick={retryFirstRunDownload}
                data-tour-retry=""
              >
                {t("introTour.download.retry")}
              </FluentButton>
              <FluentButton
                appearance="transparent"
                size="small"
                onClick={() => useIntroTourStore.getState().requestPicker(true)}
              >
                {t("introTour.download.choose")}
              </FluentButton>
            </span>
          </>
        ) : loading ? (
          <>
            <span aria-hidden="true">
              {t("introTour.download.loading", {
                model: modelName,
                percent: download.percent,
              })}
            </span>
            <span
              className="lv-tour__bar"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={download.percent}
              aria-valuetext={t("introTour.download.loading", {
                model: modelName,
                percent: download.percent,
              })}
              aria-label={t("introTour.download.label")}
            >
              <i style={{ width: `${download.percent}%` }} />
            </span>
          </>
        ) : (
          <span>{t("introTour.download.ready", { model: modelName })}</span>
        )}
      </div>
    ) : null;

  // Gezielte Ansagen: Bereit und Fehler (der Fortschritt selbst ist eine
  // progressbar und wird nicht bei jedem Prozent angesagt).
  const announcement = failed
    ? t("introTour.download.failedShort")
    : selected
      ? t("introTour.download.ready", { model: modelName })
      : "";

  const body = (() => {
    switch (step.id) {
      case "welcome":
        return (
          <>
            <p>
              {dictationReady
                ? t("introTour.welcome.bodyReady")
                : t("introTour.welcome.body")}
            </p>
            {downloadBlock}
            {loading && (
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
                  : t("introTour.dictate.threeToggle", { shortcut })}
              </li>
            </ol>
            <p className="lv-tour__note">
              {t("introTour.dictate.note", { cancel: cancelKey })}
            </p>
          </>
        );
      case "try": {
        const fieldId = `${bodyId}-try`;
        const hintId = `${bodyId}-try-hint`;
        return (
          <>
            <p>
              {pushToTalk
                ? t("introTour.try.bodyHold", { shortcut })
                : t("introTour.try.bodyToggle", { shortcut })}
            </p>
            <label className="lv-tour__label" htmlFor={fieldId}>
              {t("introTour.try.field")}
            </label>
            <textarea
              id={fieldId}
              className="lv-tour__try"
              rows={3}
              disabled={!dictationReady}
              aria-describedby={hintId}
              placeholder={t("introTour.try.placeholder")}
            />
            <p id={hintId} className="lv-tour__note">
              {dictationReady
                ? t("introTour.try.ready", { cancel: cancelKey })
                : t("introTour.try.locked")}
            </p>
          </>
        );
      }
      case "pages":
        return (
          <ul className="lv-tour__list">
            <li>{t("introTour.pages.history")}</li>
            <li>{t("introTour.pages.meetings")}</li>
            <li>{t("introTour.pages.tts")}</li>
            <li>{t("introTour.pages.voice")}</li>
          </ul>
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
            {loading ? (
              <p className="lv-tour__note">{t("introTour.end.stillLoading")}</p>
            ) : null}
          </>
        );
    }
  })();

  const Icon = step.icon;
  const art =
    step.id === "dictate" ? (
      <DictationAnimation
        keys={keys}
        sample={t("introTour.dictate.sample")}
        reduced={reduced}
      />
    ) : (
      <div className="lv-tour__icon" data-done={step.id === "end" || undefined}>
        <Icon size={40} aria-hidden={true} />
      </div>
    );

  const stepLabel = t("introTour.stepOf", {
    step: index + 1,
    total: TOUR_STEPS.length,
  });

  const content = (
    <>
      <div className="lv-tour__eyebrow" aria-hidden="true">
        {t(`introTour.${step.id}.eyebrow`)} · {stepLabel}
      </div>
      <h2 className="lv-tour__title" id={titleId}>
        {t(`introTour.${step.id}.title`)}
      </h2>
      <div className="lv-tour__body" id={bodyId}>
        {body}
      </div>
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
        <FluentButton
          appearance="primary"
          onClick={next}
          disabled={!canAdvance}
          data-tour-primary=""
        >
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
  // unteren Hälfte richten die Blase an ihrer Unterkante aus.
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
        popStyle.maxHeight = vh - bottom - 16;
        popStyle["--lv-tour-arrow"] =
          `${Math.max(16, vh - mid - bottom - 6)}px`;
      } else {
        const top = Math.max(16, Math.min(mid - 40, rect.top));
        popStyle.top = top;
        popStyle.maxHeight = vh - top - 16;
        popStyle["--lv-tour-arrow"] = `${Math.max(16, mid - top - 6)}px`;
      }
    } else {
      arrow = "top";
      const left = Math.max(16, Math.min(rect.left, vw - width - 16));
      const top = Math.min(rect.bottom + 16, vh - 160);
      popStyle.left = left;
      popStyle.top = top;
      popStyle.maxHeight = vh - top - 16;
      popStyle["--lv-tour-arrow"] =
        `${Math.max(16, rect.left + rect.width / 2 - left - 6)}px`;
    }
  }

  return (
    <div
      className="lv-tour"
      data-mode={mode}
      data-reduced={reduced || undefined}
      data-testid="intro-tour"
    >
      {/* Schrittwechsel und Bereitschaft für Screenreader */}
      <div className="lv-tour__sr" role="status" aria-live="polite">
        {`${stepLabel}: ${t(`introTour.${step.id}.title`)}`}
      </div>
      <div className="lv-tour__sr" role="alert" aria-live="assertive">
        {announcement}
      </div>
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
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        tabIndex={-1}
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
