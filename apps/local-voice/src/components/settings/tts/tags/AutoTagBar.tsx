import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Sparkles } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { Select } from "@/components/ui/Select";
import { commands } from "@/bindings";
import { TAG_REGISTRY } from "@/lib/tags/registry";
import type { ChipEditorSuggestion } from "../editor/TtsChipEditor";

// ---------------------------------------------------------------------------
// Reine Hilfsfunktionen (leicht testbar, hier ohne React) — der eigentliche
// Grund, warum Auto-Tagging ein eigenes Modul ist statt Inline-Code in
// TtsSettings.tsx: Offset-Umrechnung und Splice-Logik sind die Stellen, an
// denen ein Fehler einen falsch platzierten Tag bedeutet.
// ---------------------------------------------------------------------------

/**
 * Rechnet einen char-Offset (Unicode-Skalarwerte, wie Rust `chars().count()`
 * zählt — das liefert das Backend als `offset_chars`) in einen UTF-16-Offset
 * um, wie ihn JS-Strings (und damit `textarea.selectionStart`/`slice`)
 * verwenden. `for…of` iteriert über Codepoints (behandelt Surrogatpaare als
 * EIN Schritt); `ch.length` ist für Zeichen jenseits der Basisebene (Emoji
 * u. Ä.) 2, sonst 1 — genau der Unterschied, den die Umrechnung ausgleicht.
 */
export function charOffsetToUtf16(text: string, charOffset: number): number {
  let utf16 = 0;
  let chars = 0;
  for (const ch of text) {
    if (chars >= charOffset) break;
    utf16 += ch.length;
    chars += 1;
  }
  return utf16;
}

/** Ein Insertion-Eintrag, wie ihn `commands.ttsAutoTag` liefert (nur die
 *  fürs Frontend relevanten Felder). */
export interface AutoTagInsertion {
  offset_chars: number;
  tag: string;
}

let suggestionCounter = 0;

/** Stabile, eindeutige Id je Vorschlag — `crypto.randomUUID` wo vorhanden,
 *  sonst ein Zähler+Zeitstempel-Fallback (ältere Webviews). */
function makeSuggestionId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  suggestionCounter += 1;
  return `autotag-${Date.now()}-${suggestionCounter}`;
}

/** Backend-Insertions (char-Offsets) → Editor-Suggestions (UTF-16-Offsets). */
export function insertionsToSuggestions(
  text: string,
  insertions: AutoTagInsertion[],
): ChipEditorSuggestion[] {
  return insertions.map((insertion) => ({
    id: makeSuggestionId(),
    offset: charOffsetToUtf16(text, insertion.offset_chars),
    tag: insertion.tag,
  }));
}

/**
 * Einen einzelnen Vorschlag annehmen oder verwerfen. Annehmen splict das Tag
 * an seinem Offset in den Text UND zieht alle VERBLEIBENDEN Vorschläge nach,
 * deren Offset auf oder hinter der Einfügestelle liegt (die eingefügte Länge
 * in UTF-16-Einheiten — schon die richtige Einheit, weil `offset` hier schon
 * UTF-16 ist). Verwerfen entfernt nur den Vorschlag, der Text bleibt unberührt.
 */
export function resolveSuggestion(
  text: string,
  suggestions: ChipEditorSuggestion[],
  id: string,
  accept: boolean,
): { text: string; suggestions: ChipEditorSuggestion[]; inserted: boolean } {
  const target = suggestions.find((s) => s.id === id);
  if (!target) {
    return { text, suggestions, inserted: false };
  }
  const remaining = suggestions.filter((s) => s.id !== id);
  if (!accept) {
    return { text, suggestions: remaining, inserted: false };
  }
  const insertText = `[${target.tag}]`;
  const nextText =
    text.slice(0, target.offset) + insertText + text.slice(target.offset);
  const shifted = remaining.map((s) =>
    s.offset >= target.offset
      ? { ...s, offset: s.offset + insertText.length }
      : s,
  );
  return { text: nextText, suggestions: shifted, inserted: true };
}

/**
 * "Alle annehmen"/"Alle verwerfen": bei Annahme wird in OFFSET-ABSTEIGENDER
 * Reihenfolge gesplict — jede Einfügestelle liegt dann noch unberührt vom
 * Text davor, also ist keine Nachziehe-Rechnung nötig (Brief-Vorgabe).
 */
export function resolveAllSuggestions(
  text: string,
  suggestions: ChipEditorSuggestion[],
  accept: boolean,
): { text: string; suggestions: ChipEditorSuggestion[]; count: number } {
  const count = suggestions.length;
  if (count === 0) {
    return { text, suggestions, count: 0 };
  }
  if (!accept) {
    return { text, suggestions: [], count };
  }
  const descending = [...suggestions].sort((a, b) => b.offset - a.offset);
  let nextText = text;
  for (const s of descending) {
    nextText =
      nextText.slice(0, s.offset) + `[${s.tag}]` + nextText.slice(s.offset);
  }
  return { text: nextText, suggestions: [], count };
}

// ---------------------------------------------------------------------------
// Die Leiste
// ---------------------------------------------------------------------------

/** UI-Sentinel fuer "Standard-KI" — leerer String gilt der Select-Komponente
 *  als "nichts gewaehlt" und zeigte den Platzhalter statt des Labels (gleiche
 *  Falle wie bei der Stimmenwahl weiter oben in TtsSettings.tsx, dort schon
 *  mit demselben Muster geloest). Settings-Wert bleibt "" — nur die UI-Seite
 *  bekommt den Sentinel. */
const DEFAULT_PROVIDER_UI_VALUE = "@default";

interface AutoTagBarProps {
  /** Der Text des AKTIVEN Reiters (nur der Original-Reiter montiert diese
   *  Leiste — siehe TtsSettings.tsx). */
  text: string;
  /** Offene Vorschläge, UTF-16-Offsets — dieselbe Liste, die der Editor als
   *  `suggestions`-Prop bekommt. */
  suggestions: ChipEditorSuggestion[];
  /** Text, gegen den `suggestions`-Offsets aktuell gültig sind — `null` ohne
   *  offene Liste. Weicht `text` davon ab, hat der Nutzer seit der letzten
   *  Anwendung getippt: Annehmen würde blind splicen (Review-Befund 2). */
  sourceText: string | null;
  onSuggestionsChange: (
    next: ChipEditorSuggestion[],
    sourceText: string | null,
  ) => void;
  /** Text wirklich ändern (Annehmen) — bekommt den vorherigen Text mit, damit
   *  der Aufrufer einen Undo-Toast anbieten kann. */
  onApplyText: (nextText: string, previousText: string, count: number) => void;
}

export const AutoTagBar: React.FC<AutoTagBarProps> = ({
  text,
  suggestions,
  sourceText,
  onSuggestionsChange,
  onApplyText,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Immer der ZULETZT gerenderte Text — anders als eine in runAutoTag()
  // eingefangene Variable bleibt ein Ref waehrend eines laufenden `await`
  // aktuell, wenn der Nutzer zwischenzeitlich weiterschreibt (Review-Befund 2).
  const textRef = useRef(text);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  const providerValue = getSetting("tts_tag_provider") ?? "";
  const activeProviderId = getSetting("post_process_provider_id") ?? "";
  const anthropicKey =
    (
      getSetting("post_process_api_keys") as Record<string, string> | undefined
    )?.["anthropic"] ?? "";
  // Wirksamer Provider fürs Auto-Tagging: die Select-Wahl, oder — bei "" —
  // was ohnehin fürs Post-Processing aktiv ist. Nur DANN kann der fehlende
  // Anthropic-Key vorab (ohne Netzwerk-Anfrage) erkannt werden.
  const effectiveProviderId = providerValue || activeProviderId;
  const missingAnthropicKey =
    effectiveProviderId === "anthropic" && anthropicKey.trim() === "";

  const providerOptions = [
    {
      value: DEFAULT_PROVIDER_UI_VALUE,
      label: t("tts.autotag.providerDefault"),
    },
    { value: "anthropic", label: t("tts.autotag.providerClaude") },
  ];

  const runAutoTag = async () => {
    if (!text.trim() || loading) return;
    if (missingAnthropicKey) {
      setError(t("tts.autotag.missingApiKey"));
      return;
    }
    setError(null);
    setLoading(true);
    const textAtStart = text;
    const allowedTags = TAG_REGISTRY.map((tag) => tag.insert);
    const result = await commands.ttsAutoTag(
      textAtStart,
      allowedTags,
      providerValue || null,
    );
    setLoading(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    // Race (Review-Befund 2): waehrend der Anfrage lief, koennte der Text
    // editiert worden sein — die Offsets waeren dann gegen den FALSCHEN Text
    // berechnet. textRef.current ist der zum Zeitpunkt der Antwort tatsaechlich
    // aktuelle Text; weicht er vom Anfrage-Start ab, werden die Vorschlaege
    // verworfen statt (moeglicherweise falsch positioniert) angezeigt.
    if (textRef.current !== textAtStart) {
      setError(t("tts.autotag.textChangedDuringRun"));
      return;
    }
    onSuggestionsChange(
      insertionsToSuggestions(textAtStart, result.data),
      textAtStart,
    );
  };

  const acceptAll = () => {
    if (suggestions.length === 0) return;
    if (sourceText !== null && text !== sourceText) {
      // Seit der letzten Anwendung wurde getippt — die Offsets sind nicht
      // mehr vertrauenswuerdig: verwerfen statt blind zu splicen.
      onSuggestionsChange([], null);
      toast.info(t("tts.autotag.staleSuggestionsDiscarded"));
      return;
    }
    const outcome = resolveAllSuggestions(text, suggestions, true);
    onSuggestionsChange(outcome.suggestions, null);
    if (outcome.text !== text) {
      onApplyText(outcome.text, text, outcome.count);
    }
  };

  const rejectAll = () => {
    onSuggestionsChange(
      resolveAllSuggestions(text, suggestions, false).suggestions,
      null,
    );
  };

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-mid-gray/20 px-2 py-1.5">
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void runAutoTag()}
        disabled={loading || !text.trim()}
        title={t("tts.autotag.button")}
        aria-label={t("tts.autotag.button")}
      >
        <Sparkles
          width={14}
          height={14}
          className={loading ? "animate-spin" : ""}
        />
        {t("tts.autotag.button")}
      </Button>
      <div className="w-40">
        <Select
          value={
            providerValue === "" ? DEFAULT_PROVIDER_UI_VALUE : providerValue
          }
          options={providerOptions}
          isClearable={false}
          onChange={(value) => {
            void updateSetting(
              "tts_tag_provider",
              value === DEFAULT_PROVIDER_UI_VALUE ? "" : (value ?? ""),
            );
          }}
        />
      </div>
      {suggestions.length > 0 && (
        <>
          <span className="text-xs text-text/60">
            {t("tts.autotag.counter", { count: suggestions.length })}
          </span>
          <Button variant="secondary" size="sm" onClick={acceptAll}>
            {t("tts.autotag.acceptAll")}
          </Button>
          <Button variant="secondary" size="sm" onClick={rejectAll}>
            {t("tts.autotag.rejectAll")}
          </Button>
        </>
      )}
      {error && (
        <p className="w-full text-xs text-red-500 break-words">{error}</p>
      )}
    </div>
  );
};
