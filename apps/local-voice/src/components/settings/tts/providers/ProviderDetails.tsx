import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type TtsError,
  type TtsProviderEntry,
  type VoiceDescriptor,
} from "@/bindings";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Select } from "../../../ui/Select";
import { Textarea } from "../../../ui/Textarea";
import { Switch } from "@fluentui/react-components";
import {
  ConfigInputError,
  LANG_ALL,
  VOICE_DEFAULT,
  asTtsError,
  buildConfig,
  isSelectableTier,
  parseConfig,
  previewIsActive,
  type FieldValue,
  type ParsedConfig,
} from "./providerModel";
import type { PreviewState } from "./useProviderPreview";

interface Props {
  entry: TtsProviderEntry;
  preview: PreviewState | null;
  onPreview: (text: string, voice: string | null) => void;
  onCancelPreview: () => void;
  /** Die Liste neu laden (Auswahl oder Konfiguration haben sich geaendert). */
  onChanged: () => void;
}

/** Fehlertext fuer den Nutzer: Ueberschrift nach Klasse, Meldung vom Backend. */
const useErrorText = () => {
  const { t } = useTranslation();
  return useCallback(
    (error: TtsError) =>
      t(`tts.providers.errors.${error.kind}`, {
        message: error.message,
        defaultValue: error.message,
      }),
    [t],
  );
};

/**
 * Aufgeklappter Bereich eines Anbieters: Einstellungen, Stimme/Sprache,
 * Probestimme. Alles faehigkeitsgesteuert -- die Felder kommen aus der
 * Konfiguration, die Sprachen aus den Faehigkeiten, ob ein Anbieter
 * Einstellungen hat, sagt der Befehl selbst.
 */
export const ProviderDetails: React.FC<Props> = ({
  entry,
  preview,
  onPreview,
  onCancelPreview,
  onChanged,
}) => {
  const id = entry.descriptor.id;
  // Die gewaehlte Stimme gilt fuer "Als Vorlese-Stimme verwenden" UND fuer die
  // Probestimme -- so hoert man, was man gleich verwendet.
  const [voice, setVoice] = useState<string>(entry.voice ?? VOICE_DEFAULT);
  return (
    <div
      className="mt-3 space-y-4 border-t border-mid-gray/20 pt-3"
      data-testid={`tts-provider-details-${id}`}
    >
      <ConfigSection entry={entry} onChanged={onChanged} />
      <VoiceSection
        entry={entry}
        voice={voice}
        onVoice={setVoice}
        onChanged={onChanged}
      />
      <PreviewSection
        entry={entry}
        preview={preview}
        onPreview={(text) =>
          onPreview(text, voice === VOICE_DEFAULT ? null : voice)
        }
        onCancelPreview={onCancelPreview}
      />
    </div>
  );
};

// ---------------------------------------------------------------------------

const ConfigSection: React.FC<{
  entry: TtsProviderEntry;
  onChanged: () => void;
}> = ({ entry, onChanged }) => {
  const { t } = useTranslation();
  const errorText = useErrorText();
  const id = entry.descriptor.id;
  const [state, setState] = useState<
    | { kind: "loading" }
    | { kind: "none" }
    | { kind: "error"; message: string }
    | { kind: "ready"; parsed: ParsedConfig }
  >({ kind: "loading" });
  const [values, setValues] = useState<Record<string, FieldValue>>({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const result = await commands.ttsProviderGetConfig(id).catch((e) => ({
        status: "error" as const,
        error: asTtsError(e),
      }));
      if (cancelled) return;
      if (result.status === "ok") {
        try {
          const parsed = parseConfig(result.data);
          setValues(parsed.values);
          setState({ kind: "ready", parsed });
        } catch (e) {
          setState({
            kind: "error",
            message: e instanceof Error ? e.message : String(e),
          });
        }
      } else if (result.error.kind === "bad_request") {
        // Fish und Piper haben keine Konfiguration in dieser Gruppe.
        setState({ kind: "none" });
      } else {
        setState({ kind: "error", message: errorText(result.error) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, errorText]);

  const save = async () => {
    if (state.kind !== "ready") return;
    setSaveError(null);
    setSaved(false);
    let json: string;
    try {
      json = buildConfig(state.parsed, values);
    } catch (e) {
      if (e instanceof ConfigInputError) {
        setSaveError(
          t(`tts.providers.config.${e.message}`, {
            field: t(`tts.providers.fields.${e.key}`, { defaultValue: e.key }),
          }),
        );
        return;
      }
      throw e;
    }
    setSaving(true);
    try {
      const result = await commands.ttsProviderSetConfig(id, json);
      if (result.status === "ok") {
        setSaved(true);
        onChanged();
      } else {
        setSaveError(errorText(result.error));
      }
    } catch (e) {
      setSaveError(errorText(asTtsError(e)));
    } finally {
      setSaving(false);
    }
  };

  if (state.kind === "loading") return null;
  return (
    <section
      aria-label={t("tts.providers.config.title")}
      data-testid={`tts-provider-config-${id}`}
    >
      <h3 className="text-xs font-medium text-mid-gray mb-2">
        {t("tts.providers.config.title")}
      </h3>
      {state.kind === "none" && (
        <p className="text-xs text-text/70">{t("tts.providers.config.none")}</p>
      )}
      {state.kind === "error" && (
        <p role="alert" className="text-xs text-red-500">
          {t("tts.providers.config.loadError", { message: state.message })}
        </p>
      )}
      {state.kind === "ready" && (
        <div className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-2">
            {state.parsed.fields.map((field) => {
              const label = t(`tts.providers.fields.${field.key}`, {
                defaultValue: field.key,
              });
              const hint = t(`tts.providers.fieldHints.${field.key}`, {
                defaultValue: "",
              });
              const inputId = `tts-provider-${id}-${field.key}`;
              const value = values[field.key];
              const set = (v: FieldValue) => {
                setSaved(false);
                setValues((cur) => ({ ...cur, [field.key]: v }));
              };
              return (
                <div key={field.key} className="min-w-0 space-y-1">
                  {field.kind === "bool" ? (
                    <Switch
                      checked={value === true}
                      label={label}
                      data-testid={inputId}
                      onChange={(_e, data) => set(data.checked)}
                    />
                  ) : (
                    <>
                      <label
                        htmlFor={inputId}
                        className="block text-xs text-text/80"
                      >
                        {label}
                      </label>
                      {field.kind === "choice" ? (
                        <Select
                          value={String(value) === "" ? "@none" : String(value)}
                          options={(field.choices ?? []).map((c) => ({
                            value: c === "" ? "@none" : c,
                            label:
                              c === ""
                                ? t("tts.providers.config.choiceNone")
                                : c,
                          }))}
                          isClearable={false}
                          ariaLabel={label}
                          onChange={(v) =>
                            set(v === "@none" || v === null ? "" : v)
                          }
                        />
                      ) : (
                        <Input
                          id={inputId}
                          data-testid={inputId}
                          type={field.kind === "number" ? "number" : "text"}
                          value={String(value ?? "")}
                          onChange={(e) => set(e.target.value)}
                          className="w-full"
                        />
                      )}
                    </>
                  )}
                  {hint && <p className="text-xs text-text/60">{hint}</p>}
                </div>
              );
            })}
          </div>
          {saveError && (
            <p
              role="alert"
              className="text-xs text-red-500"
              data-testid={`tts-provider-config-error-${id}`}
            >
              {saveError}
            </p>
          )}
          <div className="flex items-center gap-3">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={saving}
              data-testid={`tts-provider-config-save-${id}`}
              onClick={() => void save()}
            >
              {t("tts.providers.config.save")}
            </Button>
            {saved && (
              <span role="status" className="text-xs text-text/70">
                {t("tts.providers.config.saved")}
              </span>
            )}
          </div>
        </div>
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------

const VoiceSection: React.FC<{
  entry: TtsProviderEntry;
  voice: string;
  onVoice: (voice: string) => void;
  onChanged: () => void;
}> = ({ entry, voice, onVoice, onChanged }) => {
  const { t, i18n } = useTranslation();
  const errorText = useErrorText();
  const id = entry.descriptor.id;
  const blocked = entry.blocked !== null;
  const [language, setLanguage] = useState<string>(LANG_ALL);
  const [voices, setVoices] = useState<VoiceDescriptor[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [used, setUsed] = useState(false);

  const languageName = useCallback(
    (code: string) => {
      if (code === "und") return t("tts.providers.languageAny");
      try {
        return (
          new Intl.DisplayNames([i18n.language], { type: "language" }).of(
            code,
          ) ?? code
        );
      } catch {
        return code;
      }
    },
    [i18n.language, t],
  );

  useEffect(() => {
    if (blocked) {
      setVoices(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void (async () => {
      const result = await commands
        .ttsProviderVoices(id, language === LANG_ALL ? null : language)
        .catch((e) => ({ status: "error" as const, error: asTtsError(e) }));
      if (cancelled) return;
      setLoading(false);
      if (result.status === "ok") setVoices(result.data);
      else {
        setVoices(null);
        setError(errorText(result.error));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, language, blocked, errorText]);

  const options = useMemo(() => {
    const list = [
      { value: VOICE_DEFAULT, label: t("tts.providers.voices.default") },
      ...(voices ?? []).map((v) => ({
        value: v.id,
        label: `${v.label} · ${languageName(v.language)}`,
      })),
    ];
    if (voice !== VOICE_DEFAULT && !list.some((o) => o.value === voice))
      list.push({ value: voice, label: voice });
    return list;
  }, [voices, voice, languageName, t]);

  const languageOptions = useMemo(
    () => [
      { value: LANG_ALL, label: t("tts.providers.voices.allLanguages") },
      ...entry.descriptor.caps.languages.map((code) => ({
        value: code,
        label: languageName(code),
      })),
    ],
    [entry.descriptor.caps.languages, languageName, t],
  );

  const chosen = (voices ?? []).find((v) => v.id === voice);
  const canUse = isSelectableTier(entry) && !blocked;

  const use = async () => {
    setError(null);
    setUsed(false);
    const result = await commands
      .ttsProviderSelect(id, voice === VOICE_DEFAULT ? null : voice)
      .catch((e) => ({ status: "error" as const, error: asTtsError(e) }));
    if (result.status === "ok") {
      setUsed(true);
      onChanged();
    } else setError(errorText(result.error));
  };

  return (
    <section
      aria-label={t("tts.providers.voices.title")}
      data-testid={`tts-provider-voices-${id}`}
    >
      <h3 className="text-xs font-medium text-mid-gray mb-2">
        {t("tts.providers.voices.title")}
      </h3>
      {blocked ? (
        <p className="text-xs text-text/70">
          {t("tts.providers.voices.blocked")}
        </p>
      ) : (
        <div className="space-y-2">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="min-w-0 space-y-1">
              <span className="block text-xs text-text/80">
                {t("tts.providers.voices.language")}
              </span>
              <Select
                value={language}
                options={languageOptions}
                isClearable={false}
                menuPortal
                ariaLabel={t("tts.providers.voices.language")}
                onChange={(v) => setLanguage(v ?? LANG_ALL)}
              />
            </div>
            <div className="min-w-0 space-y-1">
              <span className="block text-xs text-text/80">
                {t("tts.providers.voices.voice")}
              </span>
              <Select
                value={voice}
                options={options}
                isClearable={false}
                menuPortal
                isLoading={loading}
                ariaLabel={t("tts.providers.voices.voice")}
                onChange={(v) => {
                  setUsed(false);
                  onVoice(v ?? VOICE_DEFAULT);
                }}
              />
            </div>
          </div>
          {voices !== null && voices.length === 0 && !loading && (
            <p className="text-xs text-text/70">
              {t("tts.providers.voices.none")}
            </p>
          )}
          {chosen?.license && (
            <p className="text-xs text-text/70">
              {t("tts.providers.voices.license", {
                license: chosen.license.weights || chosen.license.code,
              })}
              {chosen.license.non_commercial &&
                ` · ${t("tts.providers.license.nonCommercial")}`}
            </p>
          )}
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={!canUse}
              data-testid={`tts-provider-use-${id}`}
              onClick={() => void use()}
            >
              {t("tts.providers.voices.use")}
            </Button>
            {used && (
              <span role="status" className="text-xs text-text/70">
                {t("tts.providers.voices.usedNow")}
              </span>
            )}
          </div>
        </div>
      )}
      {error && (
        <p
          role="alert"
          className="mt-1 text-xs text-red-500"
          data-testid={`tts-provider-voices-error-${id}`}
        >
          {error}
        </p>
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------

const PreviewSection: React.FC<{
  entry: TtsProviderEntry;
  preview: PreviewState | null;
  onPreview: (text: string) => void;
  onCancelPreview: () => void;
}> = ({ entry, preview, onPreview, onCancelPreview }) => {
  const { t } = useTranslation();
  const errorText = useErrorText();
  const id = entry.descriptor.id;
  const [text, setText] = useState(() =>
    t("tts.providers.preview.defaultText"),
  );
  const mine = preview && preview.providerId === id ? preview : null;
  const active = mine !== null && previewIsActive(mine.phase);
  const blocked = entry.blocked !== null;

  const status = (() => {
    if (!mine) return null;
    switch (mine.phase) {
      case "starting":
      case "started":
        return t("tts.providers.preview.phase.started");
      case "first_audio":
        return t("tts.providers.preview.phase.first_audio", {
          ms: mine.ttfaMs ?? 0,
        });
      case "done":
        return mine.ttfaMs === null
          ? t("tts.providers.preview.phase.done")
          : t("tts.providers.preview.phase.doneTtfa", { ms: mine.ttfaMs });
      case "cancelled":
        return t("tts.providers.preview.phase.cancelled");
      case "error":
        return null;
    }
  })();

  return (
    <section
      aria-label={t("tts.providers.preview.title")}
      data-testid={`tts-provider-preview-${id}`}
    >
      <h3 className="text-xs font-medium text-mid-gray mb-2">
        {t("tts.providers.preview.title")}
      </h3>
      <div className="space-y-2">
        <label
          htmlFor={`tts-provider-preview-text-${id}`}
          className="block text-xs text-text/80"
        >
          {t("tts.providers.preview.text")}
        </label>
        <Textarea
          id={`tts-provider-preview-text-${id}`}
          data-testid={`tts-provider-preview-text-${id}`}
          value={text}
          rows={2}
          onChange={(e) => setText(e.target.value)}
          className="w-full"
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={blocked || active || text.trim() === ""}
            data-testid={`tts-provider-preview-play-${id}`}
            onClick={() => onPreview(text)}
          >
            {t("tts.providers.preview.play")}
          </Button>
          {active && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={mine?.requestId === null}
              data-testid={`tts-provider-preview-cancel-${id}`}
              onClick={onCancelPreview}
            >
              {t("tts.providers.preview.cancel")}
            </Button>
          )}
          {status && (
            <span
              role="status"
              className="text-xs text-text/80"
              data-testid={`tts-provider-preview-status-${id}`}
              data-phase={mine?.phase}
            >
              {status}
            </span>
          )}
        </div>
        {mine?.phase === "error" && (
          <p
            role="alert"
            className="text-xs text-red-500"
            data-testid={`tts-provider-preview-error-${id}`}
          >
            {errorText({
              kind: mine.errorKind ?? "backend",
              retryable: false,
              message: mine.message ?? "",
            })}
          </p>
        )}
      </div>
    </section>
  );
};
