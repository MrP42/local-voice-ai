import React, { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Mic } from "lucide-react";
import type { VoiceLine } from "@/stores/voiceCallStore";

const SUGGESTIONS = ["s1", "s2", "s3", "s4"] as const;

export const Transcript: React.FC<{
  lines: VoiceLine[];
  active: boolean;
  onSuggest: (text: string) => void;
}> = ({ lines, active, onSuggest }) => {
  const { t, i18n } = useTranslation();
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  if (lines.length === 0) {
    return (
      <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-3 text-center p-6 text-text/70">
        <span className="grid place-items-center w-14 h-14 rounded-full bg-logo-primary/15 text-logo-primary">
          <Mic className="w-6 h-6" aria-hidden="true" />
        </span>
        <h2 className="m-0 text-xl font-semibold text-text">
          {t("voiceCall.empty.title")}
        </h2>
        <p className="m-0">
          {active ? t("voiceCall.speakNow") : t("voiceCall.empty.hint")}
        </p>
        <div className="flex flex-wrap justify-center gap-2 max-w-[640px]">
          {SUGGESTIONS.map((k) => (
            <button
              key={k}
              type="button"
              onClick={() => onSuggest(t(`voiceCall.suggestions.${k}`))}
              className="rounded-2xl border border-mid-gray/30 bg-background px-3 py-1 text-sm cursor-pointer text-text hover:border-logo-primary"
            >
              {t(`voiceCall.suggestions.${k}`)}
            </button>
          ))}
        </div>
      </div>
    );
  }

  const time = (at: number) =>
    new Date(at).toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    });

  return (
    <div
      className="flex-1 min-h-0 overflow-y-auto px-4 pt-4 pb-2"
      role="log"
      aria-label={t("voiceCall.transcript")}
    >
      <div className="mx-auto max-w-[880px] flex flex-col gap-2.5">
        {lines.map((line) => {
          const user = line.role === "user";
          return (
            <div
              key={line.id}
              className={`flex gap-2 items-end max-w-[85%] ${user ? "self-end flex-row-reverse" : "self-start"}`}
            >
              {!user && (
                <span
                  className="grid place-items-center w-7 h-7 rounded-full shrink-0 text-[11px] font-semibold bg-logo-primary/15 text-logo-primary"
                  aria-hidden="true"
                >
                  {t("voiceCall.aiAvatar")}
                </span>
              )}
              <div className="min-w-0">
                <div
                  className={`px-3 py-2 rounded-xl text-sm whitespace-pre-wrap break-words ${
                    user
                      ? "bg-logo-primary/15 rounded-br-sm"
                      : "bg-mid-gray/15 rounded-bl-sm"
                  }`}
                >
                  {line.text}
                  {line.interrupted ? " …" : ""}
                </div>
                <div
                  className={`mt-0.5 flex gap-2 text-[11px] text-text/50 ${user ? "justify-end" : ""}`}
                >
                  <span>{time(line.at)}</span>
                  {line.latencyMs !== undefined && (
                    <span>
                      {t("voiceCall.seconds", {
                        value: (line.latencyMs / 1000).toLocaleString(
                          i18n.language,
                          { maximumFractionDigits: 1 },
                        ),
                      })}
                    </span>
                  )}
                </div>
              </div>
            </div>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
};
