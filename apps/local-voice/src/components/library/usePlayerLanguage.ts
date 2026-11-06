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
        setListening((l) =>
          l
            ? {
                ...l,
                book: updated,
                edition: target.id,
                startPlaying: req.playing,
                focusLanguage: true,
              }
            : l,
        );
        await reload();
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
                try {
                  setListening((l) =>
                    l
                      ? {
                          ...l,
                          edition: undo.fromEdition,
                          startPlaying: undo.wasPlaying,
                          focusLanguage: true,
                        }
                      : l,
                  );
                  // Die Leiste der Zielfassung sichert beim Abbau ihre Stelle;
                  // erst danach den Stand der Zielfassung zurücksetzen.
                  await new Promise((resolve) =>
                    window.setTimeout(resolve, 60),
                  );
                  await flushProgress();
                  await undoSwitch(undo);
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
    [playing, edition, t, setListening, reload, openAlign],
  );

  // Konflikt (sf-11): ein anderes Gerät hat inzwischen eine andere Fassung gehört.
  const sessionStart = useRef(Date.now());
  const seen = useRef(new Set<string>());
  const bookId = listening?.book.id;
  useEffect(() => {
    sessionStart.current = Date.now();
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
            const tk = audioKeyOf(playing, there);
            if (reading.position && reading.position.key === tk) {
              const rec = loadProgress(tk);
              saveProgress(tk, {
                offset: reading.position.offset,
                length: rec?.length ?? 0,
                heard: rec?.heard ?? [],
                read: rec?.read ?? [],
                unit: "ms",
                updated: Date.now(),
              });
            }
            setListening((l) =>
              l
                ? {
                    ...l,
                    edition: there.id,
                    startPlaying: true,
                    focusLanguage: true,
                  }
                : l,
            );
          },
        },
        {
          id: "stay",
          label: t("library.sf.conflict.stay"),
          run: async () => {
            const r = await commands.librarySetReading(playing.id, {
              edition: edition.id,
              updated: Date.now(),
              device: deviceId(),
              position: { key, unit: "ms", offset: here },
            });
            if (r.status !== "ok") toast.error(r.error);
            else await reload();
          },
        },
      ],
    });
  }, [playing, edition, reading, t, setListening, reload]);

  // Beim Schließen der Leiste bleibt kein Hinweis stehen, der sich auf sie bezieht.
  useEffect(() => {
    if (!listening) dismissNotice();
  }, [listening]);

  return { playing, edition, loading, switchTo };
}
