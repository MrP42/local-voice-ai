import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type ProviderVoiceOption } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { sortVoicesByLabel } from "@/lib/voices/sortVoices";
import { openSettingsTab } from "@/lib/openSettingsTab";
import type { SelectOption } from "../../ui/Select";
import { useSpeakers } from "./speakers";
import { useProviderLabel } from "./providers/useProviderLabel";
import {
  PROVIDER_DEFAULT_VOICE,
  asTtsError,
  parseProviderVoiceValue,
  providerVoiceValue,
  shortVoiceName,
} from "./providers/providerModel";
import { PROVIDER_VOICES_CHANGED } from "./providers/VisibleVoices";

/** Eintrag der Stimmenauswahl, der keine Stimme ist, sondern ein Sprung zur Verwaltung. */
export const VOICE_MANAGE = "@manage";

/**
 * Die Stimmenwahl der Vorlesen-Seite als Baustein: Liste, Wert, Wechsel.
 *
 * Eine Quelle fuer die Vorlesen-Seite (`TtsSettings`) und die Wiedergabeleiste in den
 * Aufnahmen: dieselben Eintraege, dieselben Werte (`@default`, Fish-Stimmen-Kennung,
 * `piper:<id>`, `provider:<id>:<stimme>`), und ein Wechsel wirkt ueberall gleich -- erst alle
 * Einstellungen schreiben, dann dem Backend Bescheid geben (`tts_voice_changed`), damit
 * ein laufendes Vorlesen ab dem aktuellen Satz die neue Stimme nimmt.
 *
 * Anbieter (Phonon u. a.) stehen mit ihren Stimmen in der Liste, aber nur mit den
 * *sichtbaren*: welche das sind, stellt man unter Einstellungen > Ausgabe >
 * Sprachausgabe-Anbieter ein (`tts_provider_voice_options` liefert nur diese; die
 * gerade gewaehlte Stimme bleibt immer dabei). Die Wahl setzt Engine und Stimme
 * wie `tts_provider_select`.
 *
 * `phase`: Phase des Sprachservers (Fish); ein laufender oder startender Server gilt
 * immer als eingerichtet.
 */
export function useVoiceChoice(phase: string) {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, refreshSettings } = useSettings();
  const uiLang = i18n.language?.split("-")[0] ?? "en";
  const speakers = useSpeakers();
  const [voices, setVoices] = useState<string[]>([]);
  /* Anbieter-Wahl, die gerade unterwegs ist: die Reiter-Logik der Vorlesen-Seite ruft
     `applyVoiceValue` mit demselben Wert noch einmal, bevor die Einstellungen neu geladen
     sind -- ein zweiter `tts_provider_select` waere nur ein zweiter Stimmwechsel. */
  const pendingProviderValue = useRef<string | null>(null);
  const [providerVoices, setProviderVoices] = useState<ProviderVoiceOption[]>(
    [],
  );

  // Stimmenliste fuer das Dropdown. Die Verwaltung meldet Aenderungen ueber ein
  // Fensterereignis, damit beide nie auseinanderlaufen.
  useEffect(() => {
    const load = () => {
      void commands.ttsListVoices().then((r) => {
        if (r.status === "ok") setVoices(r.data);
      });
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    return () => window.removeEventListener("lv-voices-changed", load);
  }, []);

  // Heruntergeladene Piper-Stimmen und der Zustand der Laufzeiten kommen aus
  // dem Modell-Speicher: er folgt jedem Download, jeder Reparatur und jedem
  // Entfernen sofort (Issue #29), statt die Liste nur beim Oeffnen zu lesen.
  // Eine Stimme ist erst "usable", wenn auch das Piper-Programm vollstaendig
  // ist -- sonst scheitert das Vorlesen erst spaet.
  const ttsDownloads = useTtsModelStore((state) => state.downloads);
  const ttsRuntime = useTtsModelStore((state) => state.runtime);
  const loadTtsRuntime = useTtsModelStore((state) => state.loadRuntime);
  const fishDirSetting = getSetting("tts_fish_dir") ?? "";
  useEffect(() => {
    // Der Fish-Ordner kann sich aendern (Einstellung, Installation von Hand).
    void loadTtsRuntime();
  }, [fishDirSetting, loadTtsRuntime]);
  // Unbekannt (noch nicht geladen) gilt als vorhanden: kein Aufblitzen von
  // "nicht eingerichtet" beim Start und unveraendertes Verhalten ohne Antwort.
  // Ein laufender oder startender Fish-Server (auch ein fremd gestarteter, den
  // die App uebernommen hat) gilt immer als eingerichtet.
  const fishReady = ttsRuntime
    ? ttsRuntime.fish.ready ||
      phase === "starting" ||
      phase === "ready" ||
      phase === "speaking"
    : true;
  const piperVoices = useMemo(
    () =>
      ttsDownloads
        .filter(
          (entry) =>
            entry.kind === "voice" &&
            entry.is_downloaded &&
            entry.engine !== "phonon",
        )
        .map((entry) => ({
          id: entry.id,
          name: entry.name,
          language: entry.language,
          usable: entry.is_usable !== false,
        })),
    [ttsDownloads],
  );

  /* Der Wert des Stimmen-Dropdowns aus den Einstellungen: Piper-Stimme als
     "piper:<id>", sonst Fish-Stimme oder Standard. */
  const ttsEngineSetting = getSetting("tts_engine") ?? "fish";
  /* Liest ein Anbieter vor (`provider:<id>`, Goal tts-provider), gehoert die
     Stimme dem Anbieter und wird in dessen Einstellungen gewaehlt: das
     Dropdown zeigt dann nur einen benannten Eintrag statt faelschlich die
     Fish-Standardstimme. */
  const providerEngineId = ttsEngineSetting.startsWith("provider:")
    ? ttsEngineSetting.slice("provider:".length)
    : null;
  const providerLabel = useProviderLabel(providerEngineId);
  /* Die gemerkte Stimme des liest-vor-Anbieters (`tts_providers.voices`); leer = Vorgabe. */
  const providerVoice = providerEngineId
    ? (getSetting("tts_providers")?.voices?.[providerEngineId] ??
      PROVIDER_DEFAULT_VOICE)
    : null;

  /* Stimmen der freigegebenen Anbieter, nur die sichtbaren. Neu laden, wenn sich die
     Verwaltung (Modelle, Auswahl "im Vorlesen anzeigen") oder die Wahl aendert. */
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void commands
        .ttsProviderVoiceOptions()
        .then((r) => {
          if (cancelled) return;
          setProviderVoices(
            r.status === "ok" && Array.isArray(r.data) ? r.data : [],
          );
        })
        .catch(() => {
          if (!cancelled) setProviderVoices([]);
        });
    };
    load();
    window.addEventListener("lv-voices-changed", load);
    window.addEventListener(PROVIDER_VOICES_CHANGED, load);
    return () => {
      cancelled = true;
      window.removeEventListener("lv-voices-changed", load);
      window.removeEventListener(PROVIDER_VOICES_CHANGED, load);
    };
  }, [providerEngineId, providerVoice]);

  const voiceValue = providerEngineId
    ? providerVoiceValue(
        providerEngineId,
        providerVoice ?? PROVIDER_DEFAULT_VOICE,
      )
    : ttsEngineSetting === "piper"
      ? `piper:${getSetting("tts_piper_voice") ?? ""}`
      : (getSetting("tts_voice") ?? "@default");

  /* Eine Dropdown-Wahl in die Einstellungen schreiben: Piper-Stimme schaltet
     die Engine um, eine Fish-Stimme schaltet zurueck. */
  const applyVoiceValue = useCallback(
    (value: string) => {
      // Erst alle Einstellungen schreiben, dann dem Backend Bescheid geben: ein
      // laufendes Vorlesen soll die neue Stimme ab dem aktuellen Satz nehmen.
      void (async () => {
        const picked = parseProviderVoiceValue(value);
        if (picked) {
          if (pendingProviderValue.current === value) return;
          pendingProviderValue.current = value;
          // Anbieter und Stimme wie `tts_provider_select` (prueft Freigaben, setzt Engine und
          // Stimme, stoesst den Stimmwechsel an). Ein Fehler (z. B. gesperrt) wird gezeigt.
          const result = await commands
            .ttsProviderSelect(
              picked.providerId,
              picked.voice === PROVIDER_DEFAULT_VOICE ? null : picked.voice,
            )
            .catch((e) => ({ status: "error" as const, error: asTtsError(e) }));
          if (result.status === "error") toast.error(result.error.message);
          await refreshSettings();
          pendingProviderValue.current = null;
          return;
        }
        if (value.startsWith("piper:")) {
          await updateSetting("tts_piper_voice", value.slice(6));
          await updateSetting("tts_engine", "piper");
        } else {
          if ((getSetting("tts_engine") ?? "fish") !== "fish") {
            await updateSetting("tts_engine", "fish");
          }
          await updateSetting("tts_voice", value === "@default" ? null : value);
        }
        await commands.ttsVoiceChanged().catch(() => undefined);
      })();
    },
    [getSetting, updateSetting, refreshSettings],
  );

  /* Ist die gerade gewaehlte Stimme nicht eingerichtet? Dann sagt ein Hinweis
     unter dem Dropdown, warum -- und wohin man geht. `noVoiceSetUp`: gar keine
     Stimme ist verwendbar (weder Fish noch Piper). */
  const selectedPiper = voiceValue.startsWith("piper:")
    ? piperVoices.find((v) => `piper:${v.id}` === voiceValue)
    : undefined;
  const anyPiperUsable = piperVoices.some((v) => v.usable);
  const selectedVoiceNotSetUp = providerEngineId
    ? false
    : voiceValue.startsWith("piper:")
      ? !selectedPiper || !selectedPiper.usable
      : !fishReady;
  const noVoiceSetUp = !fishReady && !anyPiperUsable;

  /* Beschriftung einer Piper-Stimme in der Auswahl: Name, Sprache, Qualitaet
     kurz -- "Thorsten · Deutsch · HQ". Die Sprache steht IMMER dabei: am Namen
     allein erkennt niemand, ob eine Stimme Deutsch oder Englisch spricht, und
     genau davon haengt ab, ob sie brauchbar klingt. */
  const piperVoiceLabel = useCallback(
    (voice: { id: string; name: string; language: string | null }) => {
      const localized = t(`settings.models.ttsVoices.voices.${voice.id}.name`, {
        defaultValue: voice.name,
      });
      const base = localized.replace(/\s*\(.*\)\s*$/, "").trim() || voice.id;
      let language = voice.language ?? "";
      try {
        if (language) {
          language =
            new Intl.DisplayNames([uiLang], { type: "language" }).of(
              language,
            ) ?? language;
        }
      } catch {
        /* unbekanntes Kuerzel: roh anzeigen */
      }
      const quality = voice.id.endsWith("-high")
        ? "HQ"
        : voice.id.endsWith("-medium")
          ? "MQ"
          : voice.id.endsWith("-x_low")
            ? "LQ-"
            : voice.id.endsWith("-low")
              ? "LQ"
              : "";
      return [base, language, quality].filter(Boolean).join(" · ");
    },
    [t, uiLang],
  );

  /* Beschriftung einer Anbieter-Stimme: "Anbieter: Name · weiblich"; alphabetisch. */
  const providerVoiceItems: SelectOption[] = sortVoicesByLabel(
    providerVoices.map((option) => {
      const name = shortVoiceName(option.voice.label);
      const gender = option.voice.gender
        ? t(`tts.providers.voices.gender.${option.voice.gender}`, {
            defaultValue: option.voice.gender,
          })
        : null;
      return {
        value: providerVoiceValue(option.provider_id, option.voice.id),
        label: t("tts.providers.dropdown.voice", {
          provider: option.provider_label,
          name: [name, gender].filter(Boolean).join(" \u00b7 "),
        }),
      };
    }),
    (o) => o.label,
  );

  /** Die Eintraege des Stimmen-Dropdowns, in der Reihenfolge der Vorlesen-Seite. */
  const options: SelectOption[] = [
    // Fish-Stimmen nur, wenn Fish eingerichtet ist (#29).
    ...(fishReady
      ? [
          {
            value: "@default",
            label: t("tts.voices.scriptVoices"),
          },
          // Anzeigename statt technischer Kennung. Alphabetisch nach diesem
          // Namen, nicht nach der Kennung (siehe sortVoices).
          ...sortVoicesByLabel(
            voices.map((id) => ({
              value: id,
              label: speakers.find((sp) => sp.id === id)?.displayName ?? id,
            })),
            (o) => o.label,
          ),
        ]
      : []),
    // Nur verwendbare Piper-Stimmen sind waehlbar; eine geladene Stimme ohne
    // vollstaendiges Programm steht als "nicht eingerichtet" da, nicht als
    // Fehler erst beim Vorlesen.
    ...piperVoices.map((voice) => ({
      value: `piper:${voice.id}`,
      label: voice.usable
        ? t("tts.voices.piperOption", { name: piperVoiceLabel(voice) })
        : t("tts.voices.piperNotSetUp", { name: piperVoiceLabel(voice) }),
      isDisabled: !voice.usable,
    })),
    // Die gewaehlte Fish-Stimme, obwohl Fish nicht eingerichtet ist: sichtbar
    // benannt statt still durch eine andere ersetzt. Nicht waehlbar.
    ...(!fishReady && !providerEngineId && !voiceValue.startsWith("piper:")
      ? [
          {
            value: voiceValue,
            label: t("tts.voices.fishNotSetUp"),
            isDisabled: true,
          },
        ]
      : []),
    // Piper ist aktiv, aber die Stimme fehlt (keine gewaehlt oder nicht mehr in
    // der Liste): sonst zeigte das Select den Rohwert "piper:". Nicht waehlbar.
    ...(voiceValue.startsWith("piper:") &&
    !piperVoices.some((v) => `piper:${v.id}` === voiceValue)
      ? [
          {
            value: voiceValue,
            label:
              voiceValue === "piper:"
                ? t("tts.voices.piperNoVoice")
                : t("tts.voices.piperVoiceMissing", {
                    id: voiceValue.slice(6),
                  }),
            isDisabled: true,
          },
        ]
      : []),
    // Stimmen der freigegebenen Anbieter (nur die sichtbaren): "Phonon: Nele · weiblich".
    ...providerVoiceItems,
    // Ein Anbieter liest vor, seine Stimme steht aber nicht in der Liste (noch nicht geladen,
    // nicht mehr sichtbar-gemacht oder nicht mehr freigegeben): benannter Eintrag, nicht waehlbar.
    ...(providerEngineId &&
    !providerVoiceItems.some((o) => o.value === voiceValue)
      ? [
          {
            value: voiceValue,
            label: t("tts.providers.dropdown.entry", {
              name: providerLabel ?? providerEngineId,
            }),
            isDisabled: true,
          },
        ]
      : []),
    // Kein Wert, ein Sprung: zur Stimmenverwaltung unter Einstellungen -> Ausgabe.
    { value: VOICE_MANAGE, label: t("tts.voices.manage") },
  ];

  /** Sprung zur Stimmenverwaltung (Eintrag `VOICE_MANAGE`). */
  const openVoiceManagement = useCallback(() => {
    openSettingsTab("output");
  }, []);

  return {
    voices,
    speakers,
    fishReady,
    piperVoices,
    providerEngineId,
    providerLabel,
    voiceValue,
    applyVoiceValue,
    selectedVoiceNotSetUp,
    noVoiceSetUp,
    piperVoiceLabel,
    options,
    openVoiceManagement,
  };
}
