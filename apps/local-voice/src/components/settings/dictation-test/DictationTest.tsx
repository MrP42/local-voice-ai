import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { Button } from "../../ui/Button";
import { compareTranscript, type WordDiff } from "./compare";

const PHRASES = [
  "Guten Tag, dies ist ein Test der lokalen Spracherkennung.",
  "Der Termin ist am dritten Februar um vierzehn Uhr dreißig.",
  "Der ältere Herr aus der Straße hatte großen Ärger in Köln.",
  "Die Rechnung lautet 1.234,50 Euro bei 19 Prozent Mehrwertsteuer.",
];

/**
 * Dictation test bench.
 *
 * Dictate into this page rather than into a foreign editor, and compare what
 * arrived against what was supposed to arrive. The target field is a plain
 * textarea on purpose: focus it and the ordinary injection path delivers text
 * here exactly as it would anywhere else, so this exercises the real pipeline
 * instead of a simulation of it.
 */
export const DictationTest: React.FC = () => {
  const { t } = useTranslation();
  const [reference, setReference] = useState(PHRASES[0]);
  const [heard, setHeard] = useState("");
  const [speaking, setSpeaking] = useState(false);
  const targetRef = useRef<HTMLTextAreaElement>(null);

  const result = useMemo(
    () => compareTranscript(reference, heard),
    [reference, heard],
  );

  /**
   * Have the computer read the phrase aloud through the speakers.
   *
   * Focuses the target field first and keeps it focused: the transcript is
   * inserted into whatever window and field has focus, so reading aloud
   * without doing this delivered the text nowhere.
   */
  const readAloud = () => {
    const synth = window.speechSynthesis;
    if (!synth) return;
    targetRef.current?.focus();
    synth.cancel();
    const utterance = new SpeechSynthesisUtterance(reference);
    utterance.lang = "de-DE";
    const german = synth.getVoices().find((v) => v.lang.startsWith("de"));
    if (german) utterance.voice = german;
    utterance.onstart = () => {
      setSpeaking(true);
      targetRef.current?.focus();
    };
    utterance.onend = () => {
      setSpeaking(false);
      targetRef.current?.focus();
    };
    utterance.onerror = () => setSpeaking(false);
    synth.speak(utterance);
  };

  return (
    <div className="w-full space-y-6">
      <SettingsGroup title={t("dictationTest.title")}>
        <p className="text-sm opacity-70 px-1 pb-2">
          {t("dictationTest.intro")}
        </p>
      </SettingsGroup>

      <SettingsGroup title={t("dictationTest.step1")}>
        <div className="px-1 pb-3 space-y-2">
          <textarea
            className="w-full rounded-md border border-mid-gray/30 bg-transparent p-2 text-sm"
            rows={2}
            value={reference}
            onChange={(e) => setReference(e.target.value)}
          />
          <div className="flex flex-wrap gap-2">
            {PHRASES.map((p, i) => (
              <Button size="sm" key={i} onClick={() => setReference(p)}>
                {t("dictationTest.phrase")} {i + 1}
              </Button>
            ))}
            <Button size="sm" onClick={readAloud}>
              {speaking
                ? t("dictationTest.reading")
                : t("dictationTest.readAloud")}
            </Button>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("dictationTest.step2")}>
        <div className="px-1 pb-3 space-y-2">
          <textarea
            ref={targetRef}
            className="w-full rounded-md border-2 border-mid-gray/30 focus:border-[var(--color-logo-primary)] bg-transparent p-2 text-sm outline-none"
            rows={3}
            placeholder={t("dictationTest.targetPlaceholder")}
            value={heard}
            onChange={(e) => setHeard(e.target.value)}
          />
          <div className="flex gap-2">
            <Button size="sm" onClick={() => targetRef.current?.focus()}>
              {t("dictationTest.focusField")}
            </Button>
            <Button size="sm" onClick={() => setHeard("")}>
              {t("dictationTest.clear")}
            </Button>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup title={t("dictationTest.result")}>
        <div className="px-1 pb-3 space-y-3">
          {heard.trim().length === 0 ? (
            <p className="text-sm opacity-60">{t("dictationTest.noResult")}</p>
          ) : (
            <>
              <div className="flex items-baseline gap-4">
                <span
                  className="text-2xl font-bold"
                  style={{
                    color:
                      result.accuracy >= 0.95
                        ? "var(--color-logo-primary)"
                        : undefined,
                  }}
                >
                  {Math.round(result.accuracy * 100)}%
                </span>
                <span className="text-xs opacity-70">
                  {t("dictationTest.wordStats", {
                    correct: result.correct,
                    total: result.total,
                    wrong: result.substitutions,
                    missing: result.deletions,
                    extra: result.insertions,
                  })}
                </span>
              </div>
              <DiffView diff={result.diff} />
            </>
          )}
        </div>
      </SettingsGroup>
    </div>
  );
};

const DiffView: React.FC<{ diff: WordDiff[] }> = ({ diff }) => (
  <p className="text-sm leading-7">
    {diff.map((part, i) => {
      if (part.kind === "same") {
        return <span key={i}>{part.text} </span>;
      }
      if (part.kind === "missing") {
        return (
          <span key={i} className="line-through opacity-50" title="fehlt">
            {part.text}{" "}
          </span>
        );
      }
      return (
        <span
          key={i}
          className="rounded px-1"
          style={{
            background:
              "color-mix(in srgb, var(--color-logo-primary) 30%, transparent)",
          }}
          title={part.kind === "extra" ? "zu viel" : "abweichend"}
        >
          {part.text}{" "}
        </span>
      );
    })}
  </p>
);

export default DictationTest;
