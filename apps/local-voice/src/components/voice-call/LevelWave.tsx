import React, { useEffect, useRef } from "react";
import { subscribeCallLevel } from "@/stores/voiceCallStore";

const BARS = [0.5, 0.8, 1, 0.7, 0.9, 0.6, 0.4];

/** Wellenanzeige: folgt dem Mikrofonpegel, ohne den Zustand anzufassen. */
export const LevelWave: React.FC<{ live: boolean; busy: boolean }> = ({
  live,
  busy,
}) => {
  const refs = useRef<(HTMLElement | null)[]>([]);
  useEffect(() => {
    if (!live) {
      refs.current.forEach((el) => el && (el.style.height = ""));
      return;
    }
    return subscribeCallLevel((level) => {
      const v = Math.min(1, level * 8);
      refs.current.forEach((el, i) => {
        if (el) el.style.height = `${4 + v * 14 * BARS[i]}px`;
      });
    });
  }, [live]);
  return (
    <span className="inline-flex items-center gap-0.5 h-5" aria-hidden="true">
      {BARS.map((_, i) => (
        <i
          key={i}
          ref={(el) => {
            refs.current[i] = el;
          }}
          className={`w-[3px] rounded-sm h-1 transition-[height] duration-75 ${
            live ? "bg-logo-primary" : "bg-mid-gray/40"
          } ${busy ? "animate-pulse" : ""}`}
        />
      ))}
    </span>
  );
};
