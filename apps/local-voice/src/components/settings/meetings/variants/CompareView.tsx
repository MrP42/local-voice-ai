import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import {
  commands,
  type StoredSegment,
  type TranscriptVariant,
  type TranslationReport,
} from "@/bindings";
import { Button } from "../../../ui/Button";
import { Alert } from "../../../ui/Alert";
import { Select } from "../../../ui/Select";
import { baseLanguage, languageName } from "../language/languages";
import { alignDiff } from "./diff";
import { translateVariantError, variantListLabel } from "./useVariants";

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
  const { t, i18n } = useTranslation();
  const [aId, setAId] = useState("");
  const [bId, setBId] = useState("");
  const [a, setA] = useState<StoredSegment[] | null>(null);
  const [b, setB] = useState<StoredSegment[] | null>(null);
  const [busy, setBusy] = useState<"merge" | "choose" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [report, setReport] = useState<TranslationReport | null>(null);
  const [onlyFlagged, setOnlyFlagged] = useState(false);
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

  const byId = (id: string) => variants.find((v) => v.id === id);
  const optionLabel = (v: TranscriptVariant) => variantListLabel(v, t);

  // G5: Original und seine Übersetzung stehen Satz für Satz nebeneinander (gleiche
  // Zeitmarken), nicht als Wort-Diff: ein Wort-Diff zwischen zwei Sprachen markierte alles.
  const va = byId(aId);
  const vb = byId(bId);
  const pair =
    va && vb
      ? vb.kind === "translation" && vb.source_variant_id === va.id
        ? { original: va, translation: vb }
        : va.kind === "translation" && va.source_variant_id === vb.id
          ? { original: vb, translation: va }
          : null
      : null;
  const translationId = pair?.translation.id ?? null;

  // Der Prüfbericht der Übersetzung (markierte Sätze mit Grund).
  useEffect(() => {
    setReport(null);
    if (!translationId) return;
    let current = true;
    void commands.transcriptVariantReport(translationId).then((result) => {
      if (current && result.status === "ok") setReport(result.data ?? null);
    });
    return () => {
      current = false;
    };
  }, [translationId]);

  const isPair = pair !== null;
  const rows = useMemo(
    () => (a && b && !isPair ? alignDiff(a, b) : null),
    [a, b, isPair],
  );
  const changed = rows?.filter((r) => r.changed).length ?? 0;

  const originalId = pair?.original.id ?? null;
  const sentenceRows = useMemo(() => {
    if (!originalId || !translationId || !a || !b) return null;
    const originalSegments = originalId === aId ? a : b;
    const translatedSegments = translationId === aId ? a : b;
    const byIndex = new Map(
      translatedSegments.map((s) => [s.segment_index, s] as const),
    );
    const flags = new Map(
      (report?.flagged ?? []).map((f) => [f.segment_index, f.reasons] as const),
    );
    return [...originalSegments]
      .sort(
        (x, y) => x.start_ms - y.start_ms || x.segment_index - y.segment_index,
      )
      .map((segment) => ({
        index: segment.segment_index,
        startMs: segment.start_ms,
        original: segment.text,
        translated: byIndex.get(segment.segment_index)?.text ?? "",
        reasons: flags.get(segment.segment_index) ?? [],
      }));
  }, [originalId, translationId, a, b, aId, report]);
  const flaggedCount =
    sentenceRows?.filter((r) => r.reasons.length > 0).length ?? 0;

  const options = variants.map((v) => ({ value: v.id, label: optionLabel(v) }));
  const locale = i18n.language;
  const sourceName = (v: TranscriptVariant) => {
    const code = baseLanguage(v.language);
    return code ? languageName(code, locale) : "";
  };

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
          disabled={
            busy !== null || !aId || !bId || aId === bId || pair !== null
          }
          title={pair ? t("meetings.translate.noMerge") : undefined}
          onClick={() => void merge()}
        >
          {busy === "merge"
            ? t("meetings.variants.diff.merging")
            : t("meetings.variants.diff.merge")}
        </Button>
      </div>
      {!pair && (
        <p className="text-xs text-text/60">
          {t("meetings.variants.diff.mergeHint")}
        </p>
      )}

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

      {!pair && (
        <p className="flex flex-wrap gap-x-4 text-xs text-text/70">
          <del className="text-red-600 dark:text-red-400">
            {t("meetings.variants.diff.legendDel")}
          </del>
          <ins className="text-green-700 dark:text-green-400">
            {t("meetings.variants.diff.legendIns")}
          </ins>
        </p>
      )}

      {pair ? (
        <div className="space-y-2" data-testid="sentence-compare">
          <p
            className="text-xs text-text/70"
            role="status"
            data-testid="sentence-summary"
          >
            {flaggedCount === 0
              ? t("meetings.translate.allChecked")
              : t("meetings.translate.flaggedSummary", { count: flaggedCount })}
          </p>
          {flaggedCount > 0 && (
            <label className="flex cursor-pointer items-center gap-2 text-xs text-text/80">
              <input
                type="checkbox"
                data-testid="sentence-only-flagged"
                checked={onlyFlagged}
                onChange={(e) => setOnlyFlagged(e.target.checked)}
              />
              {t("meetings.translate.onlyFlagged")}
            </label>
          )}
          {sentenceRows === null ? (
            <p className="text-sm text-text/60">
              {t("meetings.variants.diff.loading")}
            </p>
          ) : (
            <div
              role="region"
              aria-label={t("meetings.translate.region")}
              className="@container space-y-1.5"
              data-testid="sentence-rows"
            >
              <div className="hidden gap-2 text-xs font-medium text-text/60 @[30rem]:grid @[30rem]:grid-cols-[2.5rem_1fr_1fr]">
                <span />
                <span data-testid="sentence-head-original">
                  {t("meetings.translate.original", {
                    language: sourceName(pair.original),
                  })}
                </span>
                <span data-testid="sentence-head-translation">
                  {t("meetings.translate.translation", {
                    language: sourceName(pair.translation),
                  })}
                </span>
              </div>
              {sentenceRows
                .filter((row) => !onlyFlagged || row.reasons.length > 0)
                .map((row) => (
                  <div
                    key={row.index}
                    data-testid="sentence-row"
                    data-flagged={row.reasons.length > 0 ? "true" : undefined}
                    className={`grid grid-cols-1 gap-x-2 gap-y-0.5 rounded-md px-1 text-sm @[30rem]:grid-cols-[2.5rem_1fr_1fr] ${
                      row.reasons.length > 0
                        ? "border-s-2 border-yellow-500 bg-yellow-500/10"
                        : ""
                    }`}
                  >
                    <span
                      className="pt-0.5 text-xs tabular-nums text-text/60"
                      data-testid="sentence-time"
                    >
                      {formatMmSs(row.startMs)}
                    </span>
                    <p
                      className="min-w-0 break-words"
                      data-testid="sentence-original"
                    >
                      {row.original}
                    </p>
                    <div className="min-w-0">
                      <p
                        className="break-words"
                        data-testid="sentence-translation"
                      >
                        {row.translated}
                      </p>
                      {row.reasons.length > 0 && (
                        <p
                          className="flex items-center gap-1 text-xs text-yellow-700 dark:text-yellow-400"
                          data-testid="sentence-reasons"
                        >
                          <AlertTriangle
                            width={12}
                            height={12}
                            aria-hidden="true"
                          />
                          {row.reasons
                            .map((reason) =>
                              t(`meetings.translate.reasons.${reason}`, {
                                defaultValue: reason,
                              }),
                            )
                            .join(", ")}
                        </p>
                      )}
                    </div>
                  </div>
                ))}
            </div>
          )}
        </div>
      ) : rows === null ? (
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
