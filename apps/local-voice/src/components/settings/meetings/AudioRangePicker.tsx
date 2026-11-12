import React, {
  useEffect,
  useId,
  useImperativeHandle,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  RANGE_STEP_MS,
  formatRangeTime,
  normalizeRange,
  parseRangeTime,
  type RangeRounding,
  rangeLength,
  type MsRange,
} from "@/lib/meetingRange";
import { Input } from "../../ui/Input";

interface AudioRangePickerProps {
  /** Dauer der Aufnahme in ms: die Grenzen des Reglers. */
  durationMs: number;
  value: MsRange;
  onChange: (value: MsRange) => void;
  /** Vorsilbe der Test-IDs (`<id>-start`, `<id>-end`, `<id>-slider-start` …). */
  testId?: string;
  disabled?: boolean;
  /** i18n-Schlüssel der Längenzeile (`{{length}}`, `{{total}}`); Standard: Bereich von Gesamt. */
  lengthKey?: string;
  /**
   * Meldet, ob beide Felder lesbar sind (Ausführen-Knopf sperren, solange nicht) und ob ein
   * getippter Text noch nicht bestätigt ist (`pending`: der Bereich kann sich beim Klick noch ändern).
   */
  onValidityChange?: (valid: boolean, pending: boolean) => void;
  /** Griff zum erneuten Prüfen der Feldtexte im Moment des Klicks. */
  handleRef?: React.Ref<AudioRangePickerHandle>;
}

export interface AudioRangePickerHandle {
  /**
   * Liest die Texte beider Felder JETZT (auch unbestätigte) und gibt den gültigen
   * Bereich zurück, den die Felder zeigen; `null`, wenn ein Feld unlesbar ist (dann
   * darf nichts ausgeführt werden und es gilt kein älterer Wert).
   */
  validate: () => MsRange | null;
}

interface TimeFieldProps {
  label: string;
  valueMs: number;
  /** Nimmt die Eingabe entgegen und gibt den Wert zurück, der danach gilt (geklemmt). */
  onCommit: (ms: number) => number;
  testId: string;
  disabled?: boolean;
  /** Nachkommastellen: „Von“ rundet ab, „Bis“ auf (wie `parseRangeTime`). */
  rounding: RangeRounding;
  /** Meldet jeden Feldtext (auch unbestätigt) an den Wähler. */
  onDraft: (draft: string) => void;
}

/**
 * Zeitfeld: man tippt `1:05`, `1:02:30` oder `90` und bestätigt mit Enter oder dem
 * Verlassen des Feldes. Unlesbares wird zurückgesetzt und gemeldet; solange man
 * tippt, springt nichts.
 */
const TimeField: React.FC<TimeFieldProps> = ({
  label,
  valueMs,
  onCommit,
  testId,
  disabled,
  rounding,
  onDraft,
}) => {
  const { t } = useTranslation();
  const errorId = useId();
  const [draft, setDraft] = useState(formatRangeTime(valueMs));
  const [invalid, setInvalid] = useState(false);

  const onDraftRef = useRef(onDraft);
  onDraftRef.current = onDraft;
  useEffect(() => {
    onDraftRef.current(draft);
  }, [draft]);

  // Von außen geänderter Wert (Regler, anderes Feld, Zurücksetzen).
  useEffect(() => {
    setDraft(formatRangeTime(valueMs));
    setInvalid(false);
  }, [valueMs]);

  const commit = () => {
    // Unverändert (nur das Feld verlassen): der Wert bleibt, auch wenn er
    // Bruchteile einer Sekunde hat, die die Anzeige abschneidet.
    if (draft === formatRangeTime(valueMs)) {
      setInvalid(false);
      return;
    }
    const parsed = parseRangeTime(draft, rounding);
    if (parsed === null) {
      // Der unlesbare Text bleibt stehen (mit Meldung), kein stiller Rückgriff auf
      // den älteren Wert: der Wähler meldet das Feld als ungültig.
      setInvalid(true);
      return;
    }
    setInvalid(false);
    // Die Grenzprüfung kann den Wert ändern (hinter dem Ende, vor 0:00): das Feld zeigt,
    // was gilt, auch wenn sich der Wert von außen nicht ändert.
    setDraft(formatRangeTime(onCommit(parsed)));
  };

  return (
    <label className="block space-y-1">
      <span className="block text-xs font-medium text-text/60">{label}</span>
      <Input
        variant="compact"
        className="w-28"
        data-testid={testId}
        inputMode="numeric"
        autoComplete="off"
        spellCheck={false}
        value={draft}
        disabled={disabled}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errorId : undefined}
        onChange={(e) => {
          setDraft(e.target.value);
          setInvalid(false);
        }}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit();
          }
        }}
      />
      {invalid && (
        <span
          id={errorId}
          role="alert"
          data-testid={`${testId}-error`}
          className="block text-xs text-red-400"
        >
          {t("meetings.range.invalidTime")}
        </span>
      )}
    </label>
  );
};

/**
 * Bereich einer Aufnahme wählen (lva#522): Von/Bis als Zeitfelder und darunter ein
 * Zweifach-Schieberegler über die Aufnahmedauer. Beides hält denselben Bereich;
 * die Grenzen bleiben in der Aufnahme und mindestens eine Sekunde auseinander.
 * Wiederverwendet von „Neu transkribieren" (Bereich) und „Aufnahme zuschneiden".
 */
export const AudioRangePicker: React.FC<AudioRangePickerProps> = ({
  durationMs,
  value,
  onChange,
  testId = "range",
  disabled = false,
  lengthKey = "meetings.range.length",
  onValidityChange,
  handleRef,
}) => {
  const { t } = useTranslation();
  const drafts = useRef({
    start: formatRangeTime(value.start_ms),
    end: formatRangeTime(value.end_ms),
  });
  const [texts, setTexts] = useState(drafts.current);
  const report = (which: "start" | "end") => (draft: string) => {
    drafts.current = { ...drafts.current, [which]: draft };
    setTexts(drafts.current);
  };
  const valid =
    parseRangeTime(texts.start, "floor") !== null &&
    parseRangeTime(texts.end, "ceil") !== null;
  const pending =
    texts.start !== formatRangeTime(value.start_ms) ||
    texts.end !== formatRangeTime(value.end_ms);
  const onValidityRef = useRef(onValidityChange);
  onValidityRef.current = onValidityChange;
  useEffect(() => {
    onValidityRef.current?.(valid, pending);
  }, [valid, pending]);
  // Der Regler rechnet in ganzen Sekunden; das Maximum ist die aufgerundete Dauer.
  const max = Math.max(
    RANGE_STEP_MS,
    Math.ceil(durationMs / RANGE_STEP_MS) * RANGE_STEP_MS,
  );
  const percent = (ms: number) => (max > 0 ? (ms / max) * 100 : 0);
  const set = (next: MsRange, moved: "start" | "end"): MsRange => {
    const normalized = normalizeRange(next, durationMs, moved);
    onChange(normalized);
    return normalized;
  };
  useImperativeHandle(
    handleRef,
    () => ({
      validate: () => {
        const start = parseRangeTime(drafts.current.start, "floor");
        const end = parseRangeTime(drafts.current.end, "ceil");
        if (start === null || end === null) return null;
        // Ein unveränderter Text lässt den Wert stehen (Bruchteile der Sekunde bleiben).
        const startSame =
          drafts.current.start === formatRangeTime(value.start_ms);
        const endSame = drafts.current.end === formatRangeTime(value.end_ms);
        const next = {
          start_ms: startSame ? value.start_ms : start,
          end_ms: endSame ? value.end_ms : end,
        };
        const normalized = normalizeRange(
          next,
          durationMs,
          startSame ? "end" : "start",
        );
        if (
          normalized.start_ms !== value.start_ms ||
          normalized.end_ms !== value.end_ms
        ) {
          onChange(normalized);
        }
        return normalized;
      },
    }),
    [value, durationMs, onChange],
  );
  // Der Griff, der gerade weiter innen liegt, bleibt oben, damit beide greifbar sind.
  const startOnTop = value.start_ms > max / 2;

  return (
    <div className="space-y-3" data-testid={testId}>
      <div className="flex flex-wrap items-start gap-x-4 gap-y-2">
        <TimeField
          label={t("meetings.range.from")}
          valueMs={value.start_ms}
          disabled={disabled}
          testId={`${testId}-start`}
          rounding="floor"
          onDraft={report("start")}
          onCommit={(ms) => set({ ...value, start_ms: ms }, "start").start_ms}
        />
        <TimeField
          label={t("meetings.range.to")}
          valueMs={value.end_ms}
          disabled={disabled}
          testId={`${testId}-end`}
          rounding="ceil"
          onDraft={report("end")}
          onCommit={(ms) => set({ ...value, end_ms: ms }, "end").end_ms}
        />
      </div>
      <div
        role="group"
        aria-label={t("meetings.range.slider")}
        className="lv-range2"
        style={
          {
            "--_a": percent(value.start_ms) / 100,
            "--_b": percent(value.end_ms) / 100,
          } as React.CSSProperties
        }
      >
        <span className="lv-range2__rail" aria-hidden="true" />
        <span className="lv-range2__fill" aria-hidden="true" />
        <input
          type="range"
          className="lv-range2__input"
          data-testid={`${testId}-slider-start`}
          aria-label={t("meetings.range.startAria")}
          aria-valuetext={formatRangeTime(value.start_ms)}
          min={0}
          max={max}
          step={RANGE_STEP_MS}
          value={Math.min(value.start_ms, max)}
          disabled={disabled}
          style={{ zIndex: startOnTop ? 3 : 2 }}
          onChange={(e) =>
            set({ ...value, start_ms: Number(e.target.value) }, "start")
          }
        />
        <input
          type="range"
          className="lv-range2__input"
          data-testid={`${testId}-slider-end`}
          aria-label={t("meetings.range.endAria")}
          aria-valuetext={formatRangeTime(value.end_ms)}
          min={0}
          max={max}
          step={RANGE_STEP_MS}
          value={Math.min(value.end_ms, max)}
          disabled={disabled}
          style={{ zIndex: startOnTop ? 2 : 3 }}
          onChange={(e) =>
            set({ ...value, end_ms: Number(e.target.value) }, "end")
          }
        />
      </div>
      <p
        className="text-xs text-text/60"
        data-testid={`${testId}-length`}
        aria-live="polite"
      >
        {t(lengthKey, {
          length: formatRangeTime(rangeLength(value)),
          total: formatRangeTime(durationMs),
        })}
      </p>
    </div>
  );
};
