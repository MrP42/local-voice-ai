import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";

/** Laenge der Anweisung, die das Backend annimmt (`MAX_INSTRUCTION_CHARS`). */
const MAX_INSTRUCTION_CHARS = 500;

interface InstructionBarProps {
  /** Wendet die Anweisung an; `true`, wenn sie ausgefuehrt wurde (Feld wird geleert). */
  onApply: (instruction: string) => Promise<boolean>;
  busy: boolean;
  disabled?: boolean;
}

/**
 * Eingabezeile "Anweisung an die KI" unter den KI-Notizen. Eine Anweisung
 * erzeugt eine neue Version; die eigenen Notizen des Nutzers bleiben, wie sie sind.
 */
export const InstructionBar: React.FC<InstructionBarProps> = ({
  onApply,
  busy,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const canApply = !busy && !disabled && text.trim() !== "";

  const apply = async () => {
    if (!canApply) return;
    if (await onApply(text.trim())) setText("");
  };

  return (
    <div className="space-y-1" data-testid="instruction-bar">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void apply();
            }
          }}
          maxLength={MAX_INSTRUCTION_CHARS}
          disabled={busy || disabled}
          placeholder={t("meetings.enhanced.instruction.placeholder")}
          aria-label={t("meetings.enhanced.instruction.label")}
          className="min-w-[14rem] flex-1 !font-normal"
        />
        <Button
          size="sm"
          variant="secondary"
          onClick={() => void apply()}
          disabled={!canApply}
        >
          <Sparkles width={14} height={14} />
          {t("meetings.enhanced.instruction.apply")}
        </Button>
      </div>
      <p className="text-xs text-text/50">
        {t("meetings.enhanced.instruction.hint")}
      </p>
    </div>
  );
};
