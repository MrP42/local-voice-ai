import type { TtsDownloadInfo } from "@/bindings";

/**
 * One-Click-Einrichtung von Phonon: aus den Download-Zeilen (Programm, Modelle mit
 * ihren Stimmen) wird ein Plan. Reine Logik ohne Tauri, damit sie testbar bleibt.
 *
 * Reihenfolge: Programm → je gewählte Sprache ihr Modell samt allen Stimmen. Ein
 * installiertes Modell, dem Stimmen fehlen, wird erneut angestoßen (das Backend lädt
 * dann nur die fehlenden Stimmen nach).
 */

export interface SetupLanguage {
  /** Download-Kennung des Modells (z. B. `phonon-40m-de`). */
  id: string;
  /** Sprachcode laut Katalog (`de`, `en`). */
  language: string;
  info: TtsDownloadInfo;
  voices: number;
  /** Modell und alle Stimmen liegen vor. */
  complete: boolean;
  /** Gesperrt bei Hugging Face: braucht Freigabe und Zugang. */
  gated: boolean;
  nonCommercial: boolean;
}

export interface SetupPlan {
  runtime: TtsDownloadInfo | null;
  languages: SetupLanguage[];
}

/** Größe einer Phonon-Stimme (`.safetensors`, Katalog). */
const VOICE_BYTES = 256_136;

const isPhonon = (d: TtsDownloadInfo) => d.engine === "phonon";

export function buildPlan(downloads: TtsDownloadInfo[]): SetupPlan {
  const phonon = downloads.filter(isPhonon);
  const runtime =
    phonon.find((d) => d.kind === "runtime" && !d.unsupported_reason) ?? null;
  const languages = phonon
    .filter((d) => d.kind === "model" && !d.unsupported_reason)
    .map((info) => {
      const voices = info.bundled_voices ?? 0;
      const present = info.bundled_voices_present ?? 0;
      return {
        id: info.id,
        language: info.language ?? "und",
        info,
        voices,
        complete: info.is_downloaded && present >= voices,
        gated: info.gated,
        nonCommercial: info.license_non_commercial,
      };
    })
    // Deutsch zuerst (Hauptsprache), dann alphabetisch.
    .sort((a, b) =>
      a.language === b.language
        ? a.id.localeCompare(b.id)
        : a.language === "de"
          ? -1
          : b.language === "de"
            ? 1
            : a.language.localeCompare(b.language),
    );
  return { runtime, languages };
}

/** Alles eingerichtet, was gewählt ist (Programm + gewählte Sprachen vollständig). */
export function isComplete(plan: SetupPlan, chosen: ReadonlySet<string>) {
  const runtimeOk = plan.runtime === null || plan.runtime.is_downloaded;
  return (
    runtimeOk &&
    plan.languages.filter((l) => chosen.has(l.id)).every((l) => l.complete)
  );
}

/** Ist irgendetwas von Phonon schon installiert? */
export function anyInstalled(plan: SetupPlan) {
  return (
    (plan.runtime?.is_downloaded ?? false) ||
    plan.languages.some((l) => l.info.is_downloaded)
  );
}

/** Die Download-Kennungen in Ausführungsreihenfolge (nur, was noch fehlt). */
export function steps(plan: SetupPlan, chosen: ReadonlySet<string>): string[] {
  const out: string[] = [];
  if (plan.runtime && !plan.runtime.is_downloaded) out.push(plan.runtime.id);
  for (const l of plan.languages) {
    if (chosen.has(l.id) && !l.complete) out.push(l.id);
  }
  return out;
}

/** Bytes, die für die Auswahl noch geladen werden (fertige Teile zählen nicht). */
export function pendingBytes(plan: SetupPlan, chosen: ReadonlySet<string>) {
  let total = 0;
  if (plan.runtime && !plan.runtime.is_downloaded)
    total += plan.runtime.size_bytes;
  for (const l of plan.languages) {
    if (!chosen.has(l.id) || l.complete) continue;
    total += l.info.is_downloaded
      ? VOICE_BYTES *
        Math.max(0, l.voices - (l.info.bundled_voices_present ?? 0))
      : l.info.size_bytes;
  }
  return total;
}

/** Stimmen der Auswahl. */
export function voiceCount(plan: SetupPlan, chosen: ReadonlySet<string>) {
  return plan.languages
    .filter((l) => chosen.has(l.id))
    .reduce((n, l) => n + l.voices, 0);
}

/**
 * Vorauswahl: alles, was schon da ist, dazu jede frei nutzbare Sprache (nicht gesperrt, nicht
 * nichtkommerziell, z. B. Englisch). So bietet die Einrichtung die englischen Stimmen auch an,
 * wenn Deutsch schon installiert ist. Ist noch nichts da: alle Sprachen, deren Freigabe möglich ist.
 */
export function defaultChoice(
  plan: SetupPlan,
  gatedPossible: boolean,
): Set<string> {
  const installed = plan.languages.filter((l) => l.info.is_downloaded);
  if (installed.length > 0) {
    const free = plan.languages.filter((l) => !l.gated && !l.nonCommercial);
    return new Set([...installed, ...free].map((l) => l.id));
  }
  return new Set(
    plan.languages.filter((l) => gatedPossible || !l.gated).map((l) => l.id),
  );
}

/** Verlangt die Auswahl eine Lizenzbestätigung? (nichtkommerzielle Gewichte) */
export function needsLicenseConsent(
  plan: SetupPlan,
  chosen: ReadonlySet<string>,
) {
  return plan.languages.some(
    (l) => chosen.has(l.id) && l.nonCommercial && !l.info.is_downloaded,
  );
}

/** Gewählte, noch nicht installierte, gesperrte Sprachen (brauchen den HF-Zugang). */
export function gatedPending(plan: SetupPlan, chosen: ReadonlySet<string>) {
  return plan.languages.filter(
    (l) => chosen.has(l.id) && l.gated && !l.info.is_downloaded,
  );
}
