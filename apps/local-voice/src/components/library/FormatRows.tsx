import React from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, Check, Headphones, Loader2, Sparkles } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { KurzbuchInfo } from "./useKurzbuecher";
import {
  formatDuration,
  openInReader,
  openKiDocInReader,
  type BookView,
} from "@/lib/library/books";
import { LISTEN_WPM } from "@/lib/kurzbuch/kurzbuch";
import { listenedFraction } from "../settings/tts/kidocs/kiDocs";
import { kiLangLabel } from "../settings/tts/kidocs/targetLang";

export type FormatKind = "text" | "audiobook" | "kurzbuch";

type T = (key: string, options?: Record<string, unknown>) => string;

/** Eine Fassung eines Buchs mit eigenem Fortschritt (Patrick 04.11.). */
export interface FormatEntry {
  kind: FormatKind;
  fraction: number;
  /** Kurzer Wert rechts: „4 %“, „noch 8 h 41 min“, „8 h 53 min“, „Fertig“. */
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

/** Kurzbuch: Hördauer aus der Wortzahl. */
const kurzbuchMs = (words: number) =>
  Math.round((Math.max(0, words) / LISTEN_WPM) * 60_000);

function timeValue(
  t: T,
  fraction: number,
  totalMs: number | null,
  leftMs: number | null,
): string {
  if (fraction >= 1) return t("library.status.finished");
  if (totalMs != null && totalMs > 0) {
    return fraction > 0
      ? t("library.status.left", {
          time: formatDuration(leftMs ?? totalMs * (1 - fraction)),
        })
      : formatDuration(totalMs);
  }
  return t("library.status.percent", { percent: Math.round(fraction * 100) });
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
  if (v.book.text) {
    const f = v.textFraction;
    const value =
      f >= 1
        ? t("library.status.finished")
        : t("library.status.percent", { percent: Math.round(f * 100) });
    out.push({
      kind: "text",
      fraction: f,
      value,
      finished: f >= 1,
      label: t("library.fmt.readTitle", { title, value }),
    });
  } else if (busyText) {
    out.push({
      kind: "text",
      fraction: 0,
      value: "",
      finished: false,
      busy: busyText,
      label: busyText,
    });
  }
  if (v.book.audio_path) {
    const f = v.audioFraction;
    const value = timeValue(t, f, v.audioTotalMs, v.audioLeftMs);
    out.push({
      kind: "audiobook",
      fraction: f,
      value,
      finished: f >= 1,
      label: t("library.fmt.listenTitle", { title, value }),
    });
  }
  if (kurzbuch) {
    const f = v.book.finished
      ? 1
      : listenedFraction(kurzbuch.page, kurzbuch.doc.id);
    const total = kurzbuchMs(kurzbuch.doc.words);
    const value = timeValue(
      t,
      f,
      total || null,
      total ? total * (1 - f) : null,
    );
    const lang =
      kurzbuch.langs > 1 ? kiLangLabel(kurzbuch.doc.language) : undefined;
    out.push({
      kind: "kurzbuch",
      fraction: f,
      value,
      finished: f >= 1,
      badge: lang || undefined,
      label: t("library.fmt.kurzbuchTitle", {
        title,
        value: lang ? `${lang}, ${value}` : value,
      }),
    });
  }
  return out;
}

/** Symbol und kurzer Wert einer Fassung, z. B. für „Weitermachen“. */
export const FormatValue: React.FC<{ entry: FormatEntry }> = ({ entry }) => {
  const { t } = useTranslation();
  const Icon = ICON[entry.kind];
  return (
    <span className="library-fmtvalue" data-kind={entry.kind}>
      <Icon size={13} aria-hidden="true" />
      <span className="library-fmtvalue__name">
        {t(`library.fmt.${entry.kind}`)}
      </span>
      <span className="library-fmtvalue__value">{entry.value}</span>
    </span>
  );
};

/**
 * Fassungen auf der Bibliothekskarte (Patrick 04.11.): je vorhandenem Format
 * eine Zeile mit Symbol, Namen, eigenem Fortschritt (Balken und Wert) und
 * direktem Start. Hörbuch zeigt Rest- bzw. Gesamtdauer, wann immer bekannt.
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
  const start = (kind: FormatKind) => {
    if (kind === "text" && view.book.text)
      openInReader(view.book.text.page_id, false);
    else if (kind === "audiobook") onListen();
    else if (kind === "kurzbuch" && kurzbuch)
      openKiDocInReader({
        page: kurzbuch.page,
        doc: kurzbuch.doc.id,
        play: true,
      });
  };
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
              start(e.kind);
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
