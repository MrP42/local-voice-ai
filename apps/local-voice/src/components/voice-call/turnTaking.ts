/**
 * Gesprächsführung als reiner Zustandsautomat (ohne Audio, ohne Zeitgeber):
 * er bekommt je Frame Zeit und Pegel und meldet, wann gesprochen wird, wann
 * ein Satzende wahrscheinlich ist (früh, zum spekulativen Vorarbeiten), wann
 * es sicher ist, und wann der Sprecher die KI unterbricht.
 *
 * Zwei Schwellen für das Satzende sind der Kern der gefühlten Latenz:
 * bei `earlyEndMs` Stille beginnen Erkennung und Antwort schon, bei `endMs`
 * wird die Antwort freigegeben. Spricht der Nutzer vorher weiter, wird die
 * Vorarbeit verworfen (`resume`).
 */

export interface TurnConfig {
  /** Stille bis zum spekulativen Start (ms). */
  earlyEndMs: number;
  /** Stille bis zum sicheren Satzende (ms). */
  endMs: number;
  /** Dauer der Sprache, ab der ein Sprechbeginn gilt (ms). */
  minSpeechMs: number;
  /** Äußerungen unter dieser Sprachdauer sind Geräusche (ms). */
  minUtteranceMs: number;
  /** Absoluter Mindestpegel (RMS, 0..1). */
  minLevel: number;
  /** Faktor über dem Rauschboden. */
  noiseRatio: number;
  /** Zusätzlicher Faktor auf den Schwellen, solange die KI spricht. */
  duringAiFactor: number;
  /** Dauer ununterbrochener Sprache, ab der die KI unterbrochen wird (ms). */
  bargeInMs: number;
  /** Unterbrechen überhaupt erlauben. */
  bargeIn: boolean;
  /** Inhaltsfreie Ereignisdiagnose auf der Debug-Konsole einschalten. */
  debug?: boolean;
}

export const DEFAULT_TURN_CONFIG: TurnConfig = {
  earlyEndMs: 600,
  endMs: 1200,
  minSpeechMs: 120,
  minUtteranceMs: 300,
  minLevel: 0.012,
  noiseRatio: 3.2,
  duringAiFactor: 1.8,
  bargeInMs: 260,
  bargeIn: true,
};

export type TurnEvent =
  | { kind: "speech_start"; at: number }
  | { kind: "early_end"; at: number; speechMs: number; lastSpeechAt: number }
  | { kind: "resume"; at: number }
  | { kind: "end"; at: number; speechMs: number; lastSpeechAt: number }
  | { kind: "barge_in"; at: number };

/** Nur die ersetzte Äußerung samt zugehöriger Antwort aus dem Verlauf nehmen. */
export function withoutTurnHistory<T extends { role: string; turnId?: string }>(
  history: T[],
  turnId: string,
): T[] {
  const i = history.findIndex((m) => m.role === "user" && m.turnId === turnId);
  if (i < 0) return history.slice();
  const count = history[i + 1]?.role === "assistant" ? 2 : 1;
  return [...history.slice(0, i), ...history.slice(i + count)];
}

export class TurnTaker {
  private speaking = false;
  private candidateSince: number | null = null;
  private lastSpeechAt = 0;
  private speechStart = 0;
  private speechMs = 0;
  private earlyFired = false;
  private noise = 0.004;
  private lastFrameAt: number | null = null;
  private bargeSince: number | null = null;
  private bargeFired = false;
  private speechLevel = 0;
  private earlyAt: number | null = null;
  private resuming = false;
  private resumeSince: number | null = null;
  private stableSince: number | null = null;
  private noiseLevels: Array<{ at: number; level: number }> = [];
  private sawQuiet = false;
  private quietSince: number | null = null;

  config: TurnConfig;

  constructor(config: TurnConfig = DEFAULT_TURN_CONFIG) {
    this.config = config;
  }

  get isSpeaking(): boolean {
    return this.speaking;
  }

  reset(): void {
    this.speaking = false;
    this.candidateSince = null;
    this.earlyFired = false;
    this.speechMs = 0;
    this.bargeSince = null;
    this.bargeFired = false;
    this.speechLevel = 0;
    this.earlyAt = null;
    this.resuming = false;
    this.resumeSince = null;
    this.lastFrameAt = null;
    this.stableSince = null;
    this.noiseLevels = [];
    this.sawQuiet = false;
    this.quietSince = null;
  }

  /** Aktuelle Sprechschwelle (RMS). */
  threshold(aiSpeaking: boolean): number {
    const base = Math.max(
      this.config.minLevel,
      this.noise * this.config.noiseRatio,
    );
    const floor = Math.max(
      this.config.minLevel,
      this.noise * this.config.noiseRatio * 0.9,
    );
    const active = this.speaking && this.speechLevel > 0 && this.sawQuiet
      ? Math.max(floor, Math.min(base, this.speechLevel * 0.3))
      : base;
    const limited = this.speaking && this.speechLevel > 0 && this.sawQuiet
      ? Math.min(active, this.speechLevel * 0.6)
      : active;
    return aiSpeaking ? limited * this.config.duringAiFactor : limited;
  }

  /** Nur Ereignisse protokollieren, niemals Audiodaten oder erkannten Text. */
  private diagnose(
    events: TurnEvent[],
    at: number,
    level: number,
    threshold: number,
  ): TurnEvent[] {
    if (this.config.debug) {
      for (const event of events) {
        console.debug("[voice-call:turn]", {
          kind: event.kind,
          at: event.at,
          frameAt: at,
          lastSpeechAt: this.lastSpeechAt,
          rms: level,
          threshold,
          noise: this.noise,
          speechLevel: this.speechLevel,
        });
      }
    }
    return events;
  }

  /** Ein Frame. `at` in ms (monoton), `level` = RMS 0..1. */
  frame(at: number, level: number, aiSpeaking: boolean): TurnEvent[] {
    const dt =
      this.lastFrameAt === null ? 0 : Math.max(0, at - this.lastFrameAt);
    this.lastFrameAt = at;
    // Zeitbezogener Referenzpegel: Halbwertszeit vier Sekunden, auch in Pausen.
    this.speechLevel *= Math.pow(0.5, dt / 4000);
    const events: TurnEvent[] = [];
    // Auch nach dem sicheren Ende gehört eine schnelle Ergänzung noch zur Äußerung.
    const continuation = this.earlyAt !== null && at - this.earlyAt <= 1500;
    const pendingResumeSince = !continuation ? this.resumeSince : null;
    if (!continuation) this.resumeSince = null;
    if (!aiSpeaking) this.resuming = false;
    const threshold = this.threshold(aiSpeaking);
    const voiced = level > threshold &&
      !(aiSpeaking && !this.config.bargeIn && (continuation || this.earlyFired));

    const quiet = !voiced && !this.speaking && this.candidateSince === null;
    // Erst eine zusammenhängende Ruhephase bestätigt späteren Pegel als Sprache.
    if (quiet) {
      this.quietSince ??= at;
      if (at - this.quietSince >= 300) this.sawQuiet = true;
    } else {
      this.quietSince = null;
    }
    const belowSpeech = this.speechLevel > 0 && level <= this.speechLevel * 0.5;
    // Erst unter der relativen Halteschwelle beginnt eine mögliche Rauschpause.
    const background = belowSpeech && (
      level <= this.speechLevel * 0.3 || this.stableSince !== null
    );
    if (aiSpeaking || (this.speaking && this.sawQuiet && !background)) {
      this.stableSince = null;
      this.noiseLevels = [];
    } else {
      this.noiseLevels = this.noiseLevels.filter((sample) => at - sample.at <= 2000);
      this.noiseLevels.push({ at, level });
      const low = Math.min(...this.noiseLevels.map((sample) => sample.level));
      const high = Math.max(...this.noiseLevels.map((sample) => sample.level));
      // Schwankendes Lüfterrauschen bleibt innerhalb eines gemeinsamen Bandes.
      if (high > Math.max(0.001, low * 2)) {
        this.noiseLevels = [{ at, level }];
        this.stableSince = at;
      } else {
        this.stableSince ??= at;
      }
    }
    const stableMs = this.stableSince === null ? 0 : at - this.stableSince;
    if (!aiSpeaking && (quiet || (background && stableMs >= 300) ||
      ((!this.speaking || !this.sawQuiet) && stableMs >= 2000))) {
      // Ohne vorherige Ruhe auch stabilen Anfangslärm lernen, sonst nur in Ruhe.
      const k = level > this.noise ? 0.02 : 0.2;
      // Die untere Hülle verhindert, dass einzelne Rauschspitzen eingelernt werden.
      const lower = Math.min(...this.noiseLevels.map((sample) => sample.level));
      const ceiling = this.speechLevel > 0 && this.sawQuiet
        ? Math.min(0.1, this.speechLevel * 0.5)
        : 0.1;
      const target = Math.min(level, lower, ceiling);
      // Eine abklingende Sprachreferenz senkt nicht den eingelernten Boden.
      if (level < this.noise || target > this.noise) {
        this.noise += (target - this.noise) * k;
      }
    }

    // Unterbrechen: nur solange die KI spricht, nur bei ununterbrochener Sprache.
    if (aiSpeaking && this.config.bargeIn && !continuation && !this.resuming) {
      if (voiced) {
        // Die Bestätigung an der Fenstergrenze ohne Zeitverlust übernehmen.
        this.bargeSince ??= pendingResumeSince ?? at;
        if (!this.bargeFired && at - this.bargeSince >= this.config.bargeInMs) {
          this.bargeFired = true;
          events.push({ kind: "barge_in", at });
        }
      } else {
        this.bargeSince = null;
        this.bargeFired = false;
      }
    } else {
      this.bargeSince = null;
      this.bargeFired = false;
    }

    if (voiced) {
      if (!continuation && this.earlyFired) {
        // Nach der festen Pufferfrist beginnt auch eine späte Ergänzung einen neuen Turn.
        this.speaking = false;
        this.earlyFired = false;
        this.speechMs = 0;
        this.candidateSince = null;
      }
      if (continuation && !this.resuming) {
        // Während der KI-Ausgabe gilt auch für Ergänzungen die Unterbrechungsdauer.
        this.resumeSince ??= at;
        const resumeMs = aiSpeaking ? this.config.bargeInMs : this.config.minSpeechMs;
        if (at - this.resumeSince < resumeMs) {
          return this.diagnose(events, at, level, threshold);
        }
        this.resumeSince = null;
        this.resuming = true;
      }
      this.lastSpeechAt = at;
      if (continuation && !this.speaking) {
        this.speaking = true;
        this.earlyFired = true;
      }
      if (this.speaking) {
        this.speechLevel = Math.max(level, this.speechLevel);
        this.speechMs += dt;
        if (this.earlyFired) {
          this.earlyFired = false;
          this.earlyAt = null;
          events.push({ kind: "resume", at });
        }
      } else {
        this.candidateSince ??= at;
        if (at - this.candidateSince >= this.config.minSpeechMs) {
          this.speaking = true;
          this.speechStart = this.candidateSince;
          this.speechMs = at - this.candidateSince;
          this.speechLevel = level;
          this.earlyAt = null;
          events.push({ kind: "speech_start", at: this.speechStart });
        }
      }
      return this.diagnose(events, at, level, threshold);
    }

    // Stille
    this.candidateSince = null;
    this.resumeSince = null;
    if (!this.speaking) {
      this.candidateSince = null;
      return this.diagnose(events, at, level, threshold);
    }
    const silence = at - this.lastSpeechAt;
    if (!this.earlyFired && silence >= Math.max(500, this.config.earlyEndMs)) {
      this.earlyFired = true;
      this.earlyAt = at;
      events.push({
        kind: "early_end",
        at,
        speechMs: this.speechMs,
        lastSpeechAt: this.lastSpeechAt,
      });
    }
    if (silence >= Math.max(1000, this.config.earlyEndMs, this.config.endMs)) {
      events.push({
        kind: "end",
        at,
        speechMs: this.speechMs,
        lastSpeechAt: this.lastSpeechAt,
      });
      this.speaking = false;
      this.candidateSince = null;
      this.earlyFired = false;
      this.resuming = false;
    }
    return this.diagnose(events, at, level, threshold);
  }
}
