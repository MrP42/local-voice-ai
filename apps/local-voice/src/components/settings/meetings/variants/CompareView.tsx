import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type StoredSegment,
  type TranscriptVariant,
} from "@/bindings";
import { Button } from "../../../ui/Button";
import { Alert } from "../../../ui/Alert";
import { Select } from "../../../ui/Select";
import { alignDiff } from "./diff";
import { translateVariantError, variantKindLabel } from "./useVariants";

interface CompareViewProps {
  meetingId: string;
  variants: TranscriptVariant[];
  /** Eine Fassung wurde aktiv gesetzt: Transkript und Liste neu laden. */
  onActivated: (variant: TranscriptVariant) => Promise<void> | void;
  /** Die Zusammenfuehrung hat eine dritte Fassung angelegt (Liste ist danach neu). */
  onMerged: (variant: TranscriptVariant) => Promise<void> | void;
}

const formatMmSs = (ms: number) => {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
};

/**
 * Reiter „Vergleich“ der Arbeitsflaeche: zwei Fassungen wortweise gegeneinander
 * (Einfuegungen und Loeschungen markiert, nicht nur farbig: Unterstreichung bzw.
 * Durchstreichung), „Fassung waehlen“ und „Zusammenfuehren“.
 */
export const CompareView: React.FC<CompareViewProps> = ({
  meetingId,
  variants,
  onActivated,
  onMerged,
}) => {
  const { t } = useTranslation();
  const [aId, setAId] = useState("");
  const [bId, setBId] = useState("");
  const [a, setA] = useState<StoredSegment[] | null>(null);
  const [b, setB] = useState<StoredSegment[] | null>(null);
  const [busy, setBusy] = useState<"merge" | "choose" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const load = useRef(0);

  // Vorbelegung: A = aktive Fassung, B = die erste andere. Bleibt eine Wahl
  // gueltig, bleibt sie.
  useEffect(() => {
    const ids = variants.map((v) => v.id);
    const active = variants.find((v) => v.active)?.id ?? ids[0] ?? "";
    const nextA = ids.includes(aId) ? aId : active;
    const other =
      ids.includes(bId) && bId !== nextA
        ? bId
        : (ids.find((id) => id !== nextA) ?? "");
    if (nextA !== aId) setAId(nextA);
    if (other !== bId) setBId(other);
  }, [variants, aId, bId]);

  useEffect(() => {
    if (!aId || !bId || aId === bId) {
      setA(null);
      setB(null);
      return;
    }
    const seq = ++load.current;
    setA(null);
    setB(null);
    void Promise.all([
      commands.transcriptVariantSegments(aId),
      commands.transcriptVariantSegments(bId),
    ]).then(([ra, rb]) => {
      if (seq !== load.current) return;
      if (ra.status === "ok" && rb.status === "ok") {
        setA(ra.data ?? []);
        setB(rb.data ?? []);
      } else {
        setError(
          translateVariantError(
            ra.status === "error"
              ? ra.error
              : rb.status === "error"
                ? rb.error
                : "",
            t,
          ),
        );
      }
    });
  }, [aId, bId, t]);

  const rows = useMemo(() => (a && b ? alignDiff(a, b) : null), [a, b]);
  const changed = rows?.filter((r) => r.changed).length ?? 0;

  const byId = (id: string) => variants.find((v) => v.id === id);
  const optionLabel = (v: TranscriptVariant) =>
    t("meetings.variants.itemLabel", {
      number: v.number,
      kind: variantKindLabel(v, t),
      count: v.segment_count,
    });

  const options = variants.map((v) => ({ value: v.id, label: optionLabel(v) }));

  const choose = async (id: string) => {
    setBusy("choose");
    setError(null);
    setNotice(null);
    const result = await commands.transcriptVariantActivate(id);
    setBusy(null);
    if (result.status !== "ok") {
      setError(translateVariantError(result.error, t));
      return;
    }
    setNotice(t("meetings.variants.activated", { number: result.data.number }));
    await onActivated(result.data);
  };

  const merge = async () => {
    setBusy("merge");
    setError(null);
    setNotice(null);
    const result = await commands.transcriptVariantsMerge(meetingId, aId, bId);
    setBusy(null);
    if (result.status !== "ok") {
      setError(translateVariantError(result.error, t));
      return;
    }
    await onMerged(result.data);
    setBId(result.data.id);
    setNotice(
      t("meetings.variants.diff.merged", { number: result.data.number }),
    );
  };

  if (variants.length < 2) {
    return (
      <p className="text-sm text-text/60" data-testid="compare-need">
        {t("meetings.variants.diff.need")}
      </p>
    );
  }

  return (
    <div className="space-y-3" data-testid="compare-view">
      <div className="flex flex-wrap items-end gap-3">
        <div className="flex min-w-[12rem] flex-col gap-1 text-xs text-text/70">
          <span>{t("meetings.variants.diff.baseLabel")}</span>
          <div data-testid="compare-a">
            <Select
              value={aId || null}
              options={options}
              ariaLabel={t("meetings.variants.diff.baseLabel")}
              onChange={(value) => value && setAId(value)}
            />
          </div>
        </div>
        <div className="flex min-w-[12rem] flex-col gap-1 text-xs text-text/70">
          <span>{t("meetings.variants.diff.otherLabel")}</span>
          <div data-testid="compare-b">
            <Select
              value={bId || null}
              options={options}
              ariaLabel={t("meetings.variants.diff.otherLabel")}
              onChange={(value) => value && setBId(value)}
            />
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          data-testid="compare-choose-a"
          disabled={busy !== null || !byId(aId) || byId(aId)?.active}
          onClick={() => void choose(aId)}
        >
          {t("meetings.variants.diff.chooseA")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          data-testid="compare-choose-b"
          disabled={busy !== null || !byId(bId) || byId(bId)?.active}
          onClick={() => void choose(bId)}
        >
          {t("meetings.variants.diff.chooseB")}
        </Button>
        <Button
          size="sm"
          data-testid="compare-merge"
          disabled={busy !== null || !aId || !bId || aId === bId}
          onClick={() => void merge()}
        >
          {busy === "merge"
            ? t("meetings.variants.diff.merging")
            : t("meetings.variants.diff.merge")}
        </Button>
      </div>
      <p className="text-xs text-text/60">
        {t("meetings.variants.diff.mergeHint")}
      </p>

      {error && (
        <div data-testid="compare-error">
          <Alert variant="error">{error}</Alert>
        </div>
      )}
      {notice && (
        <p
          className="text-sm text-text/80"
          role="status"
          data-testid="compare-notice"
        >
          {notice}
        </p>
      )}

      <p className="flex flex-wrap gap-x-4 text-xs text-text/70">
        <del className="text-red-600 dark:text-red-400">
          {t("meetings.variants.diff.legendDel")}
        </del>
        <ins className="text-green-700 dark:text-green-400">
          {t("meetings.variants.diff.legendIns")}
        </ins>
      </p>

      {rows === null ? (
        <p className="text-sm text-text/60">
          {t("meetings.variants.diff.loading")}
        </p>
      ) : changed === 0 ? (
        <p className="text-sm text-text/70" data-testid="compare-same">
          {t("meetings.variants.diff.same")}
        </p>
      ) : (
        <div
          role="region"
          aria-label={t("meetings.variants.diff.region")}
          className="space-y-1.5"
          data-testid="diff-rows"
        >
          {rows
            .filter((row) => row.changed)
            .map((row) => (
              <div
                key={row.startMs}
                data-testid="diff-row"
                className="flex gap-2 text-sm"
              >
                <span className="w-10 shrink-0 pt-0.5 text-xs tabular-nums text-text/60">
                  {formatMmSs(row.startMs)}
                </span>
                <p className="min-w-0 flex-1 break-words">
                  {row.ops.map((op, i) =>
                    op.kind === "same" ? (
                      <span key={i}>{op.text} </span>
                    ) : op.kind === "del" ? (
                      <del
                        key={i}
                        data-diff="del"
                        className="text-red-600 dark:text-red-400"
                      >
                        {op.text}{" "}
                      </del>
                    ) : (
                      <ins
                        key={i}
                        data-diff="ins"
                        className="text-green-700 dark:text-green-400"
                      >
                        {op.text}{" "}
                      </ins>
                    ),
                  )}
                </p>
              </div>
            ))}
        </div>
      )}
    </div>
  );
};
