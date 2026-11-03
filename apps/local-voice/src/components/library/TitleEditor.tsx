import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, Sparkles, X } from "lucide-react";
import { cleanTitle, hasTitleNoise } from "@/lib/library/names";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

/**
 * Titel an Ort und Stelle bearbeiten. Trägt der Titel Dateinamen-Reste
 * (ASIN, „LC 128 44100 Stereo“ …), steht der bereinigte Vorschlag aus
 * {@link cleanTitle} daneben und lässt sich mit einem Klick übernehmen.
 * Enter speichert, Esc bricht ab.
 */
export const TitleEditor: React.FC<{
  title: string;
  onSave(title: string): Promise<void>;
  onCancel(): void;
}> = ({ title, onSave, onCancel }) => {
  const { t } = useTranslation();
  const [value, setValue] = useState(title);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  const trimmed = value.replace(/\s+/g, " ").trim();
  const suggestion = hasTitleNoise(value) ? cleanTitle(value) : null;
  const canSave = !!trimmed && trimmed !== title && !busy;

  const save = async () => {
    if (!canSave) return;
    setBusy(true);
    try {
      await onSave(trimmed);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="library-title-editor"
      data-testid="library-title-editor"
      onKeyDown={(e) => {
        if (e.key === "Escape" && !busy) {
          e.preventDefault();
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="library-title-editor__row">
        <Input
          ref={input}
          value={value}
          aria-label={t("library.field.title")}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            // Während einer IME-Eingabe gehört Enter der Komposition.
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void save();
            }
          }}
          data-testid="library-title-input"
        />
        <Button
          variant="primary"
          icon={<Check size={16} aria-hidden="true" />}
          onClick={() => void save()}
          disabled={!canSave}
          data-testid="library-title-save"
        >
          {t("library.save")}
        </Button>
        <Button
          variant="ghost"
          icon={<X size={16} aria-hidden="true" />}
          onClick={onCancel}
          disabled={busy}
        >
          {t("library.cancel")}
        </Button>
      </div>
      {suggestion && (
        <button
          type="button"
          className="library-title-editor__hint"
          onClick={() => {
            setValue(suggestion);
            input.current?.focus();
          }}
          data-testid="library-title-suggestion"
        >
          <Sparkles size={14} aria-hidden="true" />
          <span>
            {t("library.titleEdit.suggestion")} <strong>{suggestion}</strong>
          </span>
        </button>
      )}
    </div>
  );
};
