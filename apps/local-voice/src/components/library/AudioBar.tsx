import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { useSectionActive } from "../shell/KeepAlive";
import {
  BookOpen,
  ListMusic,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  X,
} from "lucide-react";
import { commands, type AudioVersion, type LibraryBook } from "@/bindings";
import { IconButton } from "../ui/IconButton";
import { BookCover } from "./BookCover";
import { FollowPane, textStartOf } from "./FollowPane";
import { assetUrl, formatDuration, tracksOf } from "@/lib/library/books";
import {
  makeFollowMap,
  paragraphAt,
  splitParagraphs,
  textIsNewer,
} from "@/lib/library/follow";
import {
  audioProgressKey,
  flushProgress,
  loadProgress,
  recordPosition,
  textProgressKey,
} from "@/lib/library/progress";
import { saveResumePoint } from "@/lib/reader/resumePoint";
import { usePersistentState } from "@/hooks/usePersistentState";
import { probeDuration } from "@/lib/library/probe";
import { mergeLayers, nearAnchor, spanAt } from "@/lib/library/liveSync";
import { sentenceAt, sentencesOf, type Span } from "@/lib/library/sentences";
import { useLiveSync } from "@/lib/library/useLiveSync";

/** Wie oft die Stelle beim Hören gesichert wird. */
const SAVE_EVERY_MS = 2000;
/** Dauern (Dateiname → ms) in die Audiofassung eintragen. */
export function withDurations(
  audio: AudioVersion,
  durations: Map<string, number>,
): AudioVersion {
  if (!audio.tracks?.length) {
    const ms = durations.get(audio.file);
    return ms ? { ...audio, duration_ms: audio.duration_ms ?? ms } : audio;
  }
  return {
    ...audio,
    tracks: audio.tracks.map((t) => ({
      ...t,
      duration_ms: t.duration_ms ?? durations.get(t.file) ?? null,
    })),
  };
}

const startsOf = (ms: number[]) => {
  const out: number[] = [];
  let sum = 0;
  for (const d of ms) {
    out.push(sum);
    sum += d;
  }
  return out;
};

/** Teil und Stelle darin zu einer Gesamtzeit (ms). */
function locate(
  durations: number[],
  ms: number,
): { index: number; within: number } {
  const starts = startsOf(durations);
  let index = 0;
  for (let i = 0; i < durations.length; i++) {
    if (starts[i] > ms) break;
    index = i;
    // Ein Teil ohne bekannte Dauer: dahinter lässt sich nichts verorten.
    if (!(durations[i] > 0)) break;
  }
  return { index, within: Math.max(0, ms - starts[index]) };
}

/** So lange darf die Suche nach der Startstelle höchstens dauern. */
const SEARCH_TIMEOUT_MS = 30_000;

interface Props {
  book: LibraryBook;
  onClose(): void;
  /** Dauern am Buch sichern (useLibrary.save). */
  onSave?(
    id: string,
    update: (fresh: LibraryBook) => LibraryBook,
  ): Promise<LibraryBook | null>;
  /** Mitlesen gleich einschalten (z. B. „Hören und mitlesen“). */
  startFollow?: boolean;
  /** Höhe der Leiste samt Lesefläche, damit die Seite Platz lässt. */
  onHeight?(px: number): void;
}

/**
 * Hörleiste für ein importiertes Hörbuch (lva#152, lva#204). Mehrere Teile
 * laufen nacheinander und zählen als eine durchgehende Zeit; die Stelle wird
 * beim Hören, Pausieren, Springen und Schließen gesichert – gehörte Bereiche
 * bleiben gehört. Hat das Buch Text, zeigt „Text mitlesen“ den Absatz, der
 * gerade gehört wird.
 */
export const AudioBar: React.FC<Props> = ({
  book,
  onClose,
  onSave,
  startFollow = false,
  onHeight,
}) => {
  const { t } = useTranslation();
  const audio = useRef<HTMLAudioElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const lastSave = useRef(0);
  const key = audioProgressKey(book.id);
  const textKey = book.text ? textProgressKey(book.text.page_id) : null;

  const tracks = useMemo(() => tracksOf(book.audio), [book.audio]);
  const urls = useMemo(() => {
    const paths = book.audio_paths?.length
      ? book.audio_paths
      : book.audio_path
        ? [book.audio_path]
        : [];
    return tracks.map((_, i) => assetUrl(paths[i]));
  }, [book.audio_paths, book.audio_path, tracks]);

  /** Dauer je Teil in ms (0 = noch unbekannt). */
  const [dur, setDur] = useState<number[]>(() =>
    tracks.map((x) => x.duration_ms ?? 0),
  );
  const durRef = useRef(dur);
  durRef.current = dur;
  const [current, setCurrent] = useState<number | null>(null);
  const currentRef = useRef(0);
  const [playing, setPlaying] = useState(false);
  const [at, setAt] = useState(0);
  const wantPlay = useRef(true);
  const switching = useRef(false);
  /** Das Element überlebt das Abmelden des Refs beim Schließen. */
  const lastEl = useRef<HTMLAudioElement | null>(null);
  const pendingSeek = useRef<number | null>(null);
  const [partsOpen, setPartsOpen] = useState(false);

  const [followPref, setFollowPref] = usePersistentState<"on" | "off">(
    "library.followText",
    "off",
    (v) => v === "on" || v === "off",
  );
  const follow = !!book.text && followPref === "on";
  useEffect(() => {
    // „Hören und mitlesen“ merkt sich die Wahl.
    if (startFollow) setFollowPref("on");
  }, []);

  // Text des Buchs: für Mitlesen und für die gemeinsame Position.
  const [text, setText] = useState<string | null>(null);
  const [textReady, setTextReady] = useState(!book.text);
  useEffect(() => {
    if (!book.text) return;
    let stop = false;
    void commands.pageStateLoad(book.text.page_id).then((r) => {
      if (stop) return;
      if (r.status === "ok") {
        try {
          const state = JSON.parse(r.data) as { text?: unknown };
          if (typeof state.text === "string") setText(state.text);
        } catch {
          /* kein lesbarer Seitenstand: ohne Mitlesen weiter */
        }
      }
      setTextReady(true);
    });
    return () => {
      stop = true;
    };
  }, [book.text?.page_id]);

  const paragraphs = useMemo(() => (text ? splitParagraphs(text) : []), [text]);
  const total = dur.reduce((a, b) => a + b, 0);
  // Hörbuch-Sync: Text und Hörbuch ohne gemeinsames Transkript (EPUB/PDF zu
  // einem Hörbuch) werden beim Mitlesen live abgeglichen.
  const transcriptSync = book.audio?.sync ?? [];
  // Auch ohne offene Lesefläche: die Satzzeile über der Leiste braucht die Stelle.
  const live = useLiveSync({
    bookId: book.id,
    enabled: !!text && transcriptSync.length === 0,
    playing,
    positionMs: at * 1000,
    totalMs: total,
    hintOf: (ms) => mapRef.current.msToChars(ms),
  });
  // Vorrang: Hand-Stellen, dann Transkript, dann Live-Abgleich.
  const syncPoints = useMemo(
    () => mergeLayers(live.anchors, book.audio?.sync, live.points),
    [live.anchors, book.audio?.sync, live.points],
  );
  const map = useMemo(
    () => makeFollowMap(syncPoints, text?.length ?? 0, total),
    [syncPoints, text, total],
  );
  const mapRef = useRef(map);
  mapRef.current = map;
  /** Stelle nur geschätzt: weder Transkript noch Live-Abgleich deckt sie ab. */
  const approxNow =
    transcriptSync.length === 0 &&
    !spanAt(live.covered, at * 1000) &&
    !nearAnchor(live.anchors, at * 1000);
  /** Startstelle wird im Hörbuch gesucht („Hören und mitlesen“). */
  const [searching, setSearching] = useState(false);

  /** Gesamtzeit in ms aus dem Element. */
  const globalMs = useCallback(() => {
    const el = audio.current ?? lastEl.current;
    const starts = startsOf(durRef.current);
    return (starts[currentRef.current] ?? 0) + (el ? el.currentTime * 1000 : 0);
  }, []);

  const persist = useCallback(
    (force = false) => {
      const el = audio.current ?? lastEl.current;
      if (!el || !Number.isFinite(el.duration)) return;
      const now = Date.now();
      if (!force && now - lastSave.current < SAVE_EVERY_MS) return;
      lastSave.current = now;
      const ms = Math.round(globalMs());
      const allKnown = durRef.current.every((d) => d > 0);
      // Zuerst die Textstelle, dann das Hören: Letzteres ist dann das jüngere
      // und startet beim nächsten Mal nicht an der gröberen Textstelle.
      if (text && textKey && book.text) {
        const chars = mapRef.current.msToChars(ms);
        saveResumePoint(`${book.text.page_id}:original`, text, chars);
      }
      recordPosition(key, {
        to: ms,
        length: allKnown ? durRef.current.reduce((a, b) => a + b, 0) : 0,
        unit: "ms",
      });
      if (force) void flushProgress();
    },
    [globalMs, key, text, textKey, book.text],
  );
  const persistRef = useRef(persist);
  persistRef.current = persist;

  // Startstelle: gesicherte Hörstelle, außer der Text ist klar neuer gelesen.
  // Fehlende Dauern vorher ermitteln, sonst ließe sich die Stelle nicht finden.
  // Ohne Transkript wird die gelesene Stelle im Hörbuch gesucht (Erkennung),
  // statt sie nur anteilig zu schätzen.
  const started = useRef(false);
  useEffect(() => {
    if (!textReady || !live.loaded || started.current) return;
    started.current = true;
    let stop = false;
    let done = false;
    const abort = { aborted: false };
    void (async () => {
      // Erst nach dem Effekt-Durchlauf (StrictMode ruft ihn zweimal).
      await Promise.resolve();
      if (stop) return;
      const saved = loadProgress(key);
      const textRec = textKey ? loadProgress(textKey) : null;
      let ms = saved && !saved.finished ? saved.offset : 0;
      const fromText =
        !!text &&
        !!textRec &&
        textRec.offset > 0 &&
        !textRec.finished &&
        (!saved || textIsNewer(textRec.updated, saved.updated));
      // Gesamtdauer zuerst: ohne sie lässt sich Text nicht auf Zeit abbilden.
      if ((ms > 0 || fromText) && durRef.current.some((d) => d <= 0)) {
        const measured = await measureMissing();
        if (stop) return;
        if (measured) durRef.current = measured;
      }
      if (fromText && text && textRec) {
        const totalNow = durRef.current.reduce((a, b) => a + b, 0);
        const m = makeFollowMap(syncPoints, text.length, totalNow);
        ms = Math.round(m.charsToMs(textRec.offset));
        const sure =
          transcriptSync.length > 0 ||
          !!spanAt(live.covered, ms) ||
          nearAnchor(live.anchors, ms);
        if (!sure && totalNow > 0) {
          setSearching(true);
          abort.aborted = false;
          let timer = 0;
          const found = await Promise.race([
            live.find(textRec.offset, ms, {
              msPerChar: totalNow / Math.max(1, text.length),
              totalMs: totalNow,
              abort,
            }),
            new Promise<null>((r) => {
              timer = window.setTimeout(() => r(null), SEARCH_TIMEOUT_MS);
            }),
          ]);
          window.clearTimeout(timer);
          // Zeitlimit oder Abbruch: die Suche hört nach dem laufenden Fenster auf.
          abort.aborted = true;
          setSearching(false);
          if (stop) return;
          if (found !== null) ms = Math.round(found);
        }
      }
      if (stop) return;
      const place = locate(durRef.current, ms);
      pendingSeek.current = place.within > 0 ? place.within / 1000 : null;
      currentRef.current = place.index;
      done = true;
      setCurrent(place.index);
    })();
    return () => {
      stop = true;
      abort.aborted = true;
      // Abgebrochen, bevor die Stelle feststand: beim nächsten Lauf neu.
      if (!done) started.current = false;
    };
  }, [textReady, live.loaded]);

  /** Dauern aller Teile ermitteln, die noch fehlen; speichert sie am Buch. */
  const measuring = useRef<Promise<number[] | null> | null>(null);
  const measureMissing = useCallback((): Promise<number[] | null> => {
    if (measuring.current) return measuring.current;
    const task = (async () => {
      const next = [...durRef.current];
      const found = new Map<string, number>();
      for (let i = 0; i < next.length; i++) {
        const url = urls[i];
        if (next[i] > 0 || !url) continue;
        const ms = await probeDuration(url);
        if (ms) {
          next[i] = ms;
          found.set(tracks[i].file, ms);
          setDur([...next]);
          durRef.current = [...next];
        }
      }
      if (found.size && onSave) {
        try {
          await onSave(book.id, (fresh) =>
            fresh.audio
              ? { ...fresh, audio: withDurations(fresh.audio, found) }
              : fresh,
          );
        } catch {
          /* Dauern bleiben im Speicher; beim nächsten Mal erneut */
        }
      }
      return found.size ? next : null;
    })();
    measuring.current = task;
    void task.finally(() => {
      measuring.current = null;
    });
    return task;
  }, [urls, tracks, onSave, book.id]);

  // Dauern im Hintergrund nachholen, sobald das Abspielen läuft.
  useEffect(() => {
    if (current === null || dur.every((d) => d > 0)) return;
    void measureMissing();
  }, [current]);

  /** Zu einer Gesamtzeit (Sekunden) springen, auch in ein anderes Teil. */
  const seekGlobal = useCallback(
    (seconds: number, measured = false) => {
      const el = audio.current;
      const durs = durRef.current;
      const known = durs.findIndex((d) => !(d > 0));
      // Sprung hinter ein Teil ohne bekannte Dauer: erst messen, dann springen.
      if (known >= 0 && !measured) {
        const knownMs = durs.slice(0, known).reduce((a, b) => a + b, 0);
        if (seconds * 1000 > knownMs) {
          void measureMissing().then(() => seekGlobal(seconds, true));
          return;
        }
      }
      const totalMs = durs.reduce((a, b) => a + b, 0);
      const ms = Math.max(
        0,
        Math.min(seconds * 1000, totalMs || seconds * 1000),
      );
      const place = locate(durs, ms);
      if (place.index === currentRef.current && el) {
        el.currentTime = place.within / 1000;
        return;
      }
      switching.current = true;
      pendingSeek.current = place.within / 1000;
      currentRef.current = place.index;
      setCurrent(place.index);
    },
    [measureMissing],
  );

  const skip = (seconds: number) => seekGlobal(globalMs() / 1000 + seconds);
  const toggle = () => {
    const el = audio.current;
    if (!el) return;
    if (el.paused) {
      wantPlay.current = true;
      void el.play().catch(() => {});
    } else {
      wantPlay.current = false;
      el.pause();
    }
  };

  // Die Bibliothek bleibt beim Bereichswechsel erhalten (KeepAlive); ohne
  // sichtbare Leiste hört niemand weiter: anhalten und die Stelle sichern.
  const sectionActive = useSectionActive();
  useEffect(() => {
    if (sectionActive) return;
    // Auch während des Ladens: kein späteres Anlaufen im Verborgenen.
    wantPlay.current = false;
    const el = audio.current;
    if (!el || el.paused) return;
    el.pause();
    persistRef.current(true);
  }, [sectionActive]);

  useEffect(() => {
    return () => {
      // Beim Schließen sichern, solange das Element noch Zeit kennt.
      persistRef.current(true);
    };
  }, []);

  // Höhe melden: die Seite lässt unten Platz für Leiste und Lesefläche.
  useEffect(() => {
    const el = root.current;
    if (!el || !onHeight) return;
    const report = () => onHeight(Math.ceil(el.getBoundingClientRect().height));
    report();
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => {
      observer.disconnect();
      onHeight(0);
    };
  }, [onHeight]);

  const src = current === null ? null : urls[current];
  const nowChars = map.msToChars(at * 1000);
  const activeParagraph = paragraphs.length
    ? paragraphAt(paragraphs, nowChars)
    : -1;

  /** Sätze je Absatz, erst bei Bedarf zerlegt. */
  const sentenceCache = useMemo(() => new Map<number, Span[]>(), [paragraphs]);
  const sentenceOf = useCallback(
    (chars: number): Span | null => {
      if (!paragraphs.length) return null;
      const i = paragraphAt(paragraphs, chars);
      let list = sentenceCache.get(i);
      if (!list) {
        const p = paragraphs[i];
        list = sentencesOf(p.text, textStartOf(p));
        sentenceCache.set(i, list);
      }
      return sentenceAt(list, chars);
    },
    [paragraphs, sentenceCache],
  );
  const sentence = activeParagraph >= 0 ? sentenceOf(nowChars) : null;
  /** Gerade gehörter Einschub (Text nur im Hörbuch). */
  const insertNow = live.inserts.find(
    (i) => at * 1000 >= i.start_ms && at * 1000 < i.end_ms,
  );
  const lineText =
    insertNow?.text ??
    (sentence && text ? text.slice(sentence.start, sentence.end) : "");

  if (!urls.length || urls.every((u) => !u)) return null;
  const multi = tracks.length > 1;
  const partNo = (current ?? 0) + 1;
  return (
    <div
      className="library-player"
      ref={root}
      data-follow={follow && paragraphs.length ? "on" : "off"}
      data-testid="library-player"
    >
      {follow && paragraphs.length > 0 && (
        <FollowPane
          title={book.title}
          paragraphs={paragraphs}
          active={activeParagraph}
          sentence={sentence}
          inserts={live.inserts}
          approx={approxNow}
          searching={searching}
          sentenceOf={sentenceOf}
          onPick={(chars) => {
            wantPlay.current = true;
            seekGlobal(map.charsToMs(chars) / 1000);
            const el = audio.current;
            if (el?.paused) void el.play().catch(() => {});
          }}
          onAnchor={
            transcriptSync.length === 0
              ? (chars) => void live.anchor(chars, globalMs())
              : undefined
          }
          onClose={() => setFollowPref("off")}
        />
      )}
      {partsOpen && multi && (
        <div
          className="library-parts"
          role="region"
          aria-label={t("library.parts.list")}
          data-testid="library-parts"
        >
          <ol>
            {tracks.map((track, i) => (
              <li key={track.file}>
                <button
                  type="button"
                  className="library-parts__item"
                  aria-current={i === current ? "true" : undefined}
                  onClick={() => {
                    wantPlay.current = true;
                    seekGlobal(startsOf(dur)[i] / 1000);
                    setPartsOpen(false);
                  }}
                  data-testid="library-parts-item"
                >
                  <span className="library-parts__no">{i + 1}</span>
                  <span className="library-parts__title">
                    {track.title || track.file}
                  </span>
                  <span className="library-meta">
                    {dur[i] > 0 ? formatDuration(dur[i]) : ""}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        </div>
      )}
      {!follow && text && (lineText || searching) && (
        <button
          type="button"
          className="library-audiobar__line"
          onClick={() => setFollowPref("on")}
          aria-label={t("library.follow.lineLabel", {
            text: searching ? t("library.follow.searching") : lineText,
          })}
          title={searching ? undefined : lineText}
          data-testid="library-audiobar-line"
        >
          {searching ? (
            <span className="library-meta">
              {t("library.follow.searching")}
            </span>
          ) : (
            <>
              {approxNow && (
                <span
                  className="library-audiobar__approx"
                  title={t("library.follow.approx")}
                  data-testid="library-audiobar-line-approx"
                >
                  ≈
                </span>
              )}
              {insertNow ? (
                <ins className="library-follow__ins">{lineText}</ins>
              ) : (
                <span className="library-audiobar__sentence">{lineText}</span>
              )}
            </>
          )}
        </button>
      )}
      <div
        className="library-audiobar"
        role="region"
        aria-label={t("library.audiobar", { title: book.title })}
        data-testid="library-audiobar"
      >
        <BookCover book={book} size="xs" />
        <div style={{ minWidth: 0, maxWidth: 220 }}>
          <div className="library-title" style={{ WebkitLineClamp: 1 }}>
            {book.title}
          </div>
          <div className="library-meta" data-testid="library-audiobar-sub">
            {multi
              ? t("library.parts.label", { n: partNo, total: tracks.length })
              : book.authors.join(", ")}
          </div>
        </div>
        {src && (
          <audio
            ref={audio}
            src={src}
            autoPlay={wantPlay.current}
            preload="metadata"
            onLoadedMetadata={(e) => {
              const el = e.currentTarget;
              lastEl.current = el;
              switching.current = false;
              // Dauer des Teils aus der Datei, wenn sie noch fehlt.
              const i = currentRef.current;
              if (
                Number.isFinite(el.duration) &&
                el.duration > 0 &&
                !(durRef.current[i] > 0)
              ) {
                const next = [...durRef.current];
                next[i] = Math.round(el.duration * 1000);
                durRef.current = next;
                setDur(next);
              }
              if (pendingSeek.current !== null) {
                el.currentTime = pendingSeek.current;
                pendingSeek.current = null;
              }
              if (wantPlay.current) void el.play().catch(() => {});
              setAt(globalMs() / 1000);
            }}
            onTimeUpdate={() => {
              setAt(globalMs() / 1000);
              persist();
            }}
            onPlay={() => {
              wantPlay.current = true;
              setPlaying(true);
            }}
            onPause={(e) => {
              setPlaying(false);
              // Ein Teil, das zu Ende ist, pausiert nur kurz vor dem nächsten.
              if (!switching.current && !e.currentTarget.ended)
                wantPlay.current = false;
              persist(true);
            }}
            onSeeked={() => {
              setAt(globalMs() / 1000);
              persist(true);
            }}
            onEnded={() => {
              persist(true);
              let next = currentRef.current + 1;
              while (next < urls.length && !urls[next]) next++;
              if (next >= urls.length) {
                wantPlay.current = false;
                return;
              }
              switching.current = true;
              currentRef.current = next;
              setCurrent(next);
            }}
          />
        )}
        {/* Eigene Bedienung statt der eingebauten Leiste: alle Trefferflächen
            mindestens 44 px hoch (Codex-Auflage). */}
        <label className="library-audiobar__seek">
          <span className="library-meta">{formatDuration(at * 1000)}</span>
          <input
            type="range"
            min={0}
            max={Math.max(total / 1000, 0.001)}
            step={1}
            value={Math.min(at, total / 1000)}
            disabled={!total}
            aria-label={t("library.seek")}
            aria-valuetext={`${formatDuration(at * 1000)} / ${formatDuration(total)}`}
            onChange={(e) => seekGlobal(Number(e.target.value))}
            data-testid="library-audiobar-seek"
          />
          <span className="library-meta">{formatDuration(total)}</span>
        </label>
        <div className="library-audiobar__keys">
          <IconButton
            icon={<RotateCcw size={18} aria-hidden="true" />}
            label={t("library.back30")}
            onClick={() => skip(-30)}
            data-testid="library-audiobar-back"
          />
          <IconButton
            icon={
              playing ? (
                <Pause size={20} aria-hidden="true" />
              ) : (
                <Play size={20} aria-hidden="true" />
              )
            }
            label={playing ? t("library.pause") : t("library.play")}
            onClick={toggle}
            data-testid="library-audiobar-toggle"
          />
          <IconButton
            icon={<RotateCw size={18} aria-hidden="true" />}
            label={t("library.forward30")}
            onClick={() => skip(30)}
            data-testid="library-audiobar-forward"
          />
          {multi && (
            <IconButton
              icon={<ListMusic size={18} aria-hidden="true" />}
              label={t("library.parts.list")}
              onClick={() => setPartsOpen((v) => !v)}
              aria-expanded={partsOpen}
              data-testid="library-audiobar-parts"
            />
          )}
          {book.text && (
            <IconButton
              icon={<BookOpen size={18} aria-hidden="true" />}
              label={follow ? t("library.follow.off") : t("library.follow.on")}
              onClick={() => setFollowPref(follow ? "off" : "on")}
              aria-pressed={follow}
              variant={follow ? "secondary" : "ghost"}
              data-testid="library-audiobar-follow"
            />
          )}
          <IconButton
            icon={<X size={18} aria-hidden="true" />}
            label={t("library.audiobarClose")}
            onClick={onClose}
            data-testid="library-audiobar-close"
          />
        </div>
      </div>
    </div>
  );
};
