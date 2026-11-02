/**
 * Sprache eines Buchtexts: Stoppwörter zählen wie
 * `managers/tts/lang.rs`, ohne Modell und ohne Netz. Für die Frage
 * „welche Stimme passt?“ reicht die Hauptsprache; bei zu wenig
 * Anhaltspunkten bleibt es bei `null` (dann wird nichts umgestellt).
 */

const LISTS: Record<string, readonly string[]> = {
  de: "der die das und ist nicht ein eine ich sie er es wir mit auf für von zu den dem des im auch sich wie aber noch nur war wird sind haben hat bei nach aus über als wenn dann oder sehr schon kann".split(" "),
  en: "the and is not a an of to it that was for on are with as his her they at be this have from or one had by but what were we when your can there you he she will would about which their just into".split(" "),
  fr: "le la les et est un une des du que qui dans pour pas sur avec ce il elle nous vous sont mais plus au".split(" "),
  es: "el la los las y es un una que de del en por para con no se su al como más pero son está muy".split(" "),
  it: "il lo la gli le e è un una che di del della per con non si sono come più ma anche nel alla".split(" "),
  // Mitbewerber, damit Niederländisch und Portugiesisch nicht als Englisch
  // oder Spanisch durchgehen.
  nl: "de het een en van ik je dat niet is op te zijn met voor hij er maar ook als bij nog wat dan wel".split(" "),
  pt: "o os as um uma que de do da dos das em no na por para com não se seu sua mais mas são está também ao".split(" "),
};

const SETS = Object.fromEntries(
  Object.entries(LISTS).map(([k, v]) => [k, new Set(v)]),
) as Record<string, Set<string>>;

/** Höchstens so viele Zeichen werden gezählt, verteilt über den Text. */
const SAMPLE = 24_000;
const MIN_WORDS = 30;

function sampleOf(text: string): string {
  if (text.length <= SAMPLE) return text;
  // Drei Stücke (Anfang, Mitte, Ende): Titelei und Impressum allein
  // entscheiden nicht über die Sprache des Buchs.
  const part = Math.floor(SAMPLE / 3);
  const mid = Math.floor(text.length / 2 - part / 2);
  return [
    text.slice(0, part),
    text.slice(mid, mid + part),
    text.slice(text.length - part),
  ].join(" ");
}

/** ISO-639-1-Kürzel der Hauptsprache oder `null`. */
export function detectLanguage(text: string): string | null {
  const sample = sampleOf(text)
    // Seitenmarken und Sprecher-Präfixe sind keine Sprache.
    .replace(/\{\{S\. [^}]*\}\}/g, " ")
    .replace(/<[^>\n]{1,40}>/g, " ");
  const scores: Record<string, number> = Object.fromEntries(
    Object.keys(LISTS).map((k) => [k, 0]),
  );
  let words = 0;
  for (const raw of sample.toLowerCase().split(/[^\p{L}\p{N}ß]+/u)) {
    if (!raw) continue;
    words += 1;
    for (const code in SETS) if (SETS[code].has(raw)) scores[code] += 2;
  }
  for (const c of sample) {
    if ("äöüÄÖÜß".includes(c)) scores.de += 1;
    else if ("ñ¿¡".includes(c)) scores.es += 2;
    else if ("ãõ".includes(c)) scores.pt += 2;
  }
  if (words < MIN_WORDS) return null;
  const ranked = Object.entries(scores).sort((a, b) => b[1] - a[1]);
  const [best, second] = ranked;
  // Eindeutig genug: deutlich vor der zweiten und nicht nur ein Zufallstreffer.
  if (best[1] < Math.max(10, words * 0.08)) return null;
  if (best[1] < second[1] * 1.5) return null;
  return best[0];
}

/** Hauptsprache eines Sprachkürzels (`de_DE`, `en-US`, `EN`) oder `null`; `und` = jede. */
export function primaryLanguage(code: string | null | undefined): string | null {
  if (!code) return null;
  const p = code.split(/[-_]/)[0]?.toLowerCase() ?? "";
  if (!p || p === "und" || p === "mul") return null;
  return p;
}

/** Sprachen, die `detectLanguage` unterscheiden kann. */
export const isKnownLanguage = (code: string): boolean => code in LISTS;
