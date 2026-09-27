/**
 * Aehnliche Tags zu einer freien Beschreibung finden -- nach Stimmung, nicht
 * nach Buchstaben. Ein fehlendes "[happy]" soll "Freudig" vorschlagen und
 * kein "Hysterisch", nur weil die Woerter aehnlich anfangen.
 *
 * Drei Stufen, stark nach schwach:
 * 1. Wortlaut: Registry-Suche (Label, Insert, Alias, beide Sprachen) und
 *    Tippfehler-Naehe.
 * 2. Stimmungsfamilie: ein kleiner deutsch-englischer Wortschatz ordnet
 *    Woerter wie "müde", "sleepy", "geheimnisvoll" einer Familie zu; deren
 *    Mitglieder sind die Kandidaten.
 * 3. Beschreibung: Wortueberschneidung mit der Tag-Beschreibung.
 * Dokumentierte Tags (offizielle Fish-Beispiele) gewinnen Gleichstaende.
 */
import { TAG_REGISTRY, searchTags } from "./registry";
import type { TagDef } from "./types";

/** Stimmungsfamilien: Registry-Ids, die dasselbe Gefuehl tragen. */
const FAMILIES: Record<string, string[]> = {
  joy: [
    "joyful",
    "delight",
    "delighted",
    "excited",
    "amused",
    "satisfied",
    "proud",
    "grateful",
    "keen",
    "excited-tone",
    "laughing-tone",
  ],
  calm: [
    "relaxed",
    "soft-tone",
    "comforting",
    "sincere",
    "conciliative",
    "empathetic",
    "moved",
    "speaking-slowly",
  ],
  sad: [
    "sad",
    "unhappy",
    "depressed",
    "moved",
    "sobbing",
    "crying-loudly",
    "sighing",
    "painful",
  ],
  fear: ["scared", "anxious", "nervous", "worried", "panicked", "hesitating"],
  anger: [
    "angry",
    "furious",
    "frustrated",
    "upset",
    "disgusted",
    "disapproving",
    "scornful",
    "sneering",
    "disdainful",
    "sarcastic",
  ],
  surprise: [
    "surprised",
    "shocked",
    "astonished",
    "confused",
    "curious",
    "interested",
  ],
  quiet: [
    "whisper",
    "whispering",
    "low-volume",
    "low-voice",
    "volume-down",
    "soft-tone",
  ],
  loud: ["loud", "shouting", "screaming", "volume-up"],
  tired: ["sighing", "sigh", "exhale", "speaking-slowly", "low-volume"],
  laugh: ["laughing", "chuckle", "chuckling", "laughing-tone", "amused"],
  breath: ["inhale", "exhale", "panting", "sigh"],
  hurry: ["in-a-hurry-tone", "speaking-rapidly", "panting", "impatient"],
  serious: ["serious", "confident", "sincere"],
  shy: ["embarrassed", "awkward", "hesitating", "reluctant", "guilty"],
  pain: ["painful", "moaning", "groaning", "crying-loudly"],
};

/** Wortanfaenge (gefaltet: ae/oe/ue/ss) -> Familien. */
const LEXICON: [string, string[]][] = [
  ["happ", ["joy"]],
  ["glueck", ["joy"]],
  ["froh", ["joy"]],
  ["froeh", ["joy"]],
  ["cheer", ["joy"]],
  ["glad", ["joy"]],
  ["merr", ["joy"]],
  ["heiter", ["joy"]],
  ["begeist", ["joy"]],
  ["enthus", ["joy"]],
  ["playful", ["joy", "laugh"]],
  ["verspielt", ["joy", "laugh"]],
  ["frech", ["joy", "laugh"]],
  ["calm", ["calm"]],
  ["ruhig", ["calm"]],
  ["gelass", ["calm"]],
  ["peace", ["calm"]],
  ["friedl", ["calm"]],
  ["gentl", ["calm", "quiet"]],
  ["sanft", ["calm", "quiet"]],
  ["tender", ["calm"]],
  ["zaertl", ["calm"]],
  ["warm", ["calm"]],
  ["lieb", ["calm"]],
  ["loving", ["calm"]],
  ["beruhig", ["calm"]],
  ["soothing", ["calm"]],
  ["tired", ["tired"]],
  ["muede", ["tired"]],
  ["sleep", ["tired"]],
  ["schlaef", ["tired"]],
  ["yawn", ["tired"]],
  ["gaehn", ["tired"]],
  ["erschoepf", ["tired"]],
  ["exhaust", ["tired"]],
  ["weary", ["tired"]],
  ["drowsy", ["tired"]],
  ["sad", ["sad"]],
  ["traurig", ["sad"]],
  ["melanch", ["sad"]],
  ["wistful", ["sad"]],
  ["wehmuet", ["sad"]],
  ["weep", ["sad"]],
  ["wein", ["sad"]],
  ["cry", ["sad"]],
  ["kummer", ["sad"]],
  ["betrueb", ["sad"]],
  ["afraid", ["fear"]],
  ["angst", ["fear"]],
  ["aengst", ["fear"]],
  ["fear", ["fear"]],
  ["furcht", ["fear"]],
  ["timid", ["fear", "shy"]],
  ["spannung", ["fear", "serious"]],
  ["suspens", ["fear", "serious"]],
  ["tense", ["fear"]],
  ["angespannt", ["fear"]],
  ["dramat", ["fear", "serious"]],
  ["unheim", ["fear", "quiet"]],
  ["creep", ["fear", "quiet"]],
  ["grusel", ["fear", "quiet"]],
  ["mad", ["anger"]],
  ["wuet", ["anger"]],
  ["zorn", ["anger"]],
  ["annoy", ["anger"]],
  ["genervt", ["anger"]],
  ["irritat", ["anger"]],
  ["aerger", ["anger"]],
  ["rage", ["anger"]],
  ["wut", ["anger"]],
  ["bitter", ["anger", "sad"]],
  ["amaz", ["surprise"]],
  ["erstaun", ["surprise"]],
  ["wow", ["surprise"]],
  ["staun", ["surprise"]],
  ["baff", ["surprise"]],
  ["ueberrasch", ["surprise"]],
  ["wonder", ["surprise"]],
  ["verwunder", ["surprise"]],
  ["mysteri", ["quiet"]],
  ["geheim", ["quiet"]],
  ["secret", ["quiet"]],
  ["whisp", ["quiet"]],
  ["fluest", ["quiet"]],
  ["leise", ["quiet"]],
  ["quiet", ["quiet"]],
  ["soft", ["quiet", "calm"]],
  ["hush", ["quiet"]],
  ["loud", ["loud"]],
  ["laut", ["loud"]],
  ["yell", ["loud"]],
  ["schrei", ["loud"]],
  ["bruell", ["loud"]],
  ["shout", ["loud"]],
  ["ruf", ["loud"]],
  ["laugh", ["laugh"]],
  ["lach", ["laugh"]],
  ["giggl", ["laugh"]],
  ["kicher", ["laugh"]],
  ["grins", ["laugh", "joy"]],
  ["breath", ["breath"]],
  ["atem", ["breath"]],
  ["atm", ["breath"]],
  ["schnauf", ["breath"]],
  ["puste", ["breath"]],
  ["hurr", ["hurry"]],
  ["eilig", ["hurry"]],
  ["hast", ["hurry"]],
  ["schnell", ["hurry"]],
  ["fast", ["hurry"]],
  ["rush", ["hurry"]],
  ["gehetzt", ["hurry"]],
  ["serious", ["serious"]],
  ["ernst", ["serious"]],
  ["stern", ["serious"]],
  ["streng", ["serious"]],
  ["feierl", ["serious"]],
  ["solemn", ["serious"]],
  ["shy", ["shy"]],
  ["schuecht", ["shy"]],
  ["verleg", ["shy"]],
  ["unsicher", ["shy", "fear"]],
  ["pain", ["pain"]],
  ["schmerz", ["pain"]],
  ["hurt", ["pain"]],
  ["weh", ["pain"]],
  ["stoehn", ["pain"]],
];

const fold = (s: string): string =>
  s
    .toLowerCase()
    .replace(/ä/g, "ae")
    .replace(/ö/g, "oe")
    .replace(/ü/g, "ue")
    .replace(/ß/g, "ss");

const tokens = (s: string): string[] =>
  fold(s)
    .split(/[^a-z0-9]+/)
    .filter((t) => t.length >= 3);

const editDistance = (a: string, b: string): number => {
  const prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    let last = prev[0];
    prev[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const tmp = prev[j];
      prev[j] = Math.min(
        prev[j] + 1,
        prev[j - 1] + 1,
        last + (a[i - 1] === b[j - 1] ? 0 : 1),
      );
      last = tmp;
    }
  }
  return prev[b.length];
};

/** Familien, die eine freie Beschreibung anspricht. */
export function moodFamilies(description: string): string[] {
  const found = new Set<string>();
  for (const token of tokens(description)) {
    for (const [stem, families] of LEXICON) {
      if (
        token.startsWith(stem) ||
        (stem.length >= 5 && token.includes(stem))
      ) {
        families.forEach((f) => found.add(f));
      }
    }
  }
  return [...found];
}

/** Die besten `max` Registry-Tags zu einer freien Beschreibung. */
export function similarTags(
  description: string,
  uiLang: string,
  max = 5,
): TagDef[] {
  const score = new Map<string, number>();
  const bump = (tag: TagDef, value: number) =>
    score.set(tag.id, Math.max(score.get(tag.id) ?? 0, 0) + value);
  const q = fold(description.trim());
  if (!q) return [];

  // 1. Wortlaut
  searchTags(description.trim(), uiLang).forEach((tag, i) =>
    bump(tag, 6 - Math.min(i, 3)),
  );
  for (const tag of TAG_REGISTRY) {
    const c = fold(tag.insert);
    if (c && editDistance(c, q) <= Math.max(1, Math.floor(q.length / 4)))
      bump(tag, 5);
  }

  // 2. Stimmungsfamilie
  const byId = new Map(TAG_REGISTRY.map((tag) => [tag.id, tag]));
  for (const family of moodFamilies(description)) {
    FAMILIES[family]?.forEach((id, rank) => {
      const tag = byId.get(id);
      // Vorne in der Familie steht das Kernwort -- leicht bevorzugt.
      if (tag) bump(tag, 3 - Math.min(rank, 4) * 0.1);
    });
  }

  // 3. Beschreibung
  const qTokens = tokens(description);
  if (qTokens.length) {
    for (const tag of TAG_REGISTRY) {
      const text = tokens(
        [
          tag.description?.en,
          tag.description?.de,
          tag.label.en,
          tag.label.de,
          ...(tag.aliases ?? []),
        ]
          .filter(Boolean)
          .join(" "),
      );
      const hits = qTokens.filter((t) =>
        text.some((w) => w.startsWith(t.slice(0, 5))),
      ).length;
      if (hits) bump(tag, hits);
    }
  }

  return [...score.entries()]
    .map(([id, value]) => ({ tag: byId.get(id) as TagDef, value }))
    .filter((entry) => entry.tag && entry.value > 0)
    .sort(
      (a, b) =>
        b.value - a.value ||
        Number(b.tag.verified === true) - Number(a.tag.verified === true),
    )
    .slice(0, max)
    .map((entry) => entry.tag);
}
