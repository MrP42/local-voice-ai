import { useSettings } from "@/hooks/useSettings";
import type { ScriptEngine } from "@/lib/voices/scriptCheck";
import { useProviderInfo } from "./providers/useProviderLabel";

/** Welche Art Engine liest vor: Fish, Piper oder ein Anbieter aus der Registry. */
export type EngineKind = "fish" | "piper" | "provider";

/**
 * Was die gewaehlte Engine kann, so wie die Vorlesen-Seite es braucht. Eine Stelle
 * statt vieler `=== "fish"`/`=== "piper"`-Vergleiche: Sprecher im Skript, Stil-Tags,
 * Skript-Pruefung richten sich nach den Faehigkeiten, nicht nach dem Namen.
 *
 * * Fish: Sprecher (aufgenommene Stimmen) und Stil-Tags.
 * * Piper: weder noch (nur Pausen); eine Stimme je Lauf.
 * * Anbieter (`provider:<id>`, Phonon u. a.): was sein Beschreibungsblatt sagt
 *   (`voice_switching`, `style_tags`). Bis es geladen ist, nichts: lieber kurz
 *   zurueckhaltend als Sprecher oder Tags anbieten, die nicht wirken.
 */
export interface EngineCaps {
  kind: EngineKind;
  /** Kennung des Anbieters bei `kind === "provider"`, sonst `null`. */
  providerId: string | null;
  /** Anzeigename des Anbieters (bis zur Antwort seine Kennung), sonst `null`. */
  providerLabel: string | null;
  /** Wechselt die Stimme je Satz: Marker `<Name>` im Skript gelten. */
  voiceSwitching: boolean;
  /** Setzt Stil-Tags `[...]` um (sonst nur Pausen). */
  styleTags: boolean;
  /** Fuer die Skript-Pruefung (`checkScript`). */
  scriptEngine: ScriptEngine;
}

const PROVIDER_PREFIX = "provider:";

/** Kennung des Anbieters aus `tts_engine`; `null` fuer Fish, Piper und alles andere. */
export function providerIdOfEngine(setting: string): string | null {
  if (!setting.startsWith(PROVIDER_PREFIX)) return null;
  const id = setting.slice(PROVIDER_PREFIX.length);
  // `provider:fish`/`provider:piper` sind Schreibweisen der eingebauten Engines.
  return id && id !== "fish" && id !== "piper" ? id : null;
}

export function useEngineCaps(): EngineCaps {
  const { getSetting } = useSettings();
  const setting = getSetting("tts_engine") ?? "fish";
  const providerId = providerIdOfEngine(setting);
  const info = useProviderInfo(providerId);
  if (providerId) {
    const styleTags = info?.styleTags ?? false;
    return {
      kind: "provider",
      providerId,
      providerLabel: info?.label ?? providerId,
      voiceSwitching: info?.voiceSwitching ?? false,
      styleTags,
      scriptEngine: styleTags ? "fish" : "provider",
    };
  }
  if (setting === "piper" || setting === `${PROVIDER_PREFIX}piper`) {
    return {
      kind: "piper",
      providerId: null,
      providerLabel: null,
      voiceSwitching: false,
      styleTags: false,
      scriptEngine: "piper",
    };
  }
  return {
    kind: "fish",
    providerId: null,
    providerLabel: null,
    voiceSwitching: true,
    styleTags: true,
    scriptEngine: "fish",
  };
}
