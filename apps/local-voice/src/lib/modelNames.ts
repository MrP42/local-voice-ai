/**
 * Anzeigenamen der Sprachmodelle: Claude-Aliase mit Version (die CLI loest
 * `sonnet` auf das neueste Modell auf — geprueft am 06.10.2026 per
 * `modelUsage`), sonst der Name mit grossem Anfang (`gpt-6.1-sol` → `GPT-6.1-Sol`).
 */
const CLAUDE: Record<string, string> = {
  fable: "Claude Fable 5.1",
  opus: "Claude Opus 5.5",
  sonnet: "Claude Sonnet 5.5",
  haiku: "Claude Haiku 4.5",
  "claude-fable-5-1": "Claude Fable 5.1",
  "claude-opus-5-5": "Claude Opus 5.5",
  "claude-sonnet-5-5": "Claude Sonnet 5.5",
  "claude-haiku-4-5-20251001": "Claude Haiku 4.5",
};

/** `modell@effort` zerlegen (so ruft die App Abo-Modelle mit Effort auf). */
export const splitModel = (id: string): [string, string | null] => {
  const at = id.indexOf("@");
  return at < 0 ? [id, null] : [id.slice(0, at), id.slice(at + 1) || null];
};

export const displayModelName = (raw: string): string => {
  const [name] = splitModel(raw.trim());
  const known = CLAUDE[name.toLowerCase()];
  if (known) return known;
  if (/^gpt-/i.test(name))
    return name
      .split("-")
      .map((p, i) =>
        i === 0 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1),
      )
      .join("-");
  return name.charAt(0).toUpperCase() + name.slice(1);
};
