import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { save } from "@tauri-apps/plugin-dialog";
import {
  BarChart3,
  BookOpen,
  ChevronLeft,
  ChevronRight,
  Download,
  Flame,
  Headphones,
  Lock,
  Pause,
  Play,
  Sparkles,
  Table2,
  Trash2,
  Volume2,
} from "lucide-react";
import { commands, type LibraryBook, type ReadingRow } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useFunctionLlm } from "@/lib/functionLlm";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { BookCover } from "./BookCover";
import {
  audioProgressKey,
  loadProgress,
  textProgressKey,
} from "@/lib/library/progress";
import { CHANGED_EVENT, flushReadingLog } from "@/lib/library/readingClock";
import {
  categoryLabel,
  daysOf,
  formatDuration,
  FORMATS,
  goalState,
  localDay,
  minutesPerDay,
  parseDay,
  periodOf,
  previousPeriod,
  summarize,
  toCsv,
  type BookLengths,
  type Format,
  type Period,
  type PeriodKind,
} from "@/lib/library/readingStats";
import { classifyBook, needsClassification } from "@/lib/library/classify";
import "./readingStats.css";

/** Zeilen aller Geräte; lädt neu, wenn der Taktgeber gemeldet hat. */
export function useReadingRows(): {
  rows: ReadingRow[] | null;
  reload: () => void;
} {
  const [rows, setRows] = useState<ReadingRow[] | null>(null);
  const reload = useCallback(() => {
    void commands
      .readingLogRows()
      .then((r) => setRows(r.status === "ok" ? r.data : []))
      .catch(() => setRows([]));
  }, []);
  useEffect(() => {
    reload();
    const on = () => reload();
    window.addEventListener(CHANGED_EVENT, on);
    window.addEventListener("focus", on);
    return () => {
      window.removeEventListener(CHANGED_EVENT, on);
      window.removeEventListener("focus", on);
    };
  }, [reload]);
  return { rows, reload };
}

/** Textlänge und Hördauer eines Buchs aus dem Fortschritt (für die Seiten). */
export function lengthsOf(book: LibraryBook): BookLengths {
  const text = book.text
    ? loadProgress(textProgressKey(book.text.page_id))
    : null;
  const audio = book.audio ? loadProgress(audioProgressKey(book.id)) : null;
  const tracks = book.audio?.tracks ?? [];
  const fromTracks = tracks.reduce((a, t) => a + (t.duration_ms ?? 0), 0);
  return {
    textChars: text?.unit !== "ms" ? (text?.length ?? 0) : 0,
    audioMs: audio?.length || fromTracks || book.audio?.duration_ms || 0,
  };
}

const FORMAT_ICON: Record<Format, React.ElementType> = {
  lesen: BookOpen,
  hoeren: Headphones,
  vorlesen: Volume2,
};

export const Ring: React.FC<{
  value: number;
  goal: number;
  size?: number;
  stroke?: number;
  label?: boolean;
  unit?: string;
}> = ({ value, goal, size = 112, stroke = 10, label = true, unit }) => {
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const p = goal > 0 ? Math.min(1, value / goal) : 0;
  const done = goal > 0 && value >= goal;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden="true"
      className="rs-ring"
    >
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        className="rs-ring__track"
        strokeWidth={stroke}
      />
      {goal > 0 && (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          className={done ? "rs-ring__done" : "rs-ring__value"}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${c * p} ${c}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      )}
      {label && (
        <>
          <text x="50%" y="47%" textAnchor="middle" className="rs-ring__n">
            {Math.round(value)}
          </text>
          <text x="50%" y="64%" textAnchor="middle" className="rs-ring__u">
            {unit}
          </text>
        </>
      )}
    </svg>
  );
};

/** Knopf in der Bibliotheksleiste: Ring mit Tagesfortschritt. */
export const ReadingStatsButton: React.FC<{
  rows: ReadingRow[] | null;
  onOpen: () => void;
}> = ({ rows, onOpen }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const goal = getSetting("reading_goal_minutes") ?? 0;
  const today = Math.round(minutesPerDay(rows ?? []).get(localDay()) ?? 0);
  const name =
    goal > 0
      ? t("readingStats.button.goal", { today, goal })
      : t("readingStats.button.plain", { today });
  return (
    <button
      type="button"
      className="rs-chip"
      onClick={onOpen}
      aria-label={name}
      title={name}
      data-testid="reading-stats-open"
    >
      {goal > 0 ? (
        <Ring value={today} goal={goal} size={24} stroke={4} label={false} />
      ) : (
        <BarChart3 size={18} aria-hidden="true" />
      )}
      <span className="rs-chip__text" aria-hidden="true">
        {goal > 0 ? (
          <>
            <b>{today}</b> / {t("readingStats.minShort", { n: goal })}
          </>
        ) : (
          t("readingStats.title")
        )}
      </span>
    </button>
  );
};

const STEPS = [0, 10, 15, 20, 30, 45, 60, 90, 120];
const WEEKDAYS = [0, 1, 2, 3, 4, 5, 6];

/** Ziel festlegen: Minuten und Wochentage (Dialog: Esc schließt, Fokus kehrt zurück). */
const GoalDialog: React.FC<{ open: boolean; onClose: () => void }> = ({
  open,
  onClose,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const [minutes, setMinutes] = useState(
    getSetting("reading_goal_minutes") ?? 0,
  );
  const [days, setDays] = useState(getSetting("reading_goal_days") ?? 0x7f);
  useEffect(() => {
    if (!open) return;
    setMinutes(getSetting("reading_goal_minutes") ?? 0);
    setDays(getSetting("reading_goal_days") ?? 0x7f);
  }, [open]);
  const weekday = (i: number) =>
    new Date(2026, 10, 2 + i).toLocaleDateString(undefined, {
      weekday: "short",
    });
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("readingStats.goal.title")}
      description={t("readingStats.goal.text")}
      closeLabel={t("common.close", { defaultValue: "Schließen" })}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("common.cancel", { defaultValue: "Abbrechen" })}
          </Button>
          <Button
            variant="primary"
            data-testid="reading-goal-save"
            onClick={async () => {
              await updateSetting("reading_goal_minutes", minutes);
              await updateSetting("reading_goal_days", days || 0x7f);
              onClose();
            }}
          >
            {t("common.save", { defaultValue: "Speichern" })}
          </Button>
        </>
      }
    >
      <fieldset className="rs-goal">
        <legend>{t("readingStats.goal.minutes")}</legend>
        <div className="rs-steps">
          {STEPS.map((m) => (
            <button
              key={m}
              type="button"
              aria-pressed={minutes === m}
              onClick={() => setMinutes(m)}
              data-testid={`reading-goal-${m}`}
            >
              {m === 0 ? t("readingStats.goal.off") : m}
            </button>
          ))}
        </div>
      </fieldset>
      <fieldset className="rs-goal" disabled={minutes === 0}>
        <legend>{t("readingStats.goal.days")}</legend>
        <div className="rs-days">
          {WEEKDAYS.map((i) => (
            <button
              key={i}
              type="button"
              aria-pressed={(days & (1 << i)) !== 0}
              onClick={() => setDays(days ^ (1 << i))}
            >
              {weekday(i)}
            </button>
          ))}
        </div>
      </fieldset>
    </Dialog>
  );
};

/** Balken je Tag (bzw. je Woche im Jahr), gestapelt nach Format. */
const HistoryChart: React.FC<{
  period: Period;
  perDay: Map<string, Record<Format, number>>;
  goal: number;
  avg: number;
  today: string;
}> = ({ period, perDay, goal, avg, today }) => {
  const { t } = useTranslation();
  const [asTable, setAsTable] = useState(false);
  const year = period.kind === "year";
  const columns = useMemo(() => {
    const days = daysOf(period);
    if (!year)
      return days.map((d) => ({
        label: String(Number(d.slice(8))),
        key: d,
        v: perDay.get(d) ?? { lesen: 0, hoeren: 0, vorlesen: 0 },
        future: d > today,
      }));
    // Jahr: Ø Minuten je Tag einer Kalenderwoche.
    const out: {
      label: string;
      key: string;
      v: Record<Format, number>;
      future: boolean;
    }[] = [];
    for (let i = 0; i < days.length; i += 7) {
      const week = days.slice(i, i + 7);
      const v = { lesen: 0, hoeren: 0, vorlesen: 0 };
      const counted = week.filter((d) => d <= today).length || 1;
      for (const d of week) {
        const x = perDay.get(d);
        if (!x) continue;
        for (const f of FORMATS) v[f] += x[f] / counted;
      }
      out.push({
        label: t("readingStats.weekShort", { n: out.length + 1 }),
        key: week[0],
        v,
        future: week[0] > today,
      });
    }
    return out;
  }, [period, perDay, today, year, t]);
  const total = (v: Record<Format, number>) => v.lesen + v.hoeren + v.vorlesen;
  const max = Math.max(60, goal * 1.2, ...columns.map((c) => total(c.v)));
  // Breite in echten Pixeln (1 Einheit = 1 px), damit Beschriftungen
  // unabhängig von der Fensterbreite 12 px groß bleiben (Codex 06.11.).
  const wrapRef = useRef<HTMLDivElement>(null);
  const [W, setW] = useState(820);
  useEffect(() => {
    const el = wrapRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      const w = Math.max(280, Math.round(el.clientWidth));
      setW((prev) => (Math.abs(prev - w) > 1 ? w : prev));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [asTable]);
  const H = 240;
  const pad = { l: 36, r: 10, t: 14, b: 26 };
  const iw = W - pad.l - pad.r;
  const ih = H - pad.t - pad.b;
  const y = (v: number) => pad.t + ih - (v / max) * ih;
  const bw = iw / Math.max(1, columns.length);
  const every = Math.max(1, Math.ceil((year ? 40 : 28) / bw));
  const grid = [0, 30, 60, 90, 120, 180].filter((v) => v <= max);
  return (
    <section className="rs-panel rs-history" aria-labelledby="rs-history">
      <h3 id="rs-history">
        {t("readingStats.history")}
        <span className="rs-panel__hint">
          {year ? t("readingStats.historyYear") : t("readingStats.historyDays")}
        </span>
        <Button
          variant="ghost"
          size="sm"
          icon={asTable ? <BarChart3 size={16} /> : <Table2 size={16} />}
          aria-pressed={asTable}
          aria-label={t("readingStats.asTable")}
          title={t("readingStats.asTable")}
          onClick={() => setAsTable(!asTable)}
        />
      </h3>
      {asTable ? (
        <div className="rs-tablewrap">
          <table className="rs-table" data-testid="reading-history-table">
            <thead>
              <tr>
                <th>{year ? t("readingStats.week") : t("readingStats.day")}</th>
                {FORMATS.map((f) => (
                  <th key={f} className="rs-num">
                    {t(`readingStats.format.${f}`)}
                  </th>
                ))}
                <th className="rs-num">{t("readingStats.total")}</th>
              </tr>
            </thead>
            <tbody>
              {columns
                .filter((c) => !c.future)
                .map((c) => (
                  <tr key={c.key}>
                    <td>
                      {year ? c.label : c.key.split("-").reverse().join(".")}
                    </td>
                    {FORMATS.map((f) => (
                      <td key={f} className="rs-num">
                        {Math.round(c.v[f])}
                      </td>
                    ))}
                    <td className="rs-num">{Math.round(total(c.v))}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div ref={wrapRef}>
          <svg
            viewBox={`0 0 ${W} ${H}`}
            width={W}
            height={H}
            className="rs-chart"
            role="img"
            aria-label={t("readingStats.chartLabel")}
          >
            {grid.map((v) => (
              <g key={v}>
                <line
                  x1={pad.l}
                  x2={W - pad.r}
                  y1={y(v)}
                  y2={y(v)}
                  className="rs-gridline"
                />
                <text
                  x={pad.l - 6}
                  y={y(v) + 4}
                  textAnchor="end"
                  className="rs-axis"
                >
                  {v}
                </text>
              </g>
            ))}
            {columns.map((c, i) => {
              const x = pad.l + i * bw + bw * 0.18;
              const w = Math.max(1, bw * 0.64);
              let acc = 0;
              const sum = total(c.v);
              return (
                <g key={c.key}>
                  <title>
                    {`${c.label}: ${Math.round(sum)} ${t("readingStats.minUnit")}`}
                  </title>
                  {FORMATS.map((f) => {
                    if (!c.v[f]) return null;
                    const top = y(acc + c.v[f]);
                    const bot = y(acc);
                    acc += c.v[f];
                    return (
                      <rect
                        key={f}
                        x={x}
                        y={top}
                        width={w}
                        height={Math.max(0, bot - top - 1)}
                        rx={2}
                        className={`rs-f-${f}`}
                      />
                    );
                  })}
                  {goal > 0 && sum >= goal && (
                    <circle
                      cx={x + w / 2}
                      cy={y(sum) - 6}
                      r={2.5}
                      className="rs-hit"
                    />
                  )}
                  {(i % every === 0 || c.key === today) && (
                    <text
                      x={x + w / 2}
                      y={H - 6}
                      textAnchor="middle"
                      className={
                        c.key === today ? "rs-axis rs-axis--today" : "rs-axis"
                      }
                    >
                      {c.label}
                    </text>
                  )}
                </g>
              );
            })}
            {goal > 0 && (
              <g>
                <line
                  x1={pad.l}
                  x2={W - pad.r}
                  y1={y(goal)}
                  y2={y(goal)}
                  className="rs-goalline"
                />
                <rect
                  x={pad.l + 2}
                  y={y(goal) - 18}
                  width={92}
                  height={16}
                  rx={2}
                  className="rs-goalbg"
                />
                <text x={pad.l + 6} y={y(goal) - 5} className="rs-goaltext">
                  {t("readingStats.goalLine", { n: goal })}
                </text>
              </g>
            )}
            {avg > 0 && (
              <g>
                <line
                  x1={pad.l}
                  x2={W - pad.r}
                  y1={y(avg)}
                  y2={y(avg)}
                  className="rs-avgline"
                />
                <text
                  x={W - pad.r}
                  y={y(avg) + (avg > goal ? -4 : 12)}
                  textAnchor="end"
                  className="rs-axis"
                >
                  {t("readingStats.avgLine", { n: Math.round(avg) })}
                </text>
              </g>
            )}
          </svg>
        </div>
      )}
      <div className="rs-legend">
        {FORMATS.map((f) => (
          <span key={f}>
            <i className={`rs-f-${f}`} />
            {t(`readingStats.format.${f}`)}
          </span>
        ))}
        {goal > 0 && (
          <span>
            <i className="rs-hit rs-hit--dot" />
            {t("readingStats.goalHit")}
          </span>
        )}
      </div>
    </section>
  );
};

const Bars: React.FC<{
  items: { key: string; label: string; sec: number }[];
  total: number;
  selected?: string | null;
  onSelect?: (key: string | null) => void;
  tone?: "brand" | "green";
  value?: (sec: number) => string;
}> = ({ items, total, selected, onSelect, tone = "brand", value }) => {
  const max = Math.max(1, ...items.map((i) => i.sec));
  return (
    <ul className="rs-bars">
      {items.map((i) => (
        <li key={i.key}>
          <button
            type="button"
            className="rs-bar"
            disabled={!onSelect}
            aria-pressed={onSelect ? selected === i.key : undefined}
            onClick={() => onSelect?.(selected === i.key ? null : i.key)}
          >
            <span className="rs-bar__name">{i.label}</span>
            <span className="rs-bar__track">
              <i
                className={`rs-bar__fill rs-bar__fill--${tone}`}
                style={{ width: `${(i.sec / max) * 100}%` }}
              />
            </span>
            <span className="rs-bar__value">
              {value
                ? value(i.sec)
                : `${Math.round((i.sec / Math.max(1, total)) * 100)} %`}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
};

/** Jahreskalender: Farbstufe nach Minuten, Punkt = Ziel erreicht. */
const YearGrid: React.FC<{
  period: Period;
  perDay: Map<string, number>;
  goal: number;
  today: string;
  hit: number;
  due: number;
}> = ({ period, perDay, goal, today, hit, due }) => {
  const { t, i18n } = useTranslation();
  const days = daysOf(period);
  const lead = (parseDay(period.from).getDay() + 6) % 7;
  const level = (m: number) =>
    m <= 0 ? 0 : m < 15 ? 1 : m < 30 ? 2 : m < 60 ? 3 : 4;
  const months = Array.from({ length: 12 }, (_, i) =>
    new Date(2026, i, 1).toLocaleDateString(i18n.language, { month: "short" }),
  );
  return (
    <section className="rs-panel" aria-labelledby="rs-year">
      <h3 id="rs-year">
        {t("readingStats.yearTitle", { year: period.from.slice(0, 4) })}
        <span className="rs-panel__hint">{t("readingStats.yearHint")}</span>
      </h3>
      <div className="rs-months" aria-hidden="true">
        {months.map((m) => (
          <span key={m}>{m}</span>
        ))}
      </div>
      <div
        className="rs-heat"
        role="img"
        aria-label={t("readingStats.yearLabel")}
      >
        {Array.from({ length: lead }, (_, i) => (
          <i key={`l${i}`} className="rs-heat__pad" />
        ))}
        {days.map((d) => {
          const m = perDay.get(d) ?? 0;
          return (
            <i
              key={d}
              title={`${d.split("-").reverse().join(".")}: ${Math.round(m)} ${t("readingStats.minUnit")}`}
              className={`rs-heat__d rs-l${level(m)}${d > today ? " rs-heat__future" : ""}${goal > 0 && m >= goal ? " rs-heat__hit" : ""}`}
            />
          );
        })}
      </div>
      <div className="rs-legend">
        {t("readingStats.less")}
        {[0, 1, 2, 3, 4].map((l) => (
          <i key={l} className={`rs-heat__d rs-l${l}`} />
        ))}
        {t("readingStats.more")}
        {goal > 0 && <span>· {t("readingStats.goalDays", { hit, due })}</span>}
      </div>
    </section>
  );
};

const BOOKS_SHOWN = 10;

export const ReadingStats: React.FC<{
  books: LibraryBook[];
  onBack: () => void;
  onOpenBook: (id: string) => void;
  onBooksChanged: () => void;
}> = ({ books, onBack, onOpenBook, onBooksChanged }) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const llm = useFunctionLlm("read_aloud");
  const { rows, reload } = useReadingRows();
  const today = localDay();
  const goal = getSetting("reading_goal_minutes") ?? 0;
  const mask = getSetting("reading_goal_days") ?? 0x7f;
  const paused = getSetting("reading_stats_paused") ?? false;
  const syncOn = getSetting("reading_stats_sync") ?? false;
  const aiOn = getSetting("reading_stats_ai") ?? true;
  const [kind, setKind] = useState<PeriodKind>("month");
  const [shift, setShift] = useState(0);
  const [category, setCategory] = useState<string | null>(null);
  const [topic, setTopic] = useState<string | null>(null);
  const [allBooks, setAllBooks] = useState(false);
  const [goalOpen, setGoalOpen] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearAll, setClearAll] = useState(false);
  const [classifyOpen, setClassifyOpen] = useState(false);
  const [classifying, setClassifying] = useState<{
    done: number;
    total: number;
  } | null>(null);
  const [deviceSync, setDeviceSync] = useState(false);
  const [askSync, setAskSync] = useState(false);
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    headingRef.current?.focus();
    void flushReadingLog().then(reload);
    void commands
      .syncStatus()
      .then((s) => setDeviceSync(s.connected))
      .catch(() => undefined);
    try {
      setAskSync(
        window.localStorage.getItem("lva.ui.readingStats.syncAsked") !== "1",
      );
    } catch {
      setAskSync(true);
    }
  }, [reload]);

  const period = periodOf(kind, today, shift);
  const prev = previousPeriod(period);
  const byId = useMemo(() => new Map(books.map((b) => [b.id, b])), [books]);
  const all = rows ?? [];
  const filtered = useMemo(() => {
    if (!category && !topic) return all;
    return all.filter((r) => {
      const b = byId.get(r.book_id);
      if (!b) return false;
      if (category && b.category !== category) return false;
      if (topic && !(b.topics ?? []).includes(topic)) return false;
      return true;
    });
  }, [all, byId, category, topic]);
  const sum = useMemo(
    () => summarize(filtered, books, period, lengthsOf),
    [filtered, books, period.from, period.to],
  );
  const before = useMemo(
    () => summarize(filtered, books, prev, lengthsOf),
    [filtered, books, prev.from, prev.to],
  );
  const unfiltered = useMemo(
    () => summarize(all, books, period, lengthsOf),
    [all, books, period.from, period.to],
  );
  const g = useMemo(
    () => goalState(all, goal, mask, today, period),
    [all, goal, mask, today, period.from, period.to],
  );
  const perDayTotal = useMemo(() => minutesPerDay(filtered), [filtered]);
  const daysInPeriod = daysOf(period).filter((d) => d <= today).length || 1;
  const avgPeriod = sum.sec / 60 / daysInPeriod;
  const finished = sum.books.filter((b) => byId.get(b.bookId)?.finished).length;
  const unclassified = books.filter(needsClassification);
  const hasData = all.length > 0;
  const langName = (code: string) => {
    if (code === "und") return t("readingStats.langUnknown");
    try {
      return (
        new Intl.DisplayNames([i18n.language], { type: "language" }).of(code) ??
        code
      );
    } catch {
      return code;
    }
  };
  const periodLabel = (() => {
    const from = parseDay(period.from);
    if (kind === "year") return period.from.slice(0, 4);
    if (kind === "month")
      return from.toLocaleDateString(undefined, {
        month: "long",
        year: "numeric",
      });
    const to = parseDay(period.to);
    return `${from.toLocaleDateString(undefined, { day: "numeric", month: "numeric" })} – ${to.toLocaleDateString(undefined, { day: "numeric", month: "numeric", year: "numeric" })}`;
  })();
  const change =
    before.sec > 0
      ? Math.round(((sum.sec - before.sec) / before.sec) * 100)
      : null;

  const exportCsv = async () => {
    const target = await save({
      filters: [{ name: "CSV", extensions: ["csv"] }],
      defaultPath: `lesestatistik_${period.from}_${period.to}.csv`,
    });
    if (typeof target !== "string") return;
    const res = await commands.readingLogExport(
      target,
      toCsv(filtered, books, period, lengthsOf),
    );
    if (res.status === "ok") toast.success(t("readingStats.exported"));
    else toast.error(res.error);
  };

  const runClassify = async () => {
    setClassifyOpen(false);
    const todo = unclassified;
    setClassifying({ done: 0, total: todo.length });
    let failed = 0;
    const current = [...books];
    for (let i = 0; i < todo.length; i++) {
      try {
        const saved = await classifyBook(todo[i], current);
        const at = current.findIndex((b) => b.id === saved.id);
        if (at >= 0) current[at] = saved;
      } catch {
        failed++;
      }
      setClassifying({ done: i + 1, total: todo.length });
    }
    setClassifying(null);
    onBooksChanged();
    if (failed)
      toast.error(t("readingStats.classify.failed", { count: failed }));
    else toast.success(t("readingStats.classify.done", { count: todo.length }));
  };

  const shownBooks = allBooks ? sum.books : sum.books.slice(0, BOOKS_SHOWN);
  const categoryItems = sum.byCategory.map((c) => ({
    key: c.key,
    label: categoryLabel(c.key, t) ?? c.key,
    sec: c.sec,
  }));
  const langItems = sum.byLang.map((l) => ({
    key: l.key,
    label: langName(l.key),
    sec: l.sec,
  }));

  return (
    <div className="rs" data-testid="reading-stats">
      <div className="rs-head">
        <Button
          variant="ghost"
          icon={<ChevronLeft size={18} />}
          onClick={onBack}
          aria-label={t("readingStats.back")}
          data-testid="reading-stats-back"
        />
        <h2 ref={headingRef} tabIndex={-1}>
          {t("readingStats.title")}
        </h2>
      </div>

      {paused && (
        <div
          className="rs-notice"
          role="status"
          data-testid="reading-stats-paused"
        >
          <Pause size={16} aria-hidden="true" />
          <span>{t("readingStats.pausedText")}</span>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => updateSetting("reading_stats_paused", false)}
          >
            {t("readingStats.resume")}
          </Button>
        </div>
      )}
      {deviceSync && !syncOn && askSync && (
        <div
          className="rs-notice"
          role="region"
          aria-label={t("readingStats.sync.title")}
        >
          <Lock size={16} aria-hidden="true" />
          <span>{t("readingStats.sync.ask")}</span>
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              void updateSetting("reading_stats_sync", true);
              setAskSync(false);
            }}
          >
            {t("readingStats.sync.yes")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              try {
                window.localStorage.setItem(
                  "lva.ui.readingStats.syncAsked",
                  "1",
                );
              } catch {
                /* ohne Speicher fragt die Seite beim nächsten Mal wieder */
              }
              setAskSync(false);
            }}
          >
            {t("readingStats.sync.no")}
          </Button>
        </div>
      )}

      {rows !== null && !hasData ? (
        <section
          className="rs-panel rs-empty"
          data-testid="reading-stats-empty"
        >
          <h3>
            <BarChart3 size={18} aria-hidden="true" />{" "}
            {t("readingStats.empty.title")}
          </h3>
          <p>
            {paused
              ? t("readingStats.empty.paused")
              : t("readingStats.empty.text")}
          </p>
          <div className="rs-row">
            <Button variant="primary" onClick={() => setGoalOpen(true)}>
              {goal > 0
                ? t("readingStats.goal.change")
                : t("readingStats.goal.set")}
            </Button>
          </div>
        </section>
      ) : (
        <>
          <section className="rs-panel" aria-labelledby="rs-today">
            <h3 id="rs-today">
              {t("readingStats.today")}
              <Button
                variant="ghost"
                size="sm"
                className="rs-push"
                onClick={() => setGoalOpen(true)}
                data-testid="reading-goal-open"
              >
                {goal > 0
                  ? t("readingStats.goal.change")
                  : t("readingStats.goal.set")}
              </Button>
            </h3>
            <div className="rs-today">
              <div
                role="img"
                aria-label={
                  goal > 0
                    ? t("readingStats.button.goal", {
                        today: Math.round(g.todayMin),
                        goal,
                      })
                    : t("readingStats.button.plain", {
                        today: Math.round(g.todayMin),
                      })
                }
              >
                <Ring
                  value={g.todayMin}
                  goal={goal}
                  unit={
                    goal > 0
                      ? t("readingStats.ofGoal", { n: goal })
                      : t("readingStats.minUnit")
                  }
                />
              </div>
              <div className="rs-kpis">
                {goal > 0 ? (
                  <div
                    className={`rs-kpi${g.todayMin >= goal ? " rs-kpi--ok" : ""}`}
                  >
                    <b>
                      {g.todayMin >= goal
                        ? t("readingStats.reached")
                        : t("readingStats.minShort", {
                            n: Math.ceil(goal - g.todayMin),
                          })}
                    </b>
                    <span>
                      {g.todayMin >= goal
                        ? t("readingStats.reachedText")
                        : t("readingStats.left")}
                    </span>
                  </div>
                ) : (
                  <div className="rs-kpi">
                    <b>{t("readingStats.goal.none")}</b>
                    <span>{t("readingStats.goal.noneText")}</span>
                  </div>
                )}
                {goal > 0 && (
                  <div className={`rs-kpi${g.streak > 0 ? " rs-kpi--ok" : ""}`}>
                    <b>
                      <Flame size={16} aria-hidden="true" />{" "}
                      {t("readingStats.days", { count: g.streak })}
                    </b>
                    <span>{t("readingStats.streak")}</span>
                  </div>
                )}
                <div className="rs-kpi">
                  <b>{t("readingStats.minShort", { n: Math.round(g.avg7) })}</b>
                  <span>{t("readingStats.avg7")}</span>
                </div>
                <div className="rs-kpi">
                  <b>
                    {t("readingStats.minShort", { n: Math.round(g.avg30) })}
                  </b>
                  <span>{t("readingStats.avg30")}</span>
                </div>
                {goal > 0 && (
                  <div className="rs-kpi">
                    <b>
                      {g.hit} / {g.due}
                    </b>
                    <span>{t("readingStats.goalInPeriod")}</span>
                  </div>
                )}
              </div>
            </div>
          </section>

          <div className="rs-period">
            <div
              className="rs-seg"
              role="group"
              aria-label={t("readingStats.period")}
            >
              {(["week", "month", "year"] as PeriodKind[]).map((k) => (
                <button
                  key={k}
                  type="button"
                  aria-pressed={kind === k}
                  onClick={() => {
                    setKind(k);
                    setShift(0);
                  }}
                  data-testid={`reading-period-${k}`}
                >
                  {t(`readingStats.kind.${k}`)}
                </button>
              ))}
            </div>
            <Button
              variant="ghost"
              icon={<ChevronLeft size={18} />}
              aria-label={t("readingStats.prev")}
              onClick={() => setShift(shift - 1)}
            />
            <strong aria-live="polite" data-testid="reading-period-label">
              {periodLabel}
            </strong>
            <Button
              variant="ghost"
              icon={<ChevronRight size={18} />}
              aria-label={t("readingStats.next")}
              disabled={shift >= 0}
              onClick={() => setShift(shift + 1)}
            />
            <Button
              variant="secondary"
              disabled={shift === 0}
              onClick={() => setShift(0)}
            >
              {t("readingStats.now")}
            </Button>
            {(category || topic) && (
              <Button
                variant="secondary"
                onClick={() => {
                  setCategory(null);
                  setTopic(null);
                }}
              >
                {t("readingStats.filterClear", {
                  name: topic ?? categoryLabel(category, t),
                })}
              </Button>
            )}
          </div>

          <div className="rs-tiles">
            <div className="rs-tile">
              <span>{t("readingStats.tile.time")}</span>
              <b>{formatDuration(sum.sec)}</b>
              <em>
                {change === null
                  ? t("readingStats.tile.avg", { n: Math.round(avgPeriod) })
                  : t("readingStats.tile.change", {
                      n: change > 0 ? `+${change}` : change,
                    })}
              </em>
            </div>
            <div className="rs-tile">
              <span>{t("readingStats.tile.pages")}</span>
              <b>{Math.round(sum.pages).toLocaleString()}</b>
              <em>
                {sum.pagesEstimated
                  ? t("readingStats.tile.pagesNorm")
                  : t("readingStats.tile.pagesBook")}
              </em>
            </div>
            <div className="rs-tile">
              <span>{t("readingStats.tile.books")}</span>
              <b>{sum.books.length}</b>
              <em>
                {t("readingStats.tile.audio", {
                  count: sum.books.filter((b) => b.formats.includes("hoeren"))
                    .length,
                })}
              </em>
            </div>
            <div className="rs-tile">
              <span>{t("readingStats.tile.finished")}</span>
              <b>{finished}</b>
              <em>{t("readingStats.tile.finishedHint")}</em>
            </div>
          </div>

          {sum.sec === 0 && (
            <p className="rs-muted" data-testid="reading-stats-period-empty">
              {t("readingStats.empty.period")}
            </p>
          )}

          <div className="rs-grid">
            <HistoryChart
              period={period}
              perDay={sum.perDay}
              goal={category || topic ? 0 : goal}
              avg={avgPeriod}
              today={today}
            />
            <section className="rs-panel" aria-labelledby="rs-focus">
              <h3 id="rs-focus">
                {t("readingStats.focus")}
                <span className="rs-panel__hint">
                  {t("readingStats.share")}
                </span>
              </h3>
              {categoryItems.length ? (
                <Bars
                  items={categoryItems}
                  total={unfiltered.byCategory.reduce((a, c) => a + c.sec, 0)}
                  selected={category}
                  onSelect={setCategory}
                />
              ) : (
                <p className="rs-muted">{t("readingStats.noCategories")}</p>
              )}
              {sum.byTopic.length > 0 && (
                <>
                  <div className="rs-sub">{t("readingStats.topics")}</div>
                  <div className="rs-topics">
                    {sum.byTopic.slice(0, 12).map((x) => (
                      <button
                        key={x.key}
                        type="button"
                        className="rs-topic"
                        aria-pressed={topic === x.key}
                        onClick={() => setTopic(topic === x.key ? null : x.key)}
                      >
                        {x.key}
                        <b>{formatDuration(x.sec)}</b>
                      </button>
                    ))}
                  </div>
                </>
              )}
              {sum.fictionShare !== null && (
                <div className="rs-muted">
                  {t("readingStats.fiction", {
                    non: Math.round((1 - sum.fictionShare) * 100),
                    fic: Math.round(sum.fictionShare * 100),
                  })}
                </div>
              )}
              {aiOn && unclassified.length > 0 && (
                <div className="rs-row">
                  <Button
                    variant="secondary"
                    size="sm"
                    icon={<Sparkles size={16} />}
                    disabled={!!classifying || !llm.model}
                    onClick={() => setClassifyOpen(true)}
                    data-testid="reading-classify-all"
                  >
                    {classifying
                      ? t("readingStats.classify.running", classifying)
                      : t("readingStats.classify.all", {
                          count: unclassified.length,
                        })}
                  </Button>
                </div>
              )}
            </section>
          </div>

          <div className="rs-grid">
            <section
              className="rs-panel rs-bookspanel"
              aria-labelledby="rs-books"
            >
              <h3 id="rs-books">
                {t("readingStats.books")}
                <span className="rs-panel__hint">
                  {sum.books.length > BOOKS_SHOWN && !allBooks
                    ? t("readingStats.excerpt", {
                        shown: BOOKS_SHOWN,
                        total: sum.books.length,
                      })
                    : t("readingStats.inPeriod")}
                </span>
              </h3>
              <table className="rs-table rs-books">
                <thead>
                  <tr>
                    <th>
                      <span className="sr-only">{t("readingStats.cover")}</span>
                    </th>
                    <th>{t("readingStats.book")}</th>
                    <th className="rs-wide">{t("readingStats.category")}</th>
                    <th>{t("readingStats.formats")}</th>
                    <th className="rs-num">{t("readingStats.time")}</th>
                    <th className="rs-num rs-wide">
                      {t("readingStats.pages")}
                    </th>
                    <th className="rs-wide">{t("readingStats.state")}</th>
                  </tr>
                </thead>
                <tbody>
                  {shownBooks.map((b) => {
                    const book = byId.get(b.bookId);
                    return (
                      <tr key={b.bookId}>
                        <td className="rs-cover">
                          {book && <BookCover book={book} size="xs" />}
                        </td>
                        <td>
                          {book ? (
                            <button
                              type="button"
                              className="rs-link"
                              onClick={() => onOpenBook(book.id)}
                            >
                              {book.title}
                            </button>
                          ) : (
                            <span className="rs-muted">
                              {t("readingStats.removedBook")}
                            </span>
                          )}
                          <div className="rs-muted">
                            {[
                              book?.authors.join(", "),
                              b.langs.map(langName).join(", "),
                            ]
                              .filter(Boolean)
                              .join(" · ")}
                          </div>
                        </td>
                        <td className="rs-wide">
                          {categoryLabel(book?.category, t) ?? (
                            <span className="rs-muted">
                              {t("readingStats.unclassified")}
                            </span>
                          )}
                        </td>
                        <td>
                          <span className="rs-fm">
                            {b.formats.map((f) => {
                              const I = FORMAT_ICON[f];
                              return (
                                <i
                                  key={f}
                                  className={`rs-f-${f}`}
                                  title={t(`readingStats.format.${f}`)}
                                >
                                  <I
                                    size={12}
                                    aria-label={t(`readingStats.format.${f}`)}
                                  />
                                </i>
                              );
                            })}
                          </span>
                        </td>
                        <td className="rs-num">{formatDuration(b.sec)}</td>
                        <td className="rs-num rs-wide">
                          {b.pages ? Math.round(b.pages) : "–"}
                        </td>
                        <td className="rs-wide">
                          {book?.finished ? (
                            <span className="rs-pill rs-pill--done">
                              {t("readingStats.finished")}
                            </span>
                          ) : (
                            <span className="rs-pill">
                              {t("readingStats.daysRead", { count: b.days })}
                            </span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {sum.books.length > BOOKS_SHOWN && (
                <div className="rs-row">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => setAllBooks(!allBooks)}
                  >
                    {allBooks
                      ? t("readingStats.showLess")
                      : t("readingStats.showAll", { count: sum.books.length })}
                  </Button>
                </div>
              )}
              <div className="rs-muted">{t("readingStats.noPagesHint")}</div>
            </section>
            <section className="rs-panel" aria-labelledby="rs-langs">
              <h3 id="rs-langs">{t("readingStats.languages")}</h3>
              {langItems.length ? (
                <Bars
                  items={langItems}
                  total={sum.sec}
                  tone="green"
                  value={formatDuration}
                />
              ) : (
                <p className="rs-muted">{t("readingStats.empty.period")}</p>
              )}
              <div className="rs-sub">{t("readingStats.byFormat")}</div>
              <Bars
                items={FORMATS.map((f) => ({
                  key: f,
                  label: t(`readingStats.format.${f}`),
                  sec: sum.byFormat[f],
                }))}
                total={sum.sec}
                value={formatDuration}
              />
            </section>
          </div>

          {kind === "year" && (
            <YearGrid
              period={period}
              perDay={perDayTotal}
              goal={category || topic ? 0 : goal}
              today={today}
              hit={g.hit}
              due={g.due}
            />
          )}
        </>
      )}

      <footer className="rs-foot">
        <span className="rs-foot__lock">
          <Lock size={14} aria-hidden="true" />
          {syncOn
            ? t("readingStats.privacySync")
            : t("readingStats.privacyLocal")}{" "}
          {aiOn ? t("readingStats.privacyAi") : t("readingStats.privacyAiOff")}
        </span>
        <Button
          variant="secondary"
          size="sm"
          icon={<Download size={16} />}
          disabled={!hasData}
          onClick={() => void exportCsv()}
        >
          {t("readingStats.export")}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          icon={paused ? <Play size={16} /> : <Pause size={16} />}
          title={t("readingStats.pauseHint")}
          onClick={() => updateSetting("reading_stats_paused", !paused)}
          data-testid="reading-stats-pause"
        >
          {paused ? t("readingStats.resume") : t("readingStats.pause")}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          icon={<Trash2 size={16} />}
          disabled={!hasData}
          onClick={() => setClearOpen(true)}
          data-testid="reading-stats-clear"
        >
          {t("readingStats.clear")}
        </Button>
        <label className="rs-switch">
          <input
            type="checkbox"
            checked={syncOn}
            onChange={(e) =>
              updateSetting("reading_stats_sync", e.target.checked)
            }
          />
          {t("readingStats.sync.switch")}
        </label>
        <label className="rs-switch">
          <input
            type="checkbox"
            checked={aiOn}
            onChange={(e) =>
              updateSetting("reading_stats_ai", e.target.checked)
            }
          />
          {t("readingStats.ai.switch")}
        </label>
      </footer>

      <GoalDialog open={goalOpen} onClose={() => setGoalOpen(false)} />

      <Dialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title={t("readingStats.clearTitle")}
        description={t("readingStats.clearText")}
        closeLabel={t("common.close", { defaultValue: "Schließen" })}
        footer={
          <>
            <Button variant="secondary" onClick={() => setClearOpen(false)}>
              {t("common.cancel", { defaultValue: "Abbrechen" })}
            </Button>
            <Button
              variant="danger"
              data-testid="reading-stats-clear-confirm"
              onClick={async () => {
                const from = clearAll ? "1970-01-01" : period.from;
                const to = clearAll
                  ? today
                  : period.to < today
                    ? period.to
                    : today;
                // Erst den Puffer melden: sonst landen Zuwächse der letzten
                // Sekunden nach dem Grabstein wieder im gelöschten Zeitraum.
                await flushReadingLog();
                try {
                  const res = await commands.readingLogClear(from, to);
                  if (res.status === "ok") {
                    toast.success(
                      t("readingStats.cleared", { count: res.data }),
                    );
                    reload();
                  } else toast.error(res.error);
                } catch (e) {
                  toast.error(String(e));
                } finally {
                  setClearOpen(false);
                }
              }}
            >
              {t("readingStats.clearConfirm")}
            </Button>
          </>
        }
      >
        <div className="rs-radio" role="radiogroup">
          <label>
            <input
              type="radio"
              checked={!clearAll}
              onChange={() => setClearAll(false)}
            />
            {t("readingStats.clearPeriod", { label: periodLabel })}
          </label>
          <label>
            <input
              type="radio"
              checked={clearAll}
              onChange={() => setClearAll(true)}
            />
            {t("readingStats.clearAll")}
          </label>
        </div>
      </Dialog>

      <Dialog
        open={classifyOpen}
        onOpenChange={setClassifyOpen}
        title={t("readingStats.classify.title")}
        closeLabel={t("common.close", { defaultValue: "Schließen" })}
        footer={
          <>
            <Button variant="secondary" onClick={() => setClassifyOpen(false)}>
              {t("common.cancel", { defaultValue: "Abbrechen" })}
            </Button>
            <Button
              variant="primary"
              onClick={() => void runClassify()}
              data-testid="reading-classify-confirm"
            >
              {t("readingStats.classify.start", { count: unclassified.length })}
            </Button>
          </>
        }
      >
        <p>
          {llm.local
            ? t("readingStats.classify.local", {
                model: llm.model,
                connection: llm.connection,
              })
            : t("readingStats.classify.remote", {
                model: llm.model,
                connection: llm.connection,
              })}
        </p>
        <p className="rs-muted">{t("readingStats.classify.fields")}</p>
      </Dialog>
    </div>
  );
};
