import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronFirst,
  ChevronLast,
  ChevronLeft,
  ChevronRight,
  MoreHorizontal,
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
  /** Steht am Ende der Zeile (Schließen-Knopf des Panels). */
  trailing?: React.ReactNode;
}> = ({ group, speakers, engine, uiLang, actions, trailing }) => {
  const { t } = useTranslation();
  const [choice, setChoice] = useState("");
  const [pos, setPos] = useState(0);
  const [more, setMore] = useState(false);
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

  const primary = recommended[0];
  const spotNav = (
    <div
      className="flex shrink-0 items-center"
      role="group"
      aria-label={t("tts.scriptCheck.spotNav")}
    >
      <button
        type="button"
        onClick={() => goTo(0)}
        disabled={pos <= 0}
        className={NAV_CLASSES}
        aria-label={t("tts.scriptCheck.firstSpot")}
        title={t("tts.scriptCheck.firstSpot")}
      >
        <ChevronFirst width={14} height={14} />
      </button>
      <button
        type="button"
        onClick={() => goTo(pos - 1)}
        disabled={pos <= 0}
        className={NAV_CLASSES}
        aria-label={t("tts.scriptCheck.prevSpot")}
        title={t("tts.scriptCheck.prevSpot")}
      >
        <ChevronLeft width={14} height={14} />
      </button>
      {/* Die Position ist zugleich der Sprung zur aktuellen Stelle. */}
      <button
        type="button"
        onClick={() => goTo(pos)}
        className="h-6 min-w-[4.5rem] cursor-pointer rounded-md px-1 text-center text-xs tabular-nums text-text/80 hover:bg-mid-gray/20 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        data-testid="script-finding-position"
        aria-label={t("tts.scriptCheck.spot", { index: pos + 1, count })}
        title={t("tts.scriptCheck.spot", { index: pos + 1, count })}
      >
        {t("tts.scriptCheck.spotShort", { index: pos + 1, count })}
      </button>
      <button
        type="button"
        onClick={() => goTo(pos + 1)}
        disabled={pos >= count - 1}
        className={NAV_CLASSES}
        aria-label={t("tts.scriptCheck.nextSpot")}
        title={t("tts.scriptCheck.nextSpot")}
      >
        <ChevronRight width={14} height={14} />
      </button>
      <button
        type="button"
        onClick={() => goTo(count - 1)}
        disabled={pos >= count - 1}
        className={NAV_CLASSES}
        aria-label={t("tts.scriptCheck.lastSpot")}
        title={t("tts.scriptCheck.lastSpot")}
      >
        <ChevronLast width={14} height={14} />
      </button>
    </div>
  );

  const label = isSpeaker
    ? t("tts.scriptCheck.groupSpeaker", { name: group.name, count })
    : t("tts.scriptCheck.groupTag", { tag: group.name, count });

  return (
    <div data-testid="script-finding" className="min-w-0 flex-1">
      {/* Eine Zeile: Befund, Stellen blättern, eine Hauptaktion, Rest im
        Mehr-Menü (lva#148, Codex). Keine Liste aller Zeilen mehr. */}
      <div className="flex min-w-0 items-center gap-2">
        <p
          className="min-w-0 flex-1 truncate text-sm text-text"
          title={`${label} — ${message}`}
        >
          <span className="font-medium">{label}</span>
          <span className="text-text/70"> — {message}</span>
        </p>
        {spotNav}
        {primary && (
          <button
            type="button"
            data-testid="script-finding-recommendation"
            onClick={() => actions.replace(current, primary.apply, true)}
            className={`${PRIMARY_CLASSES} max-w-[12rem] truncate whitespace-nowrap`}
            title={`${primary.label} — ${t("tts.scriptCheck.applyAll", { count })}`}
          >
            {primary.label}
          </button>
        )}
        <button
          type="button"
          onClick={() => setMore((m) => !m)}
          aria-expanded={more}
          aria-label={t("tts.scriptCheck.moreActions")}
          title={t("tts.scriptCheck.moreActions")}
          className={NAV_CLASSES}
          data-testid="script-finding-more"
        >
          <MoreHorizontal width={14} height={14} />
        </button>
        {trailing}
      </div>

      {more && (
        <div className="mt-1.5 space-y-1.5" data-testid="script-finding-details">
          {recommended.length > 1 && (
            <div className="flex flex-wrap items-center gap-1">
              <span className="text-xs text-text/60">
                {t("tts.scriptCheck.recommended")}
              </span>
              {recommended.slice(1).map((item) => (
                <button
                  key={item.key}
                  type="button"
                  data-testid="script-finding-recommendation"
                  onClick={() => actions.replace(current, item.apply, true)}
                  className={`${PRIMARY_CLASSES} whitespace-nowrap`}
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
              className={`${ACTION_CLASSES} whitespace-nowrap`}
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
              className={`${ACTION_CLASSES} whitespace-nowrap`}
            >
              {t("tts.scriptCheck.replaceHere")}
            </button>
            <span className="mx-1 text-text/30">·</span>
            <button
              type="button"
              onClick={() => actions.replace(current, () => "", true)}
              className={`${ACTION_CLASSES} whitespace-nowrap`}
            >
              {t("tts.scriptCheck.removeAll", { count })}
            </button>
            <button
              type="button"
              onClick={() => actions.replace(current, () => "", false)}
              className={`${ACTION_CLASSES} whitespace-nowrap`}
            >
              {t("tts.scriptCheck.removeHere")}
            </button>
          </div>
        </div>
      )}
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
  const close = onClose && (
    <button
      type="button"
      onClick={onClose}
      className={NAV_CLASSES}
      aria-label={t("common.close")}
      data-testid="script-check-close"
    >
      <X width={14} height={14} />
    </button>
  );
  // Zusammenfassung für Tooltip und Screenreader; sichtbar bleibt nur das Symbol.
  const summary = [
    errors > 0
      ? t("tts.scriptCheck.count", { count: errors })
      : t("tts.scriptCheck.hintCount", { count: hints }),
    errors > 0 && hints > 0
      ? t("tts.scriptCheck.hintCount", { count: hints })
      : null,
    groups.length > 1
      ? t("tts.scriptCheck.groups", { count: groups.length })
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <section
      data-testid="script-check"
      aria-label={t("tts.scriptCheck.title")}
      className={
        errors > 0
          ? "rounded-md border border-red-500/30 bg-red-500/5 px-2 py-1"
          : "rounded-md border border-logo-primary/30 bg-logo-primary/5 px-2 py-1"
      }
    >
      <div className="flex min-w-0 items-start gap-2">
        <p
          className="flex h-7 shrink-0 items-center gap-1 text-xs font-medium text-text"
          title={summary}
        >
          <AlertTriangle
            width={14}
            height={14}
            aria-hidden="true"
            className={errors > 0 ? "text-red-500" : "text-logo-primary"}
          />
          <span className="sr-only">{summary}</span>
        </p>
        {groups.length > 1 && (
          <div className="flex h-7 shrink-0 items-center text-xs text-text/70">
            <button
              type="button"
              onClick={() => setIndex((i) => Math.max(0, i - 1))}
              disabled={index <= 0}
              className={NAV_CLASSES}
              aria-label={t("tts.scriptCheck.prevGroup")}
              title={t("tts.scriptCheck.prevGroup")}
            >
              <ChevronLeft width={14} height={14} />
            </button>
            <span
              data-testid="script-check-group-position"
              title={t("tts.scriptCheck.groups", { count: groups.length })}
              className="tabular-nums"
            >
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
              title={t("tts.scriptCheck.nextGroup")}
            >
              <ChevronRight width={14} height={14} />
            </button>
          </div>
        )}
        <GroupCard
          key={group.key}
          group={group}
          speakers={speakers}
          tags={tags}
          engine={engine}
          uiLang={uiLang}
          actions={actions}
          trailing={close}
        />
      </div>
    </section>
  );
};
