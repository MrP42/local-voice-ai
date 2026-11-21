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
    this.lastFrameAt = null;
  }

  /** Aktuelle Sprechschwelle (RMS). */
  threshold(aiSpeaking: boolean): number {
    const base = Math.max(
      this.config.minLevel,
      this.noise * this.config.noiseRatio,
    );
    const active = this.speaking && this.speechLevel > 0
      ? Math.min(base, this.speechLevel * 0.3)
      : base;
    return aiSpeaking ? active * this.config.duringAiFactor : active;
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
    const events: TurnEvent[] = [];
    // Auch nach dem sicheren Ende gehört eine schnelle Ergänzung noch zur Äußerung.
    const continuation = this.earlyAt !== null && at - this.earlyAt <= 1500;
    if (!aiSpeaking) this.resuming = false;
    const threshold = this.threshold(aiSpeaking && !continuation && !this.resuming);
    const voiced = level > threshold;
    if (continuation && voiced && aiSpeaking) this.resuming = true;

    if (
      !voiced && !this.speaking && this.candidateSince === null &&
      !aiSpeaking && level < this.config.minLevel
    ) {
      // Rauschboden nur in Ruhe nachführen (langsam hoch, schnell runter).
      const k = level > this.noise ? 0.02 : 0.2;
      this.noise = Math.min(
        0.02,
        this.config.minLevel / Math.max(1, this.config.noiseRatio),
        this.noise + (level - this.noise) * k,
      );
    }

    // Unterbrechen: nur solange die KI spricht, nur bei ununterbrochener Sprache.
    if (aiSpeaking && this.config.bargeIn && !continuation && !this.resuming) {
      if (voiced) {
        this.bargeSince ??= at;
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
      this.lastSpeechAt = at;
      if (continuation && !this.speaking) {
        this.speaking = true;
        this.earlyFired = true;
      }
      if (this.speaking) {
        this.speechLevel = Math.max(level, this.speechLevel * 0.999);
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
