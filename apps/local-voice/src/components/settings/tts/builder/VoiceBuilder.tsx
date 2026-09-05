import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { FileAudio, Loader2, Trash2, Wand2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { commands, type VoiceMeta } from "@/bindings";
import { VOICE_RECIPES } from "@/lib/voices/recipes";
import { voiceColor } from "@/lib/voices/palette";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Textarea } from "../../../ui/Textarea";
import { CandidateCard } from "./CandidateCard";
import { useBuilderDraft } from "./useBuilderDraft";

/** So viele Kandidaten je Lauf: genug zum Aussuchen, wenig genug, dass der
 *  Lauf in einer Kaffeepause fertig ist. */
const CANDIDATE_COUNT = 6;

/** Ein geworfener Fehler (Tauri liefert bei harten Fehlern eine Exception)
 *  soll denselben Weg gehen wie ein `Result`-Fehler: als Text in die Karte. */
const asMessage = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

export const VoiceBuilder: React.FC<{ onSaved: (voiceId: string) => void }> = ({
  onSaved,
}) => {
  const { t } = useTranslation();
  const {
    draft,
    setDraft,
    drafts,
    reloadDrafts,
    progress,
    setProgress,
    error,
    setError,
    patch,
  } = useBuilderDraft();
  const [busy, setBusy] = useState(false);
  // Der Assistent muss auch ohne laufenden Sprachserver bedienbar bleiben:
  // scheitert ein Lauf, steht neben der technischen Meldung der Hinweis, wo
  // der Server gestartet wird — sonst sieht man nur eine leere Liste.
  const [serverHint, setServerHint] = useState(false);
  // Der Zuschnitt bleibt als Text im Zustand: ein leeres Feld ist ein
  // gueltiger Zwischenstand, `number | null` wuerde beim Tippen stolpern.
  const [trimStart, setTrimStart] = useState("");
  const [trimEnd, setTrimEnd] = useState("");

  /** Leeres oder unsinniges Feld heisst 0 — und 0/0 nimmt die ganze Datei. */
  const seconds = (value: string): number => {
    const n = Number(value);
    return Number.isFinite(n) && n > 0 ? n : 0;
  };

  const startFromRecipe = async (recipeId: string) => {
    const recipe = VOICE_RECIPES.find((r) => r.id === recipeId);
    if (!recipe) return;
    try {
      const res = await commands.ttsBuilderCreateDraft(
        recipe.name,
        recipe.description,
        recipe.probeText,
        recipe.tags,
      );
      if (res.status === "ok") {
        setError(null);
        setDraft(res.data);
        reloadDrafts();
      } else {
        setError(res.error);
      }
    } catch (e) {
      setError(asMessage(e));
    }
  };

  const startEmpty = async () => {
    try {
      const res = await commands.ttsBuilderCreateDraft("", "", "", []);
      if (res.status === "ok") {
        setError(null);
        setDraft(res.data);
        reloadDrafts();
      } else {
        setError(res.error);
      }
    } catch (e) {
      setError(asMessage(e));
    }
  };

  const generate = async () => {
    if (!draft) return;
    setBusy(true);
    setError(null);
    setServerHint(false);
    setProgress({ done: 0, total: CANDIDATE_COUNT });
    try {
      const res = await commands.ttsBuilderGenerate(draft.id, CANDIDATE_COUNT);
      if (res.status === "ok") {
        setDraft(res.data);
      } else {
        setError(res.error);
        setServerHint(true);
      }
    } catch (e) {
      setError(asMessage(e));
      setServerHint(true);
    } finally {
      setBusy(false);
      setProgress(null);
      reloadDrafts();
    }
  };

  // Eine eigene Aufnahme einspielen. Anders als beim Wuerfeln ist kein
  // Sprachserver beteiligt — deshalb hier auch kein Serverhinweis bei Fehlern.
  const addWav = async () => {
    if (!draft) return;
    let picked: string | string[] | null = null;
    try {
      picked = await open({
        multiple: false,
        filters: [{ name: t("tts.builder.wavFilter"), extensions: ["wav"] }],
      });
    } catch (e) {
      setError(asMessage(e));
      return;
    }
    if (typeof picked !== "string") return;
    setBusy(true);
    setError(null);
    setServerHint(false);
    try {
      const res = await commands.ttsBuilderAddWav(
        draft.id,
        picked,
        seconds(trimStart),
        seconds(trimEnd),
      );
      if (res.status === "ok") {
        setDraft(res.data);
      } else {
        setError(res.error);
      }
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
      reloadDrafts();
    }
  };

  const save = async () => {
    if (!draft) return;
    if (draft.selected === null) {
      setError(t("tts.builder.errorNoCandidate"));
      return;
    }
    const recipe = VOICE_RECIPES.find((r) => r.name === draft.display_name);
    const meta: VoiceMeta = {
      version: 1,
      display_name: draft.display_name,
      color: recipe?.color ?? "slate",
      avatar: null,
      language: "de-DE",
      description: draft.description,
      default_tags: draft.tags,
      default_style: null,
      styles: [],
      // Der Tiefe-Regler des Baukastens steckt bereits in der Referenz —
      // die dauerhaften Klangregler bleiben deshalb unbesetzt und werden
      // erst in der Stimmenverwaltung gesetzt.
      sound: null,
    };
    setBusy(true);
    setError(null);
    setServerHint(false);
    try {
      const res = await commands.ttsBuilderCommit(draft.id, meta);
      if (res.status === "ok") {
        setDraft(null);
        reloadDrafts();
        onSaved(res.data);
      } else {
        setError(res.error);
      }
    } catch (e) {
      setError(asMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const discard = async () => {
    if (!draft) return;
    try {
      await commands.ttsBuilderDeleteDraft(draft.id);
    } catch (e) {
      setError(asMessage(e));
    }
    setDraft(null);
    reloadDrafts();
  };

  // Fehler und Serverhinweis stehen an beiden Stellen des Assistenten gleich
  // aus — deshalb einmal gebaut statt zweimal geschrieben.
  const errorBlock = (error !== null || serverHint) && (
    <div className="space-y-1">
      {error !== null && (
        <p className="text-sm break-words text-red-500">{error}</p>
      )}
      {serverHint && (
        <p className="text-xs text-text/60">{t("tts.builder.errorNoServer")}</p>
      )}
    </div>
  );

  if (!draft) {
    return (
      <div className="space-y-3 rounded-lg border border-mid-gray/40 p-3">
        <p className="text-sm font-medium text-text">
          {t("tts.builder.recipes")}
        </p>
        <div className="flex flex-wrap gap-2">
          {/* Rezeptkacheln tragen einen Farbpunkt neben dem Namen — dafuer
              gibt die Button-Komponente keine Form her, die A11y-Regeln der
              Vorlesen-Oberflaeche gelten hier trotzdem. */}
          {VOICE_RECIPES.map((recipe) => (
            <button
              key={recipe.id}
              type="button"
              onClick={() => void startFromRecipe(recipe.id)}
              className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded-md border border-mid-gray/40 px-3 text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:outline-none"
            >
              <span
                aria-hidden="true"
                className="size-2.5 rounded-full"
                style={{ backgroundColor: voiceColor(recipe.color) }}
              />
              {recipe.name}
            </button>
          ))}
          <button
            type="button"
            onClick={() => void startEmpty()}
            className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded-md border border-dashed border-mid-gray/40 px-3 text-sm text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:outline-none"
          >
            <Wand2 width={15} height={15} aria-hidden="true" />
            {t("tts.builder.title")}
          </button>
        </div>

        {drafts.length > 0 && (
          <div className="space-y-1 border-t border-mid-gray/15 pt-2">
            <p className="text-xs font-semibold tracking-wide text-text/40 uppercase">
              {t("tts.builder.drafts")}
            </p>
            {drafts.map((d) => (
              <button
                key={d.id}
                type="button"
                onClick={() => setDraft(d)}
                title={t("tts.builder.resume")}
                className="flex min-h-[44px] w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:outline-none"
              >
                <span className="truncate">
                  {d.display_name || t("tts.builder.title")}
                </span>
                <span className="shrink-0 text-xs text-text/45">
                  {d.candidates.length}
                </span>
              </button>
            ))}
          </div>
        )}

        {errorBlock}
      </div>
    );
  }

  return (
    <div className="space-y-4 rounded-lg border border-mid-gray/40 p-3">
      <p className="text-[11px] leading-4 text-text/45">
        {t("tts.builder.seedNote")}
      </p>

      <div className="space-y-2">
        <label className="block text-sm text-text/80" htmlFor="builder-name">
          {t("tts.builder.nameLabel")}
        </label>
        <Input
          id="builder-name"
          type="text"
          value={draft.display_name}
          onChange={(e) => patch({ display_name: e.target.value })}
          placeholder={t("tts.builder.namePlaceholder")}
          className="w-full"
        />
        <label className="block text-sm text-text/80" htmlFor="builder-desc">
          {t("tts.builder.descriptionLabel")}
        </label>
        <Textarea
          id="builder-desc"
          value={draft.description}
          onChange={(e) => patch({ description: e.target.value })}
          placeholder={t("tts.builder.descriptionPlaceholder")}
          rows={4}
          className="w-full"
        />
        <label className="block text-sm text-text/80" htmlFor="builder-probe">
          {t("tts.builder.probeLabel")}
        </label>
        <Input
          id="builder-probe"
          type="text"
          value={draft.probe_text}
          onChange={(e) => patch({ probe_text: e.target.value })}
          className="w-full"
        />
        <p className="text-[11px] leading-4 text-text/45">
          {t("tts.builder.probeHint")}
        </p>
      </div>

      <div className="space-y-2">
        <label className="block text-sm text-text/80" htmlFor="builder-depth">
          {t("tts.builder.depthLabel")}
        </label>
        <input
          id="builder-depth"
          type="range"
          min={1}
          max={1.15}
          step={0.01}
          value={draft.depth}
          onChange={(e) => patch({ depth: Number(e.target.value) })}
          className="w-full cursor-pointer"
        />
        <p className="text-[11px] leading-4 text-text/45">
          {t("tts.builder.depthHint")}
        </p>
      </div>

      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={() => void generate()} disabled={busy}>
            {busy && (
              <Loader2
                width={15}
                height={15}
                className="animate-spin"
                aria-hidden="true"
              />
            )}
            {progress !== null
              ? t("tts.builder.generating", {
                  done: progress.done,
                  total: progress.total,
                })
              : t("tts.builder.generate")}
          </Button>
          <Button
            variant="secondary"
            onClick={() => void addWav()}
            disabled={busy}
          >
            <FileAudio width={15} height={15} aria-hidden="true" />
            {t("tts.builder.addWav")}
          </Button>
          {busy && (
            <Button
              variant="ghost"
              onClick={() => void commands.ttsBuilderCancel()}
            >
              {t("tts.builder.cancel")}
            </Button>
          )}
        </div>

        <p className="text-[11px] leading-4 text-text/45">
          {t("tts.builder.addWavHint")}
        </p>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <label className="text-xs text-text/60" htmlFor="builder-trim-start">
            {t("tts.builder.trimStartLabel")}
          </label>
          <Input
            id="builder-trim-start"
            type="number"
            min={0}
            step={1}
            variant="compact"
            value={trimStart}
            onChange={(e) => setTrimStart(e.target.value)}
            className="w-20"
          />
          <label className="text-xs text-text/60" htmlFor="builder-trim-end">
            {t("tts.builder.trimEndLabel")}
          </label>
          <Input
            id="builder-trim-end"
            type="number"
            min={0}
            step={1}
            variant="compact"
            value={trimEnd}
            onChange={(e) => setTrimEnd(e.target.value)}
            className="w-20"
          />
        </div>
        <p className="text-[11px] leading-4 text-text/45">
          {t("tts.builder.trimHint")}
        </p>

        <p className="text-xs font-semibold tracking-wide text-text/40 uppercase">
          {t("tts.builder.candidates")}
        </p>
        {draft.candidates.length === 0 ? (
          <p className="text-xs text-text/50">{t("tts.builder.empty")}</p>
        ) : (
          <div className="space-y-1">
            {draft.candidates.map((candidate) => (
              <CandidateCard
                key={candidate.seed}
                draftId={draft.id}
                candidate={candidate}
                chosen={draft.selected === candidate.seed}
                onChoose={(seed) => patch({ selected: seed })}
                onError={setError}
              />
            ))}
          </div>
        )}
      </div>

      {errorBlock}

      <div className="flex flex-wrap items-center gap-2 border-t border-mid-gray/15 pt-3">
        <Button
          onClick={() => void save()}
          disabled={busy || draft.selected === null}
        >
          {busy ? t("tts.builder.saving") : t("tts.builder.save")}
        </Button>
        <Button
          variant="danger-ghost"
          onClick={() => void discard()}
          disabled={busy}
        >
          <Trash2 width={15} height={15} aria-hidden="true" />
          {t("tts.builder.discard")}
        </Button>
      </div>
    </div>
  );
};
