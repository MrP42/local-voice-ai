import React from "react";
import { useTranslation } from "react-i18next";
import {
  BookOpen,
  Check,
  Headphones,
  Loader2,
  Play,
  Sparkles,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { KurzbuchInfo } from "./useKurzbuecher";
import {
  formatDuration,
  openInReader,
  openKiDocInReader,
  type BookView,
} from "@/lib/library/books";
import { LISTEN_WPM, READ_WPM } from "@/lib/kurzbuch/kurzbuch";
import { listenedFraction } from "../settings/tts/kidocs/kiDocs";
import { kiLangLabel } from "../settings/tts/kidocs/targetLang";
import { langShort } from "@/lib/library/editions";

export type FormatKind = "text" | "audiobook" | "kurzbuch";

type T = (key: string, options?: Record<string, unknown>) => string;

/** Eine Fassung eines Buchs mit eigenem Fortschritt (Patrick 04.11.). */
export interface FormatEntry {
  kind: FormatKind;
  fraction: number;
  /** „4 %“ bzw. „Fertig“. */
  percent: string;
  /** Rest- oder Gesamtdauer („noch 8 h 41 min“, „8 h 53 min“), sofern bekannt. */
  time: string | null;
  /** Kurzer Wert: Dauer, sonst Prozent. */
  value: string;
  /** Vollständiger Name für Screenreader und Tooltip. */
  label: string;
  /** Sprache des Kurzbuchs, wenn es mehrere gibt. */
  badge?: string;
  /** Text entsteht noch (Transkription): kein Start möglich. */
  busy?: string;
  finished: boolean;
}

const ICON: Record<FormatKind, LucideIcon> = {
  text: BookOpen,
  audiobook: Headphones,
  kurzbuch: Sparkles,
};

/** Zeichen je Wort für die Lesedauer aus der Textlänge. */
const CHARS_PER_WORD = 6;
const readMs = (chars: number) =>
  Math.round((chars / CHARS_PER_WORD / READ_WPM) * 60_000);
const listenMs = (words: number) =>
  Math.round((Math.max(0, words) / LISTEN_WPM) * 60_000);

/** Dauer: begonnen „noch …“, sonst die Gesamtdauer; fertig oder unbekannt: null. */
function timeOf(
  t: T,
  fraction: number,
  totalMs: number | null,
  leftMs?: number | null,
): string | null {
  if (fraction >= 1 || totalMs == null || !(totalMs > 0)) return null;
  return fraction > 0
    ? t("library.status.left", {
        time: formatDuration(leftMs ?? totalMs * (1 - fraction)),
      })
    : formatDuration(totalMs);
}

function entry(
  t: T,
  kind: FormatKind,
  fraction: number,
  time: string | null,
  labelKey: string,
  title: string,
  badge?: string,
): FormatEntry {
  const finished = fraction >= 1;
  const percent = finished
    ? t("library.status.finished")
    : t("library.status.percent", { percent: Math.round(fraction * 100) });
  const value = finished ? percent : (time ?? percent);
  const detail = [badge, time && !finished ? `${percent}, ${time}` : percent]
    .filter(Boolean)
    .join(", ");
  return {
    kind,
    fraction,
    percent,
    time: finished ? null : time,
    value,
    finished,
    badge,
    label: t(labelKey, { title, value: detail }),
  };
}

/** Vorhandene Fassungen eines Buchs in fester Reihenfolge. */
export function formatEntries(
  t: T,
  v: BookView,
  kurzbuch: KurzbuchInfo | undefined,
  busyText?: string,
): FormatEntry[] {
  const out: FormatEntry[] = [];
  const title = v.book.title;
  // Mehrere Sprachfassungen: die Zeilen tragen das Kürzel der gezeigten Sprache.
  const lang =
    (v.book.editions?.length ?? 0) > 1 ? langShort(v.edition?.lang) : undefined;
  if (v.shown.text) {
    const f = v.textFraction;
    const total = v.textChars ? readMs(v.textChars) : null;
    out.push(
      entry(
        t,
        "text",
        f,
        timeOf(t, f, total),
        "library.fmt.readTitle",
        title,
        lang,
      ),
    );
  } else if (busyText) {
    out.push({
      kind: "text",
      fraction: 0,
      percent: "",
      time: null,
      value: "",
      finished: false,
      busy: busyText,
      label: busyText,
    });
  }
  if (v.shown.audio_path) {
    const f = v.audioFraction;
    out.push(
      entry(
        t,
        "audiobook",
        f,
        timeOf(t, f, v.audioTotalMs, v.audioLeftMs),
        "library.fmt.listenTitle",
        title,
        lang,
      ),
    );
  }
  if (kurzbuch) {
    const f = v.book.finished
      ? 1
      : listenedFraction(kurzbuch.page, kurzbuch.doc.id);
    const kiLang =
      kurzbuch.langs > 1 ? kiLangLabel(kurzbuch.doc.language) : undefined;
    out.push(
      entry(
        t,
        "kurzbuch",
        f,
        timeOf(t, f, listenMs(kurzbuch.doc.words)),
        "library.fmt.kurzbuchTitle",
        title,
        kiLang || undefined,
      ),
    );
  }
  return out;
}

/** Symbol und kurzer Wert einer Fassung, z. B. für „Weitermachen“. */
export const FormatValue: React.FC<{ entry: FormatEntry }> = ({ entry }) => {
  const { t } = useTranslation();
  const Icon = ICON[entry.kind];
  return (
    <span
      className="library-fmtvalue"
      data-kind={entry.kind}
      title={entry.label}
    >
      <Icon size={13} aria-hidden="true" />
      <span className="library-fmtvalue__name">
        {t(`library.fmt.${entry.kind}`)}
      </span>
      <span className="library-fmtvalue__value">{entry.value}</span>
    </span>
  );
};

/** Start einer Fassung: Text lesen, Hörbuch hören, Kurzbuch vorlesen. */
export function startFormat(
  kind: FormatKind,
  view: BookView,
  kurzbuch: KurzbuchInfo | undefined,
  onListen: () => void,
): void {
  if (kind === "text" && view.shown.text)
    openInReader(view.shown.text.page_id, false);
  else if (kind === "audiobook") onListen();
  else if (kind === "kurzbuch" && kurzbuch)
    openKiDocInReader({
      page: kurzbuch.page,
      doc: kurzbuch.doc.id,
      play: true,
    });
}

/**
 * Fassungen auf der Bibliothekskarte (Patrick 04.11.): je vorhandenem Format
 * eine Zeile mit Symbol, Namen, Prozent, Balken, Dauer und Startzeichen; die
 * ganze Zeile startet genau diese Fassung.
 */
export const FormatRows: React.FC<{
  view: BookView;
  busyText?: string;
  kurzbuch?: KurzbuchInfo;
  onListen: () => void;
}> = ({ view, busyText, kurzbuch, onListen }) => {
  const { t } = useTranslation();
  const entries = formatEntries(t, view, kurzbuch, busyText);
  if (!entries.length)
    return <div className="library-meta">{t("library.status.empty")}</div>;
  return (
    <div
      className="library-fmtrows"
      role="group"
      aria-label={t("library.fmt.group", { title: view.book.title })}
    >
      {entries.map((e) => {
        const Icon = ICON[e.kind];
        if (e.busy)
          return (
            <div
              key={e.kind}
              className="library-fmtrow library-fmtrow--busy"
              role="status"
              data-testid={`library-fmt-${e.kind}-${view.book.id}`}
            >
              <span className="library-fmtrow__line">
                <Loader2 size={14} className="lib-spin" aria-hidden="true" />
                <span className="library-fmtrow__busy">{e.busy}</span>
              </span>
            </div>
          );
        return (
          <button
            key={e.kind}
            type="button"
            className="library-fmtrow"
            data-kind={e.kind}
            data-finished={e.finished || undefined}
            aria-label={e.label}
            title={e.label}
            onClick={(ev) => {
              ev.stopPropagation();
              startFormat(e.kind, view, kurzbuch, onListen);
            }}
            data-testid={`library-fmt-${e.kind}-${view.book.id}`}
          >
            <span className="library-fmtrow__line">
              <Icon size={14} aria-hidden="true" />
              <span className="library-fmtrow__name">
                {t(`library.fmt.${e.kind}`)}
                {e.badge && (
                  <span className="library-fmtrow__badge">{e.badge}</span>
                )}
              </span>
              {e.time && (
                <span className="library-fmtrow__percent">{e.percent}</span>
              )}
              <span className="library-fmtrow__go" aria-hidden="true">
                <Play size={9} fill="currentColor" />
              </span>
            </span>
            <span className="library-fmtrow__line">
              <span className="library-fmtrow__bar" aria-hidden="true">
                <span style={{ width: `${Math.round(e.fraction * 100)}%` }} />
              </span>
              <span className="library-fmtrow__value">
                {e.finished && <Check size={12} aria-hidden="true" />}
                {e.value}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
};
