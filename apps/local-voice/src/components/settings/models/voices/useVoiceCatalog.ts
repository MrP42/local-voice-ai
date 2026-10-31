import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type ProviderVoiceOption,
  type TtsProviderEntry,
  type VoiceDescriptor,
} from "@/bindings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { useVoiceChoice } from "../../tts/useVoiceChoice";
import { PROVIDER_VOICES_CHANGED } from "../../tts/providers/VisibleVoices";
import { useVoiceSamplesOf } from "../../tts/providers/useVoiceSamples";
import {
  PROVIDER_DEFAULT_VOICE,
  offersDropdownVoices,
  providerVoiceValue,
} from "../../tts/providers/providerModel";
import { isPhononRow } from "../PhononModels";
import {
  deviceGroups,
  licenseKind,
  providerIsReady,
  splitLabel,
  type DeviceGroup,
  type VoiceItem,
  type VoiceState,
} from "./model";

/**
 * Die Stimmen aller Anbieter als EINE Liste: Fish (Stimmen und Skript-Wahl),
 * Piper (geladene und ladbare Stimmen), Anbieter wie Phonon (ihre Stimmenliste).
 * Alles aus vorhandenen Befehlen und Speichern -- die Auswahl geht ueber
 * dieselbe Stimmenwahl wie die Vorlesen-Seite (`useVoiceChoice`).
 */
export function useVoiceCatalog(
  entries: TtsProviderEntry[] | null,
  /** Die Anbieterliste ist nicht ladbar: nicht endlos "laedt" melden. */
  failed = false,
) {
  const { t } = useTranslation();
  const choice = useVoiceChoice("");
  const downloads = useTtsModelStore((s) => s.downloads);
  const downloadingIds = useTtsModelStore((s) => s.downloadingIds);
  const downloadProgress = useTtsModelStore((s) => s.downloadProgress);
  const verifyingIds = useTtsModelStore((s) => s.verifyingIds);
  const downloadModel = useTtsModelStore((s) => s.downloadModel);
  const cancelDownload = useTtsModelStore((s) => s.cancelDownload);

  const [lists, setLists] = useState<Record<string, VoiceDescriptor[]>>({});
  const [options, setOptions] = useState<ProviderVoiceOption[] | null>(null);
  const [loading, setLoading] = useState(true);

  // Welche Anbieter ihre Stimmen einzeln nennen, und in welchem Zustand sie sind:
  // aendert sich das, wird neu gelesen (Freigabe erteilt, Anbieter bereit).
  const providerIds = useMemo(
    () =>
      (entries ?? [])
        .filter(offersDropdownVoices)
        .map(
          (e) =>
            `${e.descriptor.id}:${e.blocked === null}:${e.enabled}:${e.health.available}`,
        )
        .join(","),
    [entries],
  );

  useEffect(() => {
    if (entries === null) return;
    let cancelled = false;
    const load = async () => {
      // Gesperrte Anbieter nennen keine Stimmen (Freigabe fehlt): nichts abrufen.
      const targets = entries.filter(
        (e) => offersDropdownVoices(e) && e.blocked === null,
      );
      const results = await Promise.all(
        targets.map(async (e) => {
          const id = e.descriptor.id;
          try {
            const r = await commands.ttsProviderVoices(id, null);
            return [id, r.status === "ok" ? r.data : null] as const;
          } catch {
            return [id, null] as const;
          }
        }),
      );
      const opts = await commands
        .ttsProviderVoiceOptions()
        .then((r) => (r.status === "ok" && Array.isArray(r.data) ? r.data : []))
        .catch(() => [] as ProviderVoiceOption[]);
      if (cancelled) return;
      const next: Record<string, VoiceDescriptor[]> = {};
      for (const [id, list] of results) if (list) next[id] = list;
      setLists(next);
      setOptions(opts);
      setLoading(false);
    };
    void load();
    const again = () => void load();
    window.addEventListener("lv-voices-changed", again);
    window.addEventListener(PROVIDER_VOICES_CHANGED, again);
    return () => {
      cancelled = true;
      window.removeEventListener("lv-voices-changed", again);
      window.removeEventListener(PROVIDER_VOICES_CHANGED, again);
    };
    // providerIds fasst die relevanten Teile von `entries` zusammen.
  }, [providerIds, entries === null]);

  const sampleProviders = useMemo(() => Object.keys(lists).sort(), [lists]);
  const samples = useVoiceSamplesOf(sampleProviders);

  const { voiceValue, speakers, fishReady } = choice;

  const items = useMemo<VoiceItem[]>(() => {
    const byId = new Map((entries ?? []).map((e) => [e.descriptor.id, e]));
    const out: VoiceItem[] = [];
    const devicesOf = (id: string): DeviceGroup[] => {
      const e = byId.get(id);
      return e ? deviceGroups(e.descriptor.caps.devices) : [];
    };
    const providerLicense = (id: string, nonCommercial?: boolean) => {
      const lic = byId.get(id)?.descriptor.license;
      return licenseKind(
        nonCommercial ?? lic?.non_commercial ?? false,
        lic?.verified ?? false,
      );
    };
    const labelOf = (id: string) => byId.get(id)?.descriptor.label ?? id;
    const base = (
      providerId: string,
      value: string,
    ): Pick<
      VoiceItem,
      "providerId" | "providerLabel" | "value" | "devices" | "selected"
    > => ({
      providerId,
      providerLabel: labelOf(providerId),
      value,
      devices: devicesOf(providerId),
      selected: value === voiceValue,
    });

    // Fish Speech
    if (fishReady) {
      const fish = byId.get("fish");
      const state: VoiceState =
        fish && !providerIsReady(fish) ? "setup" : "ready";
      const lic = providerLicense("fish");
      out.push({
        ...base("fish", "@default"),
        key: "fish:@default",
        name: t("tts.voices.scriptVoices"),
        description: t("settings.models.voiceManager.scriptDescription"),
        language: "und",
        gender: null,
        license: lic,
        licenseName: fish?.descriptor.license.weights || null,
        state,
        sizeMb: null,
        downloadId: null,
        tryTarget: null,
        samplePath: undefined,
        hiddenInReadAloud: false,
      });
      for (const s of speakers) {
        out.push({
          ...base("fish", s.id),
          key: `fish:${s.id}`,
          name: s.displayName,
          description: null,
          language: s.language || "und",
          gender: null,
          license: lic,
          licenseName: fish?.descriptor.license.weights || null,
          state,
          sizeMb: null,
          downloadId: null,
          tryTarget: { kind: "fish", voiceId: s.id },
          samplePath: undefined,
          hiddenInReadAloud: false,
        });
      }
    }

    // Piper
    const piper = byId.get("piper");
    for (const d of downloads) {
      if (d.kind !== "voice" || isPhononRow(d)) continue;
      const downloading = d.id in downloadingIds;
      const state: VoiceState = d.unsupported_reason
        ? "unsupported"
        : downloading
          ? "downloading"
          : d.is_downloaded && d.is_usable
            ? "ready"
            : d.is_downloaded
              ? "runtime"
              : "load";
      const value = `piper:${d.id}`;
      out.push({
        ...base("piper", value),
        key: value,
        name: t(`settings.models.ttsVoices.voices.${d.id}.name`, {
          defaultValue: d.name,
        }),
        description:
          t(`settings.models.ttsVoices.voices.${d.id}.description`, {
            defaultValue: d.description,
          }) || null,
        language: d.language || "und",
        gender: null,
        license: d.license_non_commercial
          ? "nonCommercial"
          : licenseKind(false, piper?.descriptor.license.verified ?? false),
        licenseName: d.license,
        state,
        sizeMb: d.size_mb,
        downloadId: d.id,
        tryTarget:
          state === "ready"
            ? { kind: "provider", providerId: "piper", voice: d.id }
            : null,
        samplePath: undefined,
        hiddenInReadAloud: false,
      });
    }

    // Anbieter mit eigenen Stimmenlisten (Phonon, Server, ...)
    for (const [pid, list] of Object.entries(lists)) {
      const entry = byId.get(pid);
      if (!entry) continue;
      const state: VoiceState = providerIsReady(entry) ? "ready" : "setup";
      const listed = options?.filter((o) => o.provider_id === pid) ?? [];
      const defaults = listed
        .filter(
          (o) =>
            o.voice.id === PROVIDER_DEFAULT_VOICE &&
            !list.some((v) => v.id === PROVIDER_DEFAULT_VOICE),
        )
        .map((o) => o.voice);
      for (const v of [...defaults, ...list]) {
        const isDefault = v.id === PROVIDER_DEFAULT_VOICE;
        const option = listed.find((o) => o.voice.id === v.id);
        const script = isDefault && option?.voice_switching === true;
        const { name, description } = script
          ? {
              name: t("tts.providers.dropdown.script", {
                provider: entry.descriptor.label,
              }),
              description: null,
            }
          : splitLabel(v.label);
        const sample = samples[pid];
        out.push({
          ...base(pid, providerVoiceValue(pid, v.id)),
          key: `${pid}:${v.id}`,
          name,
          description,
          language: v.language || "und",
          gender: v.gender,
          license: v.license
            ? licenseKind(v.license.non_commercial, v.license.verified)
            : providerLicense(pid),
          licenseName: v.license?.weights ?? entry.descriptor.license.weights,
          state,
          sizeMb: null,
          downloadId: null,
          tryTarget:
            isDefault || state !== "ready"
              ? null
              : { kind: "provider", providerId: pid, voice: v.id },
          samplePath: sample && v.id in sample ? sample[v.id] : undefined,
          hiddenInReadAloud: listed.length > 0 && !isDefault && !option,
        });
      }
    }
    return out;
  }, [
    entries,
    downloads,
    downloadingIds,
    lists,
    options,
    samples,
    speakers,
    fishReady,
    voiceValue,
    t,
  ]);

  /** Anbieter, die einzelne Stimmen nennen koennten, es aber (noch) nicht tun. */
  const providersWithoutVoices = useMemo(
    () =>
      (entries ?? []).filter(
        (e) => offersDropdownVoices(e) && !(e.descriptor.id in lists),
      ),
    [entries, lists],
  );

  const runtimeId = useMemo(
    () =>
      downloads.find((d) => d.kind === "runtime" && !d.unsupported_reason)
        ?.id ?? null,
    [downloads],
  );

  const select = useCallback(
    (item: VoiceItem) => {
      if (item.state === "ready") choice.applyVoiceValue(item.value);
    },
    [choice],
  );

  const activeName = useMemo(
    () => choice.options.find((o) => o.value === voiceValue)?.label ?? null,
    [choice.options, voiceValue],
  );

  return {
    items,
    loading: loading && !failed,
    providersWithoutVoices,
    runtimeId,
    select,
    activeName,
    install: downloadModel,
    cancelInstall: cancelDownload,
    progressOf: (id: string) =>
      id in verifyingIds
        ? ("verifying" as const)
        : (downloadProgress[id]?.percentage ?? 0),
  };
}

export type VoiceCatalog = ReturnType<typeof useVoiceCatalog>;
