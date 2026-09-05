import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Play } from "lucide-react";
import { commands, type Candidate } from "@/bindings";
import { Button } from "../../../ui/Button";

export const CandidateCard: React.FC<{
  draftId: string;
  candidate: Candidate;
  chosen: boolean;
  onChoose: (seed: number) => void;
  onError: (message: string) => void;
}> = ({ draftId, candidate, chosen, onChoose, onError }) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  // Kein JSX-Literal: die Raute ist Auszeichnung, kein uebersetzbarer Text.
  const seedLabel = `#${candidate.seed}`;

  // Die Bytes kommen bei JEDEM Abspielen frisch: der Tiefe-Regler kann
  // sich zwischendurch geaendert haben, und ein zwischengespeicherter
  // Blob wuerde dann die alte Fassung abspielen.
  const play = async () => {
    setBusy(true);
    try {
      const res = await commands.ttsBuilderCandidateWav(
        draftId,
        candidate.seed,
      );
      if (res.status !== "ok") {
        onError(res.error);
        return;
      }
      const blob = new Blob([new Uint8Array(res.data)], { type: "audio/wav" });
      const url = URL.createObjectURL(blob);
      const audio = new Audio(url);
      audio.onended = () => URL.revokeObjectURL(url);
      await audio.play();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className={`flex items-center gap-2 rounded-lg border p-2 ${
        chosen ? "border-logo-primary bg-logo-primary/10" : "border-mid-gray/40"
      }`}
    >
      {/* Reines Symbol ohne Beschriftung — dafuer gibt die Button-Komponente
          keine Groesse her, deshalb hier ein eigener Knopf mit denselben
          A11y-Regeln wie im Rest der Vorlesen-Oberflaeche. */}
      <button
        type="button"
        onClick={() => void play()}
        disabled={busy}
        title={t("tts.builder.play")}
        aria-label={t("tts.builder.play")}
        className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/70 transition-colors hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-default disabled:opacity-50"
      >
        <Play width={16} height={16} aria-hidden="true" />
      </button>
      <span className="min-w-0 flex-1 truncate text-xs text-text/45">
        {seedLabel}
      </span>
      <Button
        size="sm"
        variant={chosen ? "primary-soft" : "secondary"}
        onClick={() => onChoose(candidate.seed)}
        aria-pressed={chosen}
        className="min-h-[44px] px-3 text-sm"
      >
        {chosen && <Check width={14} height={14} aria-hidden="true" />}
        {chosen ? t("tts.builder.chosen") : t("tts.builder.choose")}
      </Button>
    </div>
  );
};
