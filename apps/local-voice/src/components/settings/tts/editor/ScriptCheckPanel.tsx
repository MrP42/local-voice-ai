import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  X,
} from "lucide-react";
import { Select, type SelectOption } from "@/components/ui/Select";
import { TAG_REGISTRY, localizedLabel } from "@/lib/tags/registry";
import { tagTextFor, useTagLanguage } from "../tags/tagLanguage";
import type { TagDef } from "@/lib/tags/types";
import {
  groupFindings,
  isHintFinding,
  knownTagsFor,
  suggestSpeakers,
  suggestTags,
  type FindingGroup,
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
  onClose?: () => void;
}

const ACTION_CLASSES =
  "h-7 cursor-pointer rounded-md border border-mid-gray/30 bg-mid-gray/10 px-2 text-xs text-text/80 hover:border-logo-primary hover:text-text disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";
const PRIMARY_CLASSES =
  "h-7 cursor-pointer rounded-md border border-logo-primary/60 bg-logo-primary/15 px-2 text-xs font-medium text-text hover:bg-logo-primary/30 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";
const NAV_CLASSES =
  "flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/20 hover:text-text disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";

/**
 * Eine Gruppe (ein Problem, n Stellen): Beschreibung, Stellen zum
 * Durchklicken, Empfehlungen, Ersetzen/Entfernen fuer alle oder nur diese.
 */
const GroupCard: React.FC<{
  group: FindingGroup;
  speakers: SpeakerRef[];
  tags: TagDef[];
  engine: ScriptEngine;
  uiLang: string;
  actions: ScriptCheckActions;
}> = ({ group, speakers, tags, engine, uiLang, actions }) => {
  const { t } = useTranslation();
  const [choice, setChoice] = useState("");
  const [pos, setPos] = useState(0);
  const isSpeaker =
    group.kind === "unknown-speaker" || group.kind === "speaker-name";
  const count = group.findings.length;
  const current = group.findings[Math.min(pos, count - 1)];

  // Neue Gruppe (anderer Name) oder weniger Stellen: Position zuruecksetzen.
  useEffect(() => {
    setPos(0);
    setChoice("");
  }, [group.key]);
  useEffect(() => {
    if (pos > count - 1) setPos(Math.max(0, count - 1));
  }, [count, pos]);

  const goTo = (index: number) => {
    const clamped = Math.max(0, Math.min(count - 1, index));
    setPos(clamped);
    actions.reveal(group.findings[clamped]);
  };

  const speakerReplacement = (speaker: SpeakerRef) => (f: ScriptFinding) =>
    speakerMarkerText(speaker, f.style);
  const tagLang = useTagLanguage();
  const tagReplacement = (tag: TagDef) => () => tagTextFor(tag, tagLang);

  const chosenReplacement = (): ((f: ScriptFinding) => string) | null => {
    if (choice === "") return null;
    if (isSpeaker) {
      const speaker = speakers.find((s) => s.id === choice);
      return speaker ? speakerReplacement(speaker) : null;
    }
    const tag = TAG_REGISTRY.find((d) => d.id === choice);
    return tag ? tagReplacement(tag) : null;
  };

  const message =
    group.kind === "free-tag"
      ? t("tts.scriptCheck.freeTag", { tag: group.name })
      : group.kind === "speaker-name"
        ? t("tts.scriptCheck.speakerName", { name: group.name })
        : isSpeaker
          ? t("tts.scriptCheck.unknownSpeaker", { name: group.name })
          : t("tts.scriptCheck.unknownTag", { tag: group.name });

  // "[Seufzend]" allein, wenn Klammertext und Beschriftung gleich sind
  // (deutsche Tag-Sprache); sonst "Seufzend [sighing]".
  const tagOptionLabel = (tag: TagDef) => {
    const text = tagTextFor(tag, tagLang);
    const label = localizedLabel(tag, uiLang);
    return text === `[${label}]` ? text : `${label} ${text}`;
  };
  const suggestedTagDefs = isSpeaker
    ? []
    : suggestTags(group.name, engine, uiLang);
  const recommended: {
    key: string;
    label: string;
    apply: (f: ScriptFinding) => string;
  }[] = isSpeaker
    ? suggestSpeakers(group.name, speakers).map((speaker) => ({
        key: speaker.id,
        label: speaker.displayName,
        apply: speakerReplacement(speaker),
      }))
    : suggestedTagDefs.slice(0, 3).map((tag) => ({
        key: tag.id,
        label: tagOptionLabel(tag),
        apply: tagReplacement(tag),
      }));

  const options: SelectOption[] = isSpeaker
    ? speakers.map((speaker) => ({
        value: speaker.id,
        label: speaker.displayName,
      }))
    : [
        // Empfehlungen oben, dann ALLE Tags (durchsuchbar), alphabetisch.
        ...suggestedTagDefs,
        ...TAG_REGISTRY.filter((tag) => !suggestedTagDefs.includes(tag)).sort(
          (a, b) =>
            localizedLabel(a, uiLang).localeCompare(localizedLabel(b, uiLang)),
        ),
      ].map((tag) => ({
        value: tag.id,
        label: tagOptionLabel(tag),
      }));

  return (
    <div data-testid="script-finding" className="space-y-2 py-2">
      <p className="text-sm text-text">
        <span className="font-medium">
          {isSpeaker
            ? t("tts.scriptCheck.groupSpeaker", { name: group.name, count })
            : t("tts.scriptCheck.groupTag", { tag: group.name, count })}
        </span>
        <span className="text-text/70"> — {message}</span>
      </p>

      {/* Stellen: durchklickbar, jede springt in den Text. */}
      <div className="flex flex-wrap items-center gap-1">
        <button
          type="button"
          onClick={() => goTo(pos - 1)}
          disabled={pos <= 0}
          className={NAV_CLASSES}
          aria-label={t("tts.scriptCheck.prevSpot")}
        >
          <ChevronLeft width={14} height={14} />
        </button>
        <span className="text-xs text-text/60">
          {t("tts.scriptCheck.spot", { index: pos + 1, count })}
        </span>
        <button
          type="button"
          onClick={() => goTo(pos + 1)}
          disabled={pos >= count - 1}
          className={NAV_CLASSES}
          aria-label={t("tts.scriptCheck.nextSpot")}
        >
          <ChevronRight width={14} height={14} />
        </button>
        <span className="mx-1 text-text/30">·</span>
        {group.findings.map((finding, index) => (
          <button
            key={`${finding.start}:${finding.end}`}
            type="button"
            onClick={() => goTo(index)}
            title={t("tts.scriptCheck.reveal")}
            className={`cursor-pointer rounded px-1.5 py-0.5 text-xs hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
              index === pos
                ? "bg-red-500/20 text-text"
                : "text-text/60 hover:text-text"
            }`}
          >
            {t("tts.scriptCheck.lineShort", { line: finding.line })}
          </button>
        ))}
      </div>

      {/* Empfehlungen: ein Klick, alle Stellen. */}
      {recommended.length > 0 && (
        <div className="flex flex-wrap items-center gap-1">
          <span className="text-xs text-text/60">
            {t("tts.scriptCheck.recommended")}
          </span>
          {recommended.map((item) => (
            <button
              key={item.key}
              type="button"
              data-testid="script-finding-recommendation"
              onClick={() => actions.replace(current, item.apply, true)}
              className={PRIMARY_CLASSES}
              title={t("tts.scriptCheck.applyAll", { count })}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-1">
        <div
          className="w-48"
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
          onClick={() => {
            const r = chosenReplacement();
            if (r) actions.replace(current, r, true);
          }}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.replaceAll", { count })}
        </button>
        <button
          type="button"
          disabled={choice === ""}
          onClick={() => {
            const r = chosenReplacement();
            if (r) actions.replace(current, r, false);
          }}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.replaceHere")}
        </button>
        <span className="mx-1 text-text/30">·</span>
        <button
          type="button"
          onClick={() => actions.replace(current, () => "", true)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.removeAll", { count })}
        </button>
        <button
          type="button"
          onClick={() => actions.replace(current, () => "", false)}
          className={ACTION_CLASSES}
        >
          {t("tts.scriptCheck.removeHere")}
        </button>
      </div>
    </div>
  );
};

/**
 * Befunde ueber dem Editor — wie die Rechtschreibpruefung in Office: nicht
 * alle auf einmal, sondern EINE Gruppe (ein Problem mit all seinen
 * Stellen), durch die man blaettert. Ohne Befund eine einzige gruene
 * Zeile, damit man sieht, dass geprueft wurde.
 */
export const ScriptCheckPanel: React.FC<ScriptCheckPanelProps> = ({
  findings,
  speakers,
  engine,
  uiLang,
  actions,
  onClose,
}) => {
  const { t } = useTranslation();
  const tags = knownTagsFor(engine);
  const groups = groupFindings(findings);
  const [index, setIndex] = useState(0);
  useEffect(() => {
    if (index > groups.length - 1) setIndex(Math.max(0, groups.length - 1));
  }, [groups.length, index]);

  if (groups.length === 0) {
    return (
      <p
        data-testid="script-check-clean"
        className="flex items-center gap-1.5 text-xs text-text/50"
      >
        <CheckCircle2 width={14} height={14} aria-hidden="true" />
        <span className="flex-1">{t("tts.scriptCheck.clean")}</span>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            className={NAV_CLASSES}
            aria-label={t("common.close")}
            data-testid="script-check-close"
          >
            <X width={14} height={14} />
          </button>
        )}
      </p>
    );
  }
  const group = groups[Math.min(index, groups.length - 1)];
  const errors = findings.filter((f) => !isHintFinding(f.kind)).length;
  const hints = findings.length - errors;
  return (
    <section
      data-testid="script-check"
      aria-label={t("tts.scriptCheck.title")}
      className={
        errors > 0
          ? "rounded-md border border-red-500/30 bg-red-500/5 px-3 py-1.5"
          : "rounded-md border border-logo-primary/30 bg-logo-primary/5 px-3 py-1.5"
      }
    >
      <div className="flex items-center justify-between gap-2">
        <p className="flex items-center gap-1.5 text-xs font-medium text-text">
          <AlertTriangle
            width={14}
            height={14}
            aria-hidden="true"
            className={errors > 0 ? "text-red-500" : "text-logo-primary"}
          />
          {errors > 0
            ? t("tts.scriptCheck.count", { count: errors })
            : t("tts.scriptCheck.hintCount", { count: hints })}
          {errors > 0 && hints > 0 && (
            <span className="text-text/60">
              {" · "}
              {t("tts.scriptCheck.hintCount", { count: hints })}
            </span>
          )}
          {groups.length > 1 && (
            <span className="text-text/60">
              {" · "}
              {t("tts.scriptCheck.groups", { count: groups.length })}
            </span>
          )}
        </p>
        <div className="flex items-center gap-1 text-xs text-text/70">
          {groups.length > 1 && (
            <>
              <button
                type="button"
                onClick={() => setIndex((i) => Math.max(0, i - 1))}
                disabled={index <= 0}
                className={NAV_CLASSES}
                aria-label={t("tts.scriptCheck.prevGroup")}
              >
                <ChevronLeft width={14} height={14} />
              </button>
              <span data-testid="script-check-group-position">
                {t("tts.scriptCheck.groupPosition", {
                  index: index + 1,
                  count: groups.length,
                })}
              </span>
              <button
                type="button"
                onClick={() =>
                  setIndex((i) => Math.min(groups.length - 1, i + 1))
                }
                disabled={index >= groups.length - 1}
                className={NAV_CLASSES}
                aria-label={t("tts.scriptCheck.nextGroup")}
              >
                <ChevronRight width={14} height={14} />
              </button>
            </>
          )}
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              className={`${NAV_CLASSES} ml-1`}
              aria-label={t("common.close")}
              data-testid="script-check-close"
            >
              <X width={14} height={14} />
            </button>
          )}
        </div>
      </div>
      <GroupCard
        key={group.key}
        group={group}
        speakers={speakers}
        tags={tags}
        engine={engine}
        uiLang={uiLang}
        actions={actions}
      />
    </section>
  );
};
