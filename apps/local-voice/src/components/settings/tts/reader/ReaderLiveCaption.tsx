import React from "react";
import { useTranslation } from "react-i18next";
import { Languages, X } from "lucide-react";
import type { LiveState, LiveUnit } from "./useReaderLive";
import { unitAt } from "./useReaderLive";

/** Zielsprachen der Live-Übersetzung (wie in der iOS-App). */
export const LIVE_LANGS = [
  "de",
  "en",
  "fr",
  "es",
  "it",
  "pt",
  "nl",
  "pl",
  "sv",
  "da",
  "no",
  "fi",
  "cs",
  "tr",
  "uk",
  "ru",
  "ar",
  "hi",
  "id",
  "th",
  "vi",
  "zh",
  "ja",
  "ko",
];

/** Name einer Sprache in der Sprache der Oberfläche („en“ → „Englisch“). */
export function liveLangName(code: string, ui: string): string {
  try {
    const name = new Intl.DisplayNames([ui || "de"], { type: "language" }).of(
      code,
    );
    return name ? name.charAt(0).toLocaleUpperCase(ui) + name.slice(1) : code;
  } catch {
    return code;
  }
}

/**
 * Untertitel der Live-Übersetzung über der Wiedergabeleiste (lva#782 A2a):
 * die Übersetzung des Satzes an der Lesestelle, darunter verständliche
 * Zustände (übersetzt gerade, kein lokales Modell, fertige Übersetzung
 * vorhanden).
 */
export const ReaderLiveCaption: React.FC<{
  target: string;
  state: LiveState;
  units: LiveUnit[];
  offset: number;
  onOpenExisting(): void;
  onClose(): void;
}> = ({ target, state, units, offset, onOpenExisting, onClose }) => {
  const { t, i18n } = useTranslation();
  const lang = liveLangName(target, i18n.language);
  const unit = unitAt(units, offset);
  let body: React.ReactNode;
  if (state.kind === "existing")
    body = (
      <span className="reader-live__hint">
        {t("tts.reader.live.existing", { lang })}{" "}
        <button
          type="button"
          className="reader-live__link"
          onClick={onOpenExisting}
          data-testid="reader-live-open-existing"
        >
          {t("tts.reader.live.openExisting")}
        </button>
      </span>
    );
  else if (state.kind === "error")
    body = (
      <span className="reader-live__hint reader-live__hint--error">
        {t(`tts.reader.live.error.${state.code}`, {
          defaultValue: t("tts.reader.live.error.failed"),
        })}
      </span>
    );
  else if (unit)
    body = (
      <span lang={target} className="reader-live__text">
        {unit.translation}
      </span>
    );
  else
    body = (
      <span className="reader-live__hint" aria-busy="true">
        {t("tts.reader.live.waiting")}
      </span>
    );
  return (
    <div
      className="reader-live"
      role="region"
      aria-label={t("tts.reader.live.region", { lang })}
      aria-live="polite"
      data-testid="reader-live"
    >
      <span className="reader-live__badge">
        <Languages width={14} height={14} aria-hidden="true" />
        {t("tts.reader.live.badge", { lang })}
      </span>
      <div className="reader-live__body">{body}</div>
      <button
        type="button"
        className="reader-live__close"
        aria-label={t("tts.reader.live.stop")}
        title={t("tts.reader.live.stop")}
        onClick={onClose}
        data-testid="reader-live-stop"
      >
        <X width={14} height={14} aria-hidden="true" />
      </button>
    </div>
  );
};
