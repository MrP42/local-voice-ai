import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { BookTemplate, SummarySource } from "@/bindings";
import { buildContents, type ContentsEntry } from "@/lib/reader/contents";
import { parseReaderDoc } from "@/lib/reader/readerDoc";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import { Select } from "../../../ui/Select";
import { KI_CATEGORIES } from "./kiDocs";
import "./kidocs.css";

type SourceKind = "book" | "chapter" | "selection" | "pick";

export interface KiSelection {
  start: number;
  end: number;
}

interface Chapter {
  title: string;
  start: number;
  end: number;
}

/** Kapitel der obersten beiden Gliederungsebenen mit ihrem Bereich im Text. */
export function chaptersOf(text: string, entries: ContentsEntry[]): Chapter[] {
  if (entries.length === 0) return [];
  const min = Math.min(...entries.map((e) => e.level));
  // Eine einzige oberste Überschrift ist der Buchtitel, kein Kapitel.
  const single =
    entries.filter((e) => e.level === min).length === 1 &&
    entries.some((e) => e.level > min);
  const top = single ? min + 1 : min;
  const out: Chapter[] = [];
  entries.forEach((e, i) => {
    if (e.level < top || e.level > top + 1) return;
    // Bereich ab Zeilenanfang (mit „#“), bis zur nächsten gleich hohen Überschrift.
    const start = text.lastIndexOf("\n", e.start - 1) + 1;
    const next = entries.slice(i + 1).find((n) => n.level <= e.level);
    const end = next ? text.lastIndexOf("\n", next.start - 1) + 1 : text.length;
    if (end > start) out.push({ title: e.title, start, end });
  });
  return out;
}

const shorten = (s: string, n = 48) =>
  s.length > n ? `${s.slice(0, n - 1).trimEnd()}…` : s;

/** Schätzung der Laufzeit für ein lokales Modell (16 000 Zeichen je Schritt). */
export function estimate(chars: number): {
  steps: number;
  min: number;
  max: number;
} {
  const parts = Math.max(1, Math.ceil(chars / 16000));
  const steps = parts + (parts > 1 ? Math.ceil(parts / 8) : 0);
  return {
    steps,
    min: Math.max(1, Math.round((steps * 10) / 60)),
    max: Math.max(1, Math.ceil((steps * 45) / 60)),
  };
}

/**
 * „Neues KI-Dokument“ (lva#183, Codex Must-fix 3): Quelle oben, Vorlagen in
 * Gruppen mit Suche in einem eigenen Scrollbereich, Aktionen fest unten,
 * Laufzeit ausdrücklich als Schätzung.
 */
export function KiDocumentDialog({
  open,
  onOpenChange,
  text,
  templates,
  readingAt,
  selection,
  onCreate,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Originaltext des Buchs. */
  text: string;
  templates: BookTemplate[];
  /** Gelesene Stelle im Original (für „Aktuelles Kapitel“), oder null. */
  readingAt: number | null;
  selection: KiSelection | null;
  onCreate: (
    source: SummarySource,
    sourceLabel: string,
    template: BookTemplate,
  ) => void;
}) {
  const { t } = useTranslation();
  const chapters = useMemo(
    () =>
      open ? chaptersOf(text, buildContents(parseReaderDoc(text, []))) : [],
    [open, text],
  );
  const currentChapter = useMemo(() => {
    if (chapters.length === 0) return -1;
    const at = readingAt ?? 0;
    let found = 0;
    chapters.forEach((c, i) => {
      if (c.start <= at) found = i;
    });
    return found;
  }, [chapters, readingAt]);

  const [kind, setKind] = useState<SourceKind>("book");
  const [picked, setPicked] = useState(0);
  const [query, setQuery] = useState("");
  const [templateId, setTemplateId] = useState("builtin:eine_seite");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setKind(selection ? "selection" : "book");
    setPicked(Math.max(0, currentChapter));
    setQuery("");
    // Nur beim Öffnen vorbelegen, nicht bei jeder Änderung der Auswahl.
  }, [open]);

  const source = useMemo((): {
    source: SummarySource;
    label: string;
    chars: number;
  } | null => {
    if (kind === "book")
      return {
        source: { kind: "book" },
        label: t("tts.kiDocs.src.book"),
        chars: text.length,
      };
    if (kind === "selection" && selection)
      return {
        source: { kind: "range", start: selection.start, end: selection.end },
        label: t("tts.kiDocs.src.selection"),
        chars: selection.end - selection.start,
      };
    const idx = kind === "chapter" ? currentChapter : picked;
    const c = chapters[idx];
    if (!c) return null;
    return {
      source: { kind: "range", start: c.start, end: c.end },
      label: shorten(c.title),
      chars: c.end - c.start,
    };
  }, [kind, selection, chapters, currentChapter, picked, text.length, t]);

  const groups = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    const match = (x: BookTemplate) =>
      !q ||
      x.title.toLocaleLowerCase().includes(q) ||
      x.description.toLocaleLowerCase().includes(q);
    const cats = [...KI_CATEGORIES, "eigene"];
    return cats
      .map((cat) => ({
        cat,
        items: templates.filter(
          (x) =>
            match(x) &&
            (cat === "eigene" ? !x.builtin : x.builtin && x.category === cat),
        ),
      }))
      .filter((g) => g.items.length > 0);
  }, [templates, query]);

  const template = templates.find((x) => x.id === templateId) ?? null;
  const est = source ? estimate(source.chars) : null;
  const canCreate = source !== null && template !== null && source.chars > 0;

  const sourceChip = (k: SourceKind, label: string, disabled = false) => (
    <button
      type="button"
      aria-pressed={kind === k}
      disabled={disabled}
      onClick={() => setKind(k)}
      className={`kidocs__chip ${kind === k ? "kidocs__chip--on" : ""} disabled:opacity-40`}
      data-testid={`kidoc-src-${k}`}
    >
      {label}
    </button>
  );

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.kiDocs.newTitle")}
      closeLabel={t("tts.kiDocs.cancel")}
      className="kidoc-new max-w-2xl"
      initialFocusRef={searchRef as React.RefObject<HTMLElement>}
      footer={
        <>
          <Button
            variant="primary"
            disabled={!canCreate}
            onClick={() =>
              source &&
              template &&
              onCreate(source.source, source.label, template)
            }
            data-testid="kidoc-create"
          >
            {t("tts.kiDocs.create")}
          </Button>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.kiDocs.cancel")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <section aria-labelledby="kidoc-src-h" className="flex flex-col gap-2">
          <h3 id="kidoc-src-h" className="text-sm font-semibold">
            {t("tts.kiDocs.src.title")}
          </h3>
          <div
            role="group"
            aria-labelledby="kidoc-src-h"
            className="kidoc-new__sources"
          >
            {sourceChip("book", t("tts.kiDocs.src.book"))}
            {sourceChip(
              "chapter",
              currentChapter >= 0
                ? t("tts.kiDocs.src.chapterNamed", {
                    title: shorten(chapters[currentChapter].title, 32),
                  })
                : t("tts.kiDocs.src.chapter"),
              currentChapter < 0,
            )}
            {sourceChip("selection", t("tts.kiDocs.src.selection"), !selection)}
            {sourceChip(
              "pick",
              t("tts.kiDocs.src.pick"),
              chapters.length === 0,
            )}
          </div>
          {kind === "pick" && chapters.length > 0 && (
            <Select
              menuPortal
              isClearable={false}
              value={String(picked)}
              options={chapters.map((c, i) => ({
                value: String(i),
                label: shorten(c.title, 80),
              }))}
              onChange={(v) => v !== null && setPicked(Number(v))}
              aria-label={t("tts.kiDocs.src.pick")}
            />
          )}
          {source && (
            <p className="kidoc-new__summary" data-testid="kidoc-src-summary">
              {t("tts.kiDocs.src.summary", {
                label: source.label,
                chars: source.chars.toLocaleString(),
              })}
            </p>
          )}
        </section>

        <section
          aria-labelledby="kidoc-tpl-h"
          className="flex min-h-0 flex-col gap-2"
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 id="kidoc-tpl-h" className="text-sm font-semibold">
              {t("tts.kiDocs.tpl.title")}
            </h3>
            <div className="relative">
              <Input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t("tts.kiDocs.tpl.search")}
                aria-label={t("tts.kiDocs.tpl.search")}
                className="kidocs__search"
                data-testid="kidoc-tpl-search"
              />
            </div>
          </div>
          <div
            role="group"
            aria-labelledby="kidoc-tpl-h"
            className="max-h-[22rem] overflow-y-auto pr-1"
            data-testid="kidoc-tpl-list"
          >
            {groups.length === 0 && (
              <p className="kidoc-new__desc py-4">{t("tts.kiDocs.tpl.none")}</p>
            )}
            {groups.map((g) => (
              <div key={g.cat}>
                <p className="kidoc-new__group">
                  {t(`tts.kiDocs.filter.${g.cat}`)}
                </p>
                {g.items.map((x) => (
                  <button
                    key={x.id}
                    type="button"
                    aria-pressed={x.id === templateId}
                    className="kidoc-new__tpl"
                    onClick={() => setTemplateId(x.id)}
                    data-testid={`kidoc-tpl-${x.id}`}
                  >
                    <span className="text-sm font-medium">{x.title}</span>
                    <span className="kidoc-new__desc">{x.description}</span>
                  </button>
                ))}
              </div>
            ))}
          </div>
        </section>

        {est && template && (
          <p className="kidoc-new__summary" data-testid="kidoc-estimate">
            {t(
              est.min === est.max
                ? "tts.kiDocs.estimateOne"
                : "tts.kiDocs.estimate",
              {
                template: template.title,
                steps: est.steps,
                min: est.min,
                max: est.max,
              },
            )}
          </p>
        )}
      </div>
    </Dialog>
  );
}
