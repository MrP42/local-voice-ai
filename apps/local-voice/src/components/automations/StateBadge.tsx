import React from "react";
import { stateTone, type Tone } from "./model";

const TONES: Record<Tone, string> = {
  green: "bg-green-500/20",
  amber: "bg-amber-500/25",
  red: "bg-red-500/20",
  blue: "bg-blue-500/20",
  gray: "bg-mid-gray/20",
};

/** Zustandsmarke (Lauf, Schritt, Recht): Farbe nur als Zugabe, der Zustand steht als Text da. */
export const StateBadge: React.FC<{
  state: string;
  label: string;
  tone?: Tone;
  testId?: string;
}> = ({ state, label, tone, testId = "state-badge" }) => (
  <span
    className={`inline-flex items-center whitespace-nowrap rounded-full px-2 py-0.5 text-xs font-medium text-text ${TONES[tone ?? stateTone(state)]}`}
    data-testid={testId}
    data-state={state}
  >
    {label}
  </span>
);
