import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { BarChart3, Sparkles, X } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useFunctionLlm } from "@/lib/functionLlm";
import { classifyBook, saveClassification } from "@/lib/library/classify";
import {
  CATEGORIES,
  categoryLabel,
  formatDuration,
} from "@/lib/library/readingStats";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
import { useReadingRows } from "./ReadingStats";
import "./readingStats.css";

/**
 * Einordnung eines Buchs (lva#354): Kategorie aus den Vorlagen oder eigene,
 * freie Themen, KI-Vorschlag mit Übernehmen. Darunter, was die Statistik
 * zu diesem Buch kennt.
 */
export const BookClassification: React.FC<{
  book: LibraryBook;
  all: LibraryBook[];
  onSaved: () => void | Promise<void>;
  onOpenStats?: () => void;
}> = ({ book, all, onSaved, onOpenStats }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const llm = useFunctionLlm("read_aloud");
  const aiOn = getSetting("reading_stats_ai") ?? true;
  const { rows } = useReadingRows();
  const [busy, setBusy] = useState(false);
  const [topicDraft, setTopicDraft] = useState("");
  const topics = book.topics ?? [];
  const suggested = book.classified_by === "ki";

  const options = useMemo(() => {
    const own = new Set<string>();
    for (const b of all)
      if (b.category?.startsWith("eigen:")) own.add(b.category);
    return [
      ...CATEGORIES.map((c) => ({
        value: c.key,
        label: categoryLabel(c.key, t) ?? c.de,
      })),
      ...[...own].map((k) => ({ value: k, label: k.slice(6) })),
    ];
  }, [all, t]);

  const known = useMemo(() => {
    const s = new Set<string>();
    for (const b of all) for (const x of b.topics ?? []) s.add(x);
    return [...s].sort((a, b) => a.localeCompare(b));
  }, [all]);

  const read = useMemo(() => {
    const mine = (rows ?? []).filter((r) => r.book_id === book.id);
    if (!mine.length) return null;
    const days = new Set(mine.map((r) => r.day));
    const last = [...days].sort().pop() ?? "";
    return {
      sec: mine.reduce((a, r) => a + r.sec, 0),
      days: days.size,
      last: last.split("-").reverse().join("."),
    };
  }, [rows, book.id]);

  const store = async (category: string | null, next: string[]) => {
    setBusy(true);
    try {
      await saveClassification(book, category, next);
      await onSaved();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };

  const ask = async () => {
    setBusy(true);
    try {
      await classifyBook(book, all);
      await onSaved();
    } catch (e) {
      toast.error(t("readingStats.class.failed", { error: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const addTopic = () => {
    const v = topicDraft.trim();
    setTopicDraft("");
    if (!v || topics.some((x) => x.toLowerCase() === v.toLowerCase())) return;
    void store(book.category ?? null, [...topics, v].slice(0, 5));
  };

  return (
    <section
      className="rs-class"
      aria-labelledby="rs-class-h"
      data-testid="book-classification"
    >
      <h3 id="rs-class-h">{t("readingStats.class.label")}</h3>
      <div className="rs-class__row">
        <span className="rs-class__k" id="rs-class-cat">
          {t("readingStats.class.category")}
        </span>
        <div className="rs-class__v">
          <div className="rs-class__select">
            <Select
              value={book.category ?? null}
              options={options}
              placeholder={t("readingStats.class.none")}
              ariaLabel={t("readingStats.class.category")}
              disabled={busy}
              isClearable
              isCreatable
              formatCreateLabel={(name) =>
                t("readingStats.class.own", { name })
              }
              onCreateOption={(name) =>
                void store(`eigen:${name.trim()}`, topics)
              }
              onChange={(v) => void store(v, topics)}
            />
          </div>
          {suggested && (
            <span className="rs-ai" data-testid="book-classification-ai">
              <Sparkles size={12} aria-hidden="true" />
              {t("readingStats.class.suggested")}
            </span>
          )}
        </div>
      </div>
      <div className="rs-class__row">
        <span className="rs-class__k">{t("readingStats.class.topics")}</span>
        <div className="rs-class__v rs-class__topics">
          {topics.map((x) => (
            <span key={x} className="rs-tchip">
              {x}
              <button
                type="button"
                aria-label={t("readingStats.class.removeTopic", { name: x })}
                disabled={busy}
                onClick={() =>
                  void store(
                    book.category ?? null,
                    topics.filter((y) => y !== x),
                  )
                }
              >
                <X size={12} aria-hidden="true" />
              </button>
            </span>
          ))}
          {topics.length < 5 && (
            <>
              <input
                className="rs-tinput"
                list="rs-known-topics"
                value={topicDraft}
                placeholder={t("readingStats.class.addTopic")}
                aria-label={t("readingStats.class.addTopic")}
                disabled={busy}
                onChange={(e) => setTopicDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addTopic();
                  }
                }}
                onBlur={() => topicDraft.trim() && addTopic()}
              />
              <datalist id="rs-known-topics">
                {known.map((x) => (
                  <option key={x} value={x} />
                ))}
              </datalist>
            </>
          )}
        </div>
      </div>
      {suggested && (
        <div className="rs-suggest">
          <span>
            {t("readingStats.class.by", { model: llm.model ?? "KI" })}
          </span>
          <Button
            size="sm"
            variant="primary"
            disabled={busy}
            onClick={() => void store(book.category ?? null, topics)}
            data-testid="book-classification-accept"
          >
            {t("readingStats.class.accept")}
          </Button>
        </div>
      )}
      {aiOn && book.classified_by !== "hand" && !suggested && llm.model && (
        <div className="rs-row">
          <Button
            size="sm"
            variant="secondary"
            icon={<Sparkles size={16} />}
            disabled={busy}
            onClick={() => void ask()}
            data-testid="book-classification-ask"
          >
            {busy
              ? t("readingStats.class.asking")
              : t("readingStats.class.ask")}
          </Button>
          <span className="rs-muted">
            {llm.local
              ? t("readingStats.classify.local", {
                  model: llm.model,
                  connection: llm.connection,
                })
              : t("readingStats.classify.remote", {
                  model: llm.model,
                  connection: llm.connection,
                })}
          </span>
        </div>
      )}
      <div className="rs-readline">
        <BarChart3 size={16} aria-hidden="true" />
        <span>
          {read
            ? t("readingStats.class.read", {
                time: formatDuration(read.sec),
                day: read.last,
                count: read.days,
              })
            : t("readingStats.class.readNone")}
        </span>
        {onOpenStats && (
          <button
            type="button"
            className="rs-link rs-push"
            onClick={onOpenStats}
          >
            {t("readingStats.class.open")}
          </button>
        )}
      </div>
    </section>
  );
};
