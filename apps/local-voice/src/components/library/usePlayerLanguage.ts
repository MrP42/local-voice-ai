import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type LibraryBook } from "@/bindings";
import type { SwitchRequest } from "./AudioBar";
import { relativeTime } from "./EditionsSection";
import {
  audioChapters,
  audioKeyOf,
  deviceId,
  editionById,
  hasAudio,
  langName,
  langShort,
  readingEdition,
} from "@/lib/library/editions";
import { chapterOf } from "@/lib/library/languageSwitch";
import {
  flushProgress,
  loadProgress,
  removeProgress,
  saveProgress,
} from "@/lib/library/progress";
import {
  applySwitch,
  planSwitch,
  undoSwitch,
  type SwitchPlan,
} from "@/lib/library/switching";
import { dismissNotice, showNotice } from "@/lib/library/notice";

/** Die laufende Hörleiste: Buch, Fassung und wie sie (neu) starten soll. */
export interface Listening {
  book: LibraryBook;
  follow: boolean;
  /** Laufende Sprachfassung; `null` = zuletzt aktive. */
  edition: string | null;
  /** Nach einem Wechsel so weiterspielen, wie es vorher lief. */
  startPlaying: boolean;
  /** Leiste ist abgebaut und wird nach einem Rückgängig neu gestartet. */
  pending?: boolean;
  /** Nach einem Wechsel den Fokus auf den Sprachknopf setzen. */
  focusLanguage: boolean;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** „2:05“ bzw. „1:02:05“ aus Millisekunden. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const [h, m, sec] = [
    Math.floor(s / 3600),
    Math.floor((s % 3600) / 60),
    s % 60,
  ];
  return h ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

const range = (chapters: number[]) =>
  chapters.length > 1
    ? `${chapters[0] + 1}–${chapters[chapters.length - 1] + 1}`
    : String((chapters[0] ?? 0) + 1);

/** Kapitelnummer (1-basiert) einer Hörstelle, sofern die Teile Dauern haben. */
function chapterNumber(
  book: LibraryBook,
  editionId: string,
  offsetMs: number,
): number | null {
  const e = editionById(book, editionId);
  if (!e?.audio) return null;
  const c = audioChapters(e);
  return c.starts.length
    ? chapterOf({ starts: c.starts, length: c.length }, offsetMs) + 1
    : null;
}

type T = (key: string, options?: Record<string, unknown>) => string;

/** Text und Ton des Hinweises nach einem Wechsel (sf-06, sf-07). */
function describe(
  t: T,
  book: LibraryBook,
  plan: SwitchPlan,
): { text: string; tone: "info" | "warn" } {
  const to = editionById(book, plan.to);
  const from = editionById(book, plan.from);
  const lang = langName(to?.lang) || langShort(to?.lang);
  if (plan.level === "chapter") {
    const at = plan.within
      ? t("library.sf.notice.at", {
          at: clock(plan.within.at),
          length: clock(plan.within.length),
        })
      : "";
    return {
      text: t("library.sf.notice.chapter", {
        lang,
        chapter: range(plan.chapters),
        total: plan.chapterTotal,
        at,
      }),
      tone: "info",
    };
  }
  const base = t("library.sf.notice.approx", {
    lang,
    percent: Math.round(plan.fraction * 100),
  });
  const detail =
    plan.hasAlignment && plan.fromChapter !== null && plan.chapters.length
      ? t("library.sf.notice.openGroup", {
          a: plan.fromChapter + 1,
          langA: langName(from?.lang),
          b: range(plan.chapters),
          langB: langName(to?.lang),
        })
      : t("library.sf.notice.noAlignment");
  return { text: `${base} ${detail}`, tone: "warn" };
}

const deviceLabel = (t: T, device: string) =>
  /^(ios|iphone)/i.test(device)
    ? t("library.sf.conflict.iphone")
    : t("library.sf.conflict.other");

interface Options {
  books: LibraryBook[] | null;
  listening: Listening | null;
  setListening(
    next: Listening | null | ((prev: Listening | null) => Listening | null),
  ): void;
  reload(): Promise<void>;
  /** „Kapitel zuordnen“ für das Paar der laufenden Fassung öffnen. */
  openAlign(bookId: string, pair: { a: string; b: string }): void;
}

/**
 * Sprachwechsel in der Hörleiste (lva#344): rechnet die Stelle um, setzt Auswahl
 * und Fortschritt, baut die Leiste auf der Zielfassung neu auf und meldet das
 * Ergebnis mit „Rückgängig“. Fehlt das Hörbuch der Zielfassung auf diesem Gerät,
 * läuft die Wiedergabe unverändert weiter, der Abgleich (Download) wird
 * angestoßen. Meldet außerdem einen Konflikt, wenn ein anderes Gerät
 * inzwischen eine andere Sprachfassung gehört hat (sf-11).
 */
export function usePlayerLanguage({
  books,
  listening,
  setListening,
  reload,
  openAlign,
}: Options) {
  const { t } = useTranslation();
  const playing = listening
    ? (books?.find((b) => b.id === listening.book.id) ?? listening.book)
    : null;
  const edition =
    playing && listening
      ? (editionById(playing, listening.edition) ?? readingEdition(playing))
      : null;
  const [loading, setLoading] = useState<ReadonlySet<string>>(new Set());
  const busy = useRef(false);
  const currentId = useRef<string | undefined>(undefined);
  currentId.current = listening?.book.id;
  /** Nur das Buch ändern, für das die Aktion gestartet wurde. */
  const patch = useCallback(
    (id: string, next: Partial<Listening>) =>
      setListening((l) => (l && l.book.id === id ? { ...l, ...next } : l)),
    [setListening],
  );
  // Warten, bis die Leiste einer Fassung abgebaut ist und ihre Stelle gesichert hat.
  const waiters = useRef(new Map<string, () => void>());
  const onDispose = useCallback((editionId: string | undefined) => {
    if (!editionId) return;
    // Prüfstelle der Tests: eine Leiste, die ihr Sichern nicht bestätigt.
    if ((window as { __lvaHoldDispose?: boolean }).__lvaHoldDispose) return;
    waiters.current.get(editionId)?.();
    waiters.current.delete(editionId);
  }, []);
  /** `true`, sobald die Leiste abgebaut ist und gesichert hat; `false` nach Fristablauf. */
  const barGone = useCallback(
    (editionId: string) =>
      new Promise<boolean>((resolve) => {
        const timer = window.setTimeout(() => {
          waiters.current.delete(editionId);
          resolve(false);
        }, 2000);
        waiters.current.set(editionId, () => {
          window.clearTimeout(timer);
          resolve(true);
        });
      }),
    [],
  );
  // Fokus und Startzustand gelten nur für den Aufbau, der sie angefordert hat.
  useEffect(() => {
    if (listening && (listening.focusLanguage || !listening.startPlaying))
      setListening((l) =>
        l ? { ...l, focusLanguage: false, startPlaying: true } : l,
      );
  }, [listening, setListening]);

  // Geladen? Dann aus der Liste der ladenden Fassungen nehmen.
  useEffect(() => {
    if (!playing || loading.size === 0) return;
    const still = [...loading].filter((id) => {
      const e = editionById(playing, id);
      return e && !hasAudio(playing, e);
    });
    if (still.length !== loading.size) setLoading(new Set(still));
  }, [playing, loading]);

  const switchTo = useCallback(
    async (req: SwitchRequest) => {
      if (!playing || !edition || busy.current) return;
      const target = editionById(playing, req.to);
      if (!target) return;
      if (!hasAudio(playing, target)) {
        // Das Hörbuch fehlt hier: Abgleich anstoßen, die Wiedergabe läuft weiter.
        setLoading((prev) => new Set(prev).add(target.id));
        void commands.syncNow().catch(() => {});
        showNotice({
          owner: "library",
          tone: "info",
          text: t("library.sf.notice.loading", {
            lang: langName(target.lang) || langShort(target.lang),
            current: langName(edition.lang) || langShort(edition.lang),
          }),
          actions: [],
        });
        return;
      }
      busy.current = true;
      try {
        const plan = await planSwitch(
          playing,
          edition.id,
          target.id,
          "audio",
          req.ms,
          req.total,
        );
        if (!plan) throw new Error(t("library.sf.notice.failed"));
        const { book: updated, undo } = await applySwitch(
          playing,
          plan,
          { offset: req.ms, unit: "ms" },
          { wasPlaying: req.playing, total: undefined },
        );
        patch(playing.id, {
          book: updated,
          edition: target.id,
          startPlaying: req.playing,
          focusLanguage: true,
        });
        await reload();
        if (currentId.current !== playing.id) return;
        const said = describe(t as T, updated, plan);
        showNotice({
          owner: "library",
          tone: said.tone,
          text: said.text,
          actions: [
            ...(plan.level === "approx"
              ? [
                  {
                    id: "align",
                    label: t("library.sf.notice.align"),
                    run: () =>
                      openAlign(updated.id, { a: plan.from, b: plan.to }),
                  },
                ]
              : []),
            {
              id: "undo",
              label: t("library.sf.notice.undo"),
              run: async () => {
                const same = currentId.current === playing.id;
                const stay = () => {
                  if (same)
                    patch(playing.id, {
                      edition: target.id,
                      pending: false,
                      startPlaying: false,
                      focusLanguage: true,
                    });
                };
                try {
                  if (same) {
                    // Zielleiste abbauen, Quellleiste noch nicht starten.
                    const gone = barGone(target.id);
                    patch(playing.id, { pending: true });
                    if (!(await gone)) {
                      stay();
                      toast.error(t("library.sf.notice.undoBlocked"));
                      return;
                    }
                    await flushProgress();
                  }
                  const restored = await undoSwitch(undo);
                  if (same)
                    patch(playing.id, {
                      book: restored,
                      edition: undo.fromEdition,
                      pending: false,
                      startPlaying: undo.wasPlaying,
                      focusLanguage: true,
                    });
                } catch (e) {
                  stay();
                  toast.error(e instanceof Error ? e.message : String(e));
                }
                // Ein Fehler beim Neuladen ändert nichts am erfolgreichen Rückgängig.
                try {
                  await reload();
                } catch (e) {
                  toast.error(e instanceof Error ? e.message : String(e));
                }
              },
            },
          ],
        });
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        busy.current = false;
      }
    },
    [playing, edition, t, patch, barGone, reload, openAlign],
  );

  // Konflikt (sf-11): ein anderes Gerät hat inzwischen eine andere Fassung gehört.
  const sessionStart = useRef(Date.now());
  const seen = useRef(new Set<string>());
  const bookId = listening?.book.id;
  useEffect(() => {
    sessionStart.current = Date.now();
  }, [bookId]);
  // Ein Hinweis gehört zu seinem Buch: wird ein anderes gestartet, verschwindet er.
  useEffect(() => {
    dismissNotice();
  }, [bookId]);
  const reading = playing?.reading;
  useEffect(() => {
    if (!playing || !edition || !reading) return;
    if (reading.edition === edition.id) return;
    if (!reading.device || reading.device === deviceId()) return;
    if (reading.updated <= sessionStart.current) return;
    const stamp = `${playing.id}:${reading.updated}`;
    if (seen.current.has(stamp)) return;
    seen.current.add(stamp);
    const there = editionById(playing, reading.edition);
    if (!there) return;
    const key = audioKeyOf(playing, edition);
    const here = loadProgress(key)?.offset ?? 0;
    const theirs =
      reading.position?.unit === "ms" ? reading.position.offset : null;
    const chapterThere =
      theirs !== null ? chapterNumber(playing, there.id, theirs) : null;
    const chapterHere = chapterNumber(playing, edition.id, here);
    showNotice({
      owner: "library",
      tone: "alert",
      sticky: true,
      text: t("library.sf.conflict.text", {
        device: deviceLabel(t as T, reading.device),
        lang: langName(there.lang) || langShort(there.lang),
        there: [
          chapterThere
            ? t("library.sf.conflict.chapter", { n: chapterThere })
            : null,
          relativeTime(reading.updated),
        ]
          .filter(Boolean)
          .join(", "),
        current: langName(edition.lang) || langShort(edition.lang),
        here: chapterHere
          ? t("library.sf.conflict.chapterHere", { n: chapterHere })
          : "",
      }),
      actions: [
        {
          id: "go",
          label: t("library.sf.conflict.go"),
          run: async () => {
            if (currentId.current !== playing.id) return;
            if (!hasAudio(playing, there)) {
              // Das Hörbuch fehlt hier: Abgleich anstoßen, die Wiedergabe läuft weiter.
              setLoading((prev) => new Set(prev).add(there.id));
              void commands.syncNow().catch(() => {});
              toast(
                t("library.sf.notice.loading", {
                  lang: langName(there.lang) || langShort(there.lang),
                  current: langName(edition.lang) || langShort(edition.lang),
                }),
              );
              return;
            }
            const tk = audioKeyOf(playing, there);
            if (
              reading.position &&
              reading.position.key === tk &&
              reading.position.unit === "ms"
            ) {
              const rec = loadProgress(tk);
              // Die Stelle des anderen Geräts gilt; „fertig“ der Fassung weicht ihr.
              const { finished: _finished, ...rest } = rec ?? {
                heard: [],
                read: [],
                length: 0,
              };
              void _finished;
              saveProgress(tk, {
                ...rest,
                offset: reading.position.offset,
                length: rec?.length ?? 0,
                heard: rec?.heard ?? [],
                read: rec?.read ?? [],
                unit: "ms",
                updated: Date.now(),
              });
            }
            patch(playing.id, {
              edition: there.id,
              startPlaying: true,
              focusLanguage: true,
            });
          },
        },
        {
          id: "stay",
          label: t("library.sf.conflict.stay"),
          run: async () => {
            if (currentId.current !== playing.id) return;
            // Die Stelle gilt zum Zeitpunkt des Klicks, nicht der Anzeige.
            await flushProgress();
            const now = loadProgress(key)?.offset ?? here;
            const r = await commands.librarySetReading(playing.id, {
              edition: edition.id,
              updated: Date.now(),
              device: deviceId(),
              position: { key, unit: "ms", offset: now },
            });
            if (r.status !== "ok") toast.error(r.error);
            else await reload();
          },
        },
      ],
    });
  }, [playing, edition, reading, t, patch, reload]);

  // Beim Schließen der Leiste bleibt kein Hinweis stehen, der sich auf sie bezieht.
  useEffect(() => {
    if (!listening) dismissNotice();
  }, [listening]);

  return { playing, edition, loading, switchTo, onDispose };
}
