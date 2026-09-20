import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, CheckCircle2 } from "lucide-react";
import { Select, type SelectOption } from "@/components/ui/Select";
import { localizedLabel } from "@/lib/tags/registry";
import type { TagDef } from "@/lib/tags/types";
import {
  knownTagsFor,
  type ScriptEngine,
  type ScriptFinding,
} from "@/lib/voices/scriptCheck";
import {
  speakerMarkerText,
  type SpeakerRef,
} from "@/lib/voices/speakerMarkers";

/**
 * Was die Befundliste am Text tun darf. `everywhere` heisst: alle Stellen
 * mit demselben Befund (gleiche Art, gleicher Name) im aktiven Reiter --
 * Suchen-und-Ersetzen, nicht nur diese eine Stelle.
 */
export interface ScriptCheckActions {
  reveal(finding: ScriptFinding): void;
  /** `replacement` liefert je Stelle den fertigen Text; `` entfernt. */
  replace(
    finding: ScriptFinding,
    replacement: (f: ScriptFinding) => string,
    everywhere: boolean,
  ): void;
}

interface ScriptCheckPanelProps {
  findings: ScriptFinding[];
  speakers: SpeakerRef[];
  engine: ScriptEngine;
  uiLang: string;
  actions: ScriptCheckActions;
}

const ACTION_CLASSES =
  "h-7 cursor-pointer rounded-md border border-mid-gray/30 bg-mid-gray/10 px-2 text-xs text-text/80 hover:border-logo-primary hover:text-text disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";

/** Eine Zeile je Befund: Beschreibung, Ersetzen-Auswahl, vier Massnahmen. */
const FindingRow: React.FC<{
  finding: ScriptFinding;
  speakers: SpeakerRef[];
  tags: TagDef[];
  uiLang: string;
  actions: ScriptCheckActions;
}> = ({ finding, speakers, tags, uiLang, actions }) => {
  const { t } = useTranslation();
  const [choice, setChoice] = useState("");
  const isSpeaker = finding.kind === "unknown-speaker";

  const replacementFor = (): ((f: ScriptFinding) => string) | null => {
    if (choice === "") return null;
    if (isSpeaker) {
      const speaker = speakers.find((s) => s.id === choice);
      if (!speaker) return null;
      return (f) => speakerMarkerText(speaker, f.style);
    }
    const tag = tags.find((d) => d.id === choice);
    if (!tag) return null;
    return () => `[${tag.insert}]`;
  };

  const replace = (everywhere: boolean) => {
    const replacement = replacementFor();
    if (replacement) actions.replace(finding, replacement, everywhere);
  };

  const message = isSpeaker
    ? t("tts.scriptCheck.unknownSpeaker", { name: finding.name })
    : t("tts.scriptCheck.unknownTag", { tag: finding.name });

  const options: SelectOption[] = isSpeaker
    ? speakers.map((speaker) => ({
        value: speaker.id,
        label: speaker.displayName,
      }))
    : tags.map((tag) => ({
        value: tag.id,
        label: `${localizedLabel(tag, uiLang)} [${tag.insert}]`,
      }));

  return (
    <li
      data-testid="script-finding"
      className="flex flex-wrap items-center gap-x-2 gap-y-1 py-1"
    >
      <button
        type="button"
        onClick={() => actions.reveal(finding)}
        title={t("tts.scriptCheck.reveal")}
        className="min-w-0 flex-1 cursor-pointer truncate text-start text-xs text-text hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
      >
        <span className="text-text/50">
          {t("tts.scriptCheck.line", { line: finding.line })}
        </span>{" "}
        {message}
      </button>
      <span className="flex flex-wrap items-center gap-1">
        <div
          className="w-44"
          data-testid="script-finding-replacement"
          title={t("tts.scriptCheck.replaceWith")}
        >
          <Select
            value={choice === "" ? null : choice}
            options={options}
            placeholder={t("tts.scriptCheck.replaceWith")}
            isClearable={false}
            onChange={(value) => setChoice(value ?? "")}
          />
        </div>
        <button
          type="button"
          disabled={choice === ""}
          onClick={() => replace(false)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.replaceHere")}
        </button>
        <button
          type="button"
          disabled={choice === ""}
          onClick={() => replace(true)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.replaceEverywhere")}
        </button>
        <button
          type="button"
          onClick={() => actions.replace(finding, () => "", false)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.removeHere")}
        </button>
        <button
          type="button"
          onClick={() => actions.replace(finding, () => "", true)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.removeEverywhere")}
        </button>
      </span>
    </li>
  );
};

/**
 * Kompakte Befundliste ueber dem Editor: Anzahl in der Kopfzeile, je Befund
 * eine Zeile. Ohne Befund eine einzige gruene Zeile -- damit man sieht,
 * dass geprueft wurde, nicht nur, dass nichts da ist.
 */
export const ScriptCheckPanel: React.FC<ScriptCheckPanelProps> = ({
  findings,
  speakers,
  engine,
  uiLang,
  actions,
}) => {
  const { t } = useTranslation();
  const tags = knownTagsFor(engine);
  if (findings.length === 0) {
    return (
      <p
        data-testid="script-check-clean"
        className="flex items-center gap-1.5 text-xs text-text/50"
      >
        <CheckCircle2 width={14} height={14} aria-hidden="true" />
        {t("tts.scriptCheck.clean")}
      </p>
    );
  }
  return (
    <section
      data-testid="script-check"
      aria-label={t("tts.scriptCheck.title")}
      className="rounded-md border border-red-500/30 bg-red-500/5 px-3 py-1.5"
    >
      <p className="flex items-center gap-1.5 text-xs font-medium text-text">
        <AlertTriangle
          width={14}
          height={14}
          aria-hidden="true"
          className="text-red-500"
        />
        {t("tts.scriptCheck.count", { count: findings.length })}
      </p>
      <ul className="divide-y divide-red-500/15">
        {findings.map((finding) => (
          <FindingRow
            key={`${finding.kind}:${finding.start}:${finding.end}`}
            finding={finding}
            speakers={speakers}
            tags={tags}
            uiLang={uiLang}
            actions={actions}
          />
        ))}
      </ul>
    </section>
  );
};
