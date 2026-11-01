import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Popover,
  PopoverSurface,
  PopoverTrigger,
} from "@fluentui/react-components";
import { Info } from "lucide-react";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import {
  formatUsd,
  median,
  useActiveLlm,
  useCallCost,
  useSttModelName,
  type CallCost,
} from "./useVoiceInfo";
import { useVoiceChoice } from "../settings/tts/useVoiceChoice";
import { useServerPhase } from "../settings/meetings/ReadAloudBar";
import { noKnowledge } from "./orchestration";
import { useKnowledge } from "./useKnowledge";

const DOT: Record<string, string> = {
  idle: "bg-mid-gray/60",
  starting: "bg-yellow-400 animate-pulse",
  listening: "bg-green-500",
  hearing: "bg-green-500",
  thinking: "bg-logo-primary animate-pulse",
  speaking: "bg-logo-primary",
};

function useCostTexts(cost: CallCost) {
  const { t, i18n } = useTranslation();
  switch (cost.kind) {
    case "subscription":
      return {
        chip: t("voiceCall.cost.subscription"),
        detail: t("voiceCall.cost.subscriptionDetail"),
      };
    case "local":
      return {
        chip: t("voiceCall.cost.local"),
        detail: t("voiceCall.cost.localDetail"),
      };
    case "usd": {
      const v = formatUsd(cost.usd, i18n.language);
      return { chip: `≈ ${v}`, detail: v };
    }
    case "unknown":
      return {
        chip: t("voiceCall.cost.unknown"),
        detail: t("voiceCall.cost.unknownDetail"),
      };
    default:
      return { chip: null, detail: t("voiceCall.cost.none") };
  }
}

/** Genau eine Zeile: Status, Modell, erster Ton, Token, Kosten; Details hinter dem i. */
export const InfoLine: React.FC = () => {
  const { t, i18n } = useTranslation();
  const state = useVoiceCallStore((s) => s.state);
  const ready = useVoiceCallStore((s) => s.ready);
  const metrics = useVoiceCallStore((s) => s.metrics);
  const usage = useVoiceCallStore((s) => s.usage);
  const openSettings = useVoiceCallStore((s) => s.openSettings);
  const knowledge = useKnowledge();
  const llm = useActiveLlm();
  const stt = useSttModelName();
  const cost = useCallCost(usage, llm);
  const costText = useCostTexts(cost);
  const phase = useServerPhase();
  const { options, voiceValue } = useVoiceChoice(phase);
  const voiceLabel = options.find((o) => o.value === voiceValue)?.label ?? "–";
  const last = metrics[metrics.length - 1];
  const med = useMemo(
    () => median(metrics.map((m) => m.firstSoundMs)),
    [metrics],
  );
  const num = (n: number) => n.toLocaleString(i18n.language);
  const sec = (ms: number) =>
    `${(ms / 1000).toLocaleString(i18n.language, { maximumFractionDigits: 1 })} s`;
  const total = usage.promptTokens + usage.completionTokens;
  const model = ready?.llm || llm.label;

  return (
    <div className="flex items-center gap-2.5 h-9 px-4 border-b border-mid-gray/20 text-xs text-text/70 whitespace-nowrap overflow-hidden">
      <Popover positioning="below-start" withArrow={false}>
        <PopoverTrigger disableButtonEnhancement>
          <button
            type="button"
            className="grid place-items-center w-7 h-7 rounded cursor-pointer hover:bg-mid-gray/15 text-text/70 hover:text-text shrink-0"
            aria-label={t("voiceCall.info.open")}
          >
            <Info className="w-4 h-4" aria-hidden="true" />
          </button>
        </PopoverTrigger>
        <PopoverSurface aria-label={t("voiceCall.info.title")}>
          <div className="w-[min(420px,calc(100vw-48px))] text-[13px] whitespace-normal">
            <h3 className="m-0 mb-1.5 text-sm font-semibold">
              {t("voiceCall.info.title")}
            </h3>
            <p className="m-0">{t("voiceCall.description")}</p>
            <ul className="my-1.5 ps-4">
              <li>{t("voiceCall.info.tipInterrupt")}</li>
              <li>{t("voiceCall.info.tipHeadset")}</li>
              <li>{t("voiceCall.info.tipText")}</li>
              <li>{t("voiceCall.info.tipMute")}</li>
            </ul>
            <table
              className="w-full text-xs border-collapse mt-2"
              aria-label={t("voiceCall.info.usageTable")}
            >
              <tbody>
                {[
                  [
                    t("voiceCall.info.tokens"),
                    `${num(usage.promptTokens)} / ${num(usage.completionTokens)}`,
                  ],
                  [t("voiceCall.info.cost"), costText.detail],
                  [
                    t("voiceCall.info.latency"),
                    last
                      ? t("voiceCall.latencyDetail", {
                          stt: last.sttMs,
                          llm: last.llmFirstMs,
                          tts: last.ttsFirstMs,
                        })
                      : "–",
                  ],
                  [
                    t("voiceCall.info.recognitionVoice"),
                    `${stt ?? "–"} · ${voiceLabel}`,
                  ],
                ].map(([k, v]) => (
                  <tr key={k} className="border-t border-mid-gray/20">
                    <td className="py-1 pe-3">{k}</td>
                    <td className="py-1 text-end text-text/70">{v}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </PopoverSurface>
      </Popover>

      <span className="inline-flex items-center gap-1.5 shrink-0">
        <span
          className={`w-2 h-2 rounded-full ${DOT[state]}`}
          aria-hidden="true"
        />
        <span role="status" aria-live="polite">
          {t(`voiceCall.state.${state}`)}
        </span>
      </span>
      {model && (
        <>
          <span className="w-px h-3.5 bg-mid-gray/30 shrink-0" />
          <span className="truncate min-w-16" title={t("voiceCall.info.model")}>
            {model}
            {ready?.llm_local && !usage.anyCloud
              ? ` · ${t("voiceCall.local")}`
              : ""}
          </span>
        </>
      )}
      {last && (
        <>
          <span className="w-px h-3.5 bg-mid-gray/30 shrink-0 max-sm:hidden" />
          <span
            className="shrink-0 max-sm:hidden"
            title={t("voiceCall.latencyDetail", {
              stt: last.sttMs,
              llm: last.llmFirstMs,
              tts: last.ttsFirstMs,
            })}
          >
            {t("voiceCall.firstSound", {
              last: sec(last.firstSoundMs),
              median: sec(med),
            })}
          </span>
        </>
      )}
      {total > 0 && (
        <>
          <span className="w-px h-3.5 bg-mid-gray/30 shrink-0" />
          <span
            className="shrink-0 tabular-nums"
            title={t("voiceCall.info.tokensHint", {
              input: num(usage.promptTokens),
              output: num(usage.completionTokens),
            })}
          >
            {t("voiceCall.tokens", { count: num(total) })}
          </span>
        </>
      )}
      {costText.chip && (
        <span
          className={`shrink-0 inline-flex items-center h-5 px-1.5 rounded-full border text-[11px] ${
            cost.kind === "usd"
              ? "bg-logo-primary/15 border-transparent text-logo-primary"
              : "bg-mid-gray/10 border-mid-gray/25"
          }`}
        >
          {costText.chip}
        </span>
      )}
      {noKnowledge(knowledge) && (
        <button
          type="button"
          data-testid="knowledge-chip"
          title={t("voiceCall.knowledge.chipTitle")}
          onClick={() => openSettings("knowledge")}
          className="shrink-0 inline-flex items-center h-5 px-1.5 rounded-full border border-mid-gray/25 bg-mid-gray/10 text-[11px] cursor-pointer text-logo-primary hover:border-logo-primary"
        >
          {t("voiceCall.knowledge.chip")}
        </button>
      )}
    </div>
  );
};
