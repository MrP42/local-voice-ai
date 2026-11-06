import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Languages } from "lucide-react";
import type { LanguagePair, LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import {
  describeBook,
  joinEditions,
  languagePairs,
  pairTarget,
} from "@/lib/library/editionOps";
import { editionsOf, langName } from "@/lib/library/editions";
import "./sprachfassungen.css";

export interface ResolvedPair {
  pair: LanguagePair;
  a: LibraryBook;
  b: LibraryBook;
}

/** Erkannte Paare (nie automatisch zusammenlegen): bei jeder Änderung der Bücher neu gefragt. */
export function useLanguagePairs(books: LibraryBook[] | null) {
  const [pairs, setPairs] = useState<LanguagePair[]>([]);
  const [later, setLater] = useState(false);
  const signature = (books ?? [])
    .map((b) => `${b.id}:${editionsOf(b).length}`)
    .join(",");
  useEffect(() => {
    let stale = false;
    if (!books || books.length < 2) {
      setPairs([]);
      return;
    }
    void languagePairs().then((list) => {
      if (!stale) setPairs(list);
    });
    return () => {
      stale = true;
    };
    // Nur bei geänderter Zusammensetzung der Bibliothek neu fragen.
  }, [signature]);
  const resolved = useMemo<ResolvedPair[]>(
    () =>
      pairs.flatMap((pair) => {
        const a = books?.find((b) => b.id === pair.a);
        const b = books?.find((x) => x.id === pair.b);
        return a && b ? [{ pair, a, b }] : [];
      }),
    [pairs, books],
  );
  return { pairs: resolved, later, postpone: () => setLater(true) };
}

/** Zusammenlegen der gewählten Paare nacheinander; jedes Paar mit eigenem Ziel (zuletzt benutzt). */
export async function joinPairs(
  list: ResolvedPair[],
): Promise<{ done: number; error: string | null }> {
  let done = 0;
  const gone = new Set<string>();
  for (const { a, b } of list) {
    if (gone.has(a.id) || gone.has(b.id)) continue;
    const { target, source } = pairTarget(a, b);
    try {
      await joinEditions(target.id, [source.id]);
      gone.add(source.id);
      done += 1;
    } catch (e) {
      return { done, error: e instanceof Error ? e.message : String(e) };
    }
  }
  return { done, error: null };
}

const titleOf = (b: LibraryBook) => b.title;

/** Hinweisleiste über der Bibliothek (sf-01). */
export const PairsBar: React.FC<{
  pairs: ResolvedPair[];
  busy: boolean;
  onJoin(pair: ResolvedPair): void;
  onReview(): void;
  onLater(): void;
}> = ({ pairs, busy, onJoin, onReview, onLater }) => {
  const { t } = useTranslation();
  if (!pairs.length) return null;
  const first = pairs.find((p) => p.pair.sure) ?? pairs[0];
  const sure = first.pair.sure;
  const langOf = (b: LibraryBook) => langName(editionsOf(b)[0]?.lang);
  return (
    <div
      className="library-mergebar sf-pairsbar"
      role="status"
      data-testid="sf-pairs-bar"
    >
      <Languages size={18} aria-hidden="true" />
      <span className="library-mergebar__text">
        <b>
          {t(sure ? "library.sf.pairs.barSure" : "library.sf.pairs.barMaybe")}
        </b>{" "}
        {t("library.sf.pairs.barBooks", {
          a: titleOf(first.a),
          langA: langOf(first.a),
          b: titleOf(first.b),
          langB: langOf(first.b),
        })}{" "}
        {sure && first.pair.reason === "work_key"
          ? t("library.sf.pairs.sameWork")
          : ""}
        {pairs.length > 1
          ? ` ${t("library.sf.pairs.more", { count: pairs.length - 1 })}`
          : ""}
      </span>
      <Button variant="ghost" onClick={onLater} data-testid="sf-pairs-later">
        {t("library.sf.pairs.later")}
      </Button>
      <Button onClick={onReview} data-testid="sf-pairs-review">
        {t("library.sf.pairs.reviewAll")}
      </Button>
      {sure && (
        <Button
          variant="primary"
          disabled={busy}
          onClick={() => onJoin(first)}
          data-testid="sf-pairs-join"
        >
          {t("library.sf.pairs.join")}
        </Button>
      )}
    </div>
  );
};

/** Prüfen-Dialog (sf-02): sichere Paare vorausgewählt, unsichere nicht. */
export const PairsDialog: React.FC<{
  open: boolean;
  pairs: ResolvedPair[];
  busy: boolean;
  onConfirm(chosen: ResolvedPair[]): void;
  onCancel(): void;
}> = ({ open, pairs, busy, onConfirm, onCancel }) => {
  const { t } = useTranslation();
  const keyOf = (p: ResolvedPair) => `${p.pair.a}|${p.pair.b}`;
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const reset = useCallback(
    () => setChecked(new Set(pairs.filter((p) => p.pair.sure).map(keyOf))),
    [pairs],
  );
  useEffect(() => {
    if (open) reset();
  }, [open, reset]);
  const chosen = pairs.filter((p) => checked.has(keyOf(p)));
  const kinds = {
    audio: t("library.sf.kind.audio"),
    text: t("library.sf.kind.text"),
  };
  const why = (p: ResolvedPair) => {
    const author = p.a.authors[0] ?? "";
    if (p.pair.reason === "work_key")
      return t("library.sf.pairs.whyWork", {
        key:
          p.a.work_key ??
          editionsOf(p.a).find((e) => e.work_key)?.work_key ??
          "",
      });
    if (p.pair.reason === "title_author")
      return t("library.sf.pairs.whyTitle", { author });
    return t("library.sf.pairs.whyAuthor", { author });
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !busy && onCancel()}
      title={t("library.sf.pairs.title")}
      closeLabel={t("library.cancel")}
      className="library-dialog-touch"
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            {t("library.cancel")}
          </Button>
          <Button
            variant="primary"
            disabled={busy || chosen.length === 0}
            onClick={() => onConfirm(chosen)}
            data-testid="sf-pairs-confirm"
          >
            {t("library.sf.pairs.confirm", { count: chosen.length })}
          </Button>
        </>
      }
    >
      <div className="sf-pairs" data-testid="sf-pairs-dialog">
        <p>{t("library.sf.pairs.intro")}</p>
        {pairs.map((p) => {
          const key = keyOf(p);
          return (
            <label className="sf-pair" key={key}>
              <input
                type="checkbox"
                checked={checked.has(key)}
                onChange={(e) =>
                  setChecked((prev) => {
                    const next = new Set(prev);
                    if (e.target.checked) next.add(key);
                    else next.delete(key);
                    return next;
                  })
                }
                data-testid="sf-pair-check"
              />
              <span className="sf-pair__body">
                {[p.a, p.b].map((b) => (
                  <span key={b.id}>
                    <b>{b.title}</b>{" "}
                    <span className="sf-pair__why">
                      {describeBook(b, kinds)}
                    </span>
                  </span>
                ))}
                <span
                  className="sf-badge"
                  data-tone={p.pair.sure ? "ok" : "warn"}
                >
                  {t(
                    p.pair.sure
                      ? "library.sf.pairs.sure"
                      : "library.sf.pairs.unsure",
                  )}
                </span>
                <span className="sf-pair__why">{why(p)}</span>
              </span>
            </label>
          );
        })}
        <p className="sf-pairs__foot">{t("library.sf.pairs.foot")}</p>
      </div>
    </Dialog>
  );
};

/** Alle Paare (sicher und unsicher), nach dem Zusammenlegen gemeldet. */
export function reportJoined(
  t: (key: string, options?: Record<string, unknown>) => string,
  result: { done: number; error: string | null },
): void {
  if (result.done > 0)
    toast(t("library.sf.pairs.joined", { count: result.done }));
  if (result.error) toast.error(result.error);
}
