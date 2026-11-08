/**
 * Führung beim ersten Start im Chat (lva#394 C2). Rein: Schritte, Ziele und
 * die Anweisung für das Mitmach-Beispiel. Keine Oberfläche, kein Store.
 *
 * Jeder Schritt hat GENAU einen Text und höchstens ein Ziel; Markierung und
 * Text wechseln immer gemeinsam (Codex-Auflage 4).
 */
export type GuideStepId =
  "welcome" | "talk" | "plus" | "model" | "knowledge" | "pages" | "story";

export interface GuideStep {
  id: GuideStepId;
  /** CSS-Selektor des markierten Elements; fehlt es, steht nur der Text. */
  target?: string;
}

export const GUIDE_STEPS: readonly GuideStep[] = [
  { id: "welcome" },
  { id: "talk", target: "[data-testid='call-button']" },
  { id: "plus", target: "[data-guide='plus']" },
  { id: "model", target: "[data-testid='model-picker']" },
  { id: "knowledge", target: "[data-testid='permission-chip']" },
  { id: "pages", target: "[data-tour='nav-primary']" },
  { id: "story" },
];

/** Sprecher, die das Mitmach-Beispiel höchstens verwendet. */
export const STORY_MAX_SPEAKERS = 4;

/**
 * Anweisung an das Modell für eine Geschichte mit mehreren Sprechern. Die
 * Marker `<Name>` schaltet die Vorlese-Pipeline selbst um (speakers.rs); nur
 * Namen aus `speakers` sind erlaubt, sonst liest die Standardstimme.
 */
export function storyPrompt(opts: {
  topic: string;
  speakers: readonly string[];
  language: string;
}): string {
  const names = opts.speakers.slice(0, STORY_MAX_SPEAKERS);
  const de = opts.language.startsWith("de");
  const topic = opts.topic.trim();
  if (names.length < 2)
    return de
      ? `Erzähl mir eine kurze Geschichte (höchstens 2 Minuten Vorlesezeit). Thema: ${topic}`
      : `Tell me a short story (at most 2 minutes read aloud). Topic: ${topic}`;
  const list = names.map((n) => `<${n}>`).join(", ");
  return de
    ? [
        `Erzähl mir eine kurze Geschichte (höchstens 2 Minuten Vorlesezeit). Thema: ${topic}`,
        `Setze vor jeden Abschnitt genau einen Sprechermarker aus dieser Liste und keine anderen: ${list}.`,
        `<${names[0]}> ist der Erzähler, die übrigen sprechen die Figuren. Schreib den Marker allein an den Zeilenanfang, danach den Text.`,
      ].join("\n")
    : [
        `Tell me a short story (at most 2 minutes read aloud). Topic: ${topic}`,
        `Put exactly one speaker marker from this list before each passage and no others: ${list}.`,
        `<${names[0]}> is the narrator, the others voice the characters. Put the marker alone at the start of the line, then the text.`,
      ].join("\n");
}

/** Enthält der Text Sprechermarker aus der bekannten Liste? */
export function hasSpeakerMarkers(
  text: string,
  speakers: readonly string[],
): boolean {
  if (speakers.length === 0) return false;
  const known = new Set(speakers.map((s) => s.toLowerCase()));
  for (const m of text.matchAll(/<([^<>:\n]{1,60})>/g))
    if (known.has(m[1].trim().toLowerCase())) return true;
  return false;
}

export interface StorySegment {
  /** Sprecher aus dem Marker; `null` für Text vor dem ersten Marker. */
  speaker: string | null;
  text: string;
}

/**
 * Zerlegt eine Antwort mit Markern `<Name>` in Abschnitte. Nur bekannte Namen
 * zählen als Marker (wie die Pipeline); unbekannte bleiben Text.
 */
export function splitStory(
  text: string,
  speakers: readonly string[],
): StorySegment[] {
  const known = new Map(speakers.map((s) => [s.toLowerCase(), s]));
  const out: StorySegment[] = [];
  let current: StorySegment = { speaker: null, text: "" };
  let last = 0;
  for (const m of text.matchAll(/<([^<>:\n]{1,60})>/g)) {
    const name = known.get(m[1].trim().toLowerCase());
    if (!name) continue;
    current.text += text.slice(last, m.index);
    if (current.text.trim())
      out.push({ ...current, text: current.text.trim() });
    current = { speaker: name, text: "" };
    last = (m.index ?? 0) + m[0].length;
  }
  current.text += text.slice(last);
  if (current.text.trim()) out.push({ ...current, text: current.text.trim() });
  return out;
}
