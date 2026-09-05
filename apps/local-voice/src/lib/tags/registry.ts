import {
  Smile,
  Drama,
  Speech,
  Volume2,
  Wind,
  PauseCircle,
  Sparkles,
} from "lucide-react";
import type { TagCategoryDef, TagDef } from "./types";

/**
 * Kuratiert aus den offiziellen Fish-Speech-Quellen (Paket-Brief A2, keine
 * Websuche nötig): den S2-Pro-Beispieltags aus dem README (eckige Klammern,
 * versteht auch Freitext) und der S1-Festliste (runde Klammern, feste
 * Wortliste). Wo ein S1-Wort exakt (gleicher String) auch als S2-Beispiel
 * vorkommt, ist es EIN Registry-Eintrag mit gesetztem `s1`; wo die Wortform
 * abweicht (z. B. S2 "sigh" vs. S1 "sighing"), sind es bewusst zwei separate
 * Einträge — sie klingen unterschiedlich lang/intensiv und sollen beide
 * auffindbar sein, ohne dass eines das andere verdeckt.
 */
export const TAG_CATEGORIES: TagCategoryDef[] = [
  { id: "emotionBasic", icon: Smile },
  { id: "emotionAdvanced", icon: Drama },
  { id: "tone", icon: Speech },
  { id: "dynamics", icon: Volume2 },
  { id: "effects", icon: Wind },
  { id: "pauses", icon: PauseCircle },
  { id: "special", icon: Sparkles },
];

export const TAG_REGISTRY: TagDef[] = [
  // ---- Emotionen: Basis ------------------------------------------------
  {
    id: "angry",
    insert: "angry",
    category: "emotionBasic",
    label: { en: "Angry", de: "Wütend" },
    description: {
      en: "Sharp, tense delivery of clear anger.",
      de: "Scharfe, angespannte Stimme voller Wut.",
    },
    aliases: ["wut", "zornig", "mad"],
    s1: "angry",
  },
  {
    id: "excited",
    insert: "excited",
    category: "emotionBasic",
    label: { en: "Excited", de: "Aufgeregt" },
    description: {
      en: "Energetic, upbeat delivery full of anticipation.",
      de: "Energiegeladene, freudig gespannte Stimme.",
    },
    aliases: ["aufgeregt", "gespannt", "thrilled"],
    s1: "excited",
  },
  {
    id: "surprised",
    insert: "surprised",
    category: "emotionBasic",
    label: { en: "Surprised", de: "Überrascht" },
    description: {
      en: "A sudden rise in pitch, as if caught off guard.",
      de: "Plötzlicher Tonhöhenanstieg, wie unerwartet ertappt.",
    },
    aliases: ["ueberrascht", "erstaunt"],
    s1: "surprised",
  },
  {
    id: "delight",
    insert: "delight",
    category: "emotionBasic",
    label: { en: "Delight", de: "Freude" },
    description: {
      en: "Warm, bright pleasure in the voice.",
      de: "Warme, helle Freude in der Stimme.",
    },
    aliases: ["freude", "erfreut", "delighted"],
    s1: null,
  },
  {
    id: "sad",
    insert: "sad",
    category: "emotionBasic",
    label: { en: "Sad", de: "Traurig" },
    description: {
      en: "Slower, lower, heavier delivery of sadness.",
      de: "Langsamere, tiefere, schwerere traurige Stimme.",
    },
    aliases: ["traurig", "betruebt"],
    s1: "sad",
  },
  {
    id: "shocked",
    insert: "shocked",
    category: "emotionBasic",
    label: { en: "Shocked", de: "Schockiert" },
    description: {
      en: "A stunned, breath-caught reaction.",
      de: "Eine fassungslose Reaktion mit stockendem Atem.",
    },
    aliases: ["schockiert", "fassungslos"],
    s1: null,
  },
  {
    id: "satisfied",
    insert: "satisfied",
    category: "emotionBasic",
    label: { en: "Satisfied", de: "Zufrieden" },
    description: {
      en: "Content, settled tone of quiet approval.",
      de: "Zufriedener, ruhiger Ton stiller Zustimmung.",
    },
    aliases: ["zufrieden", "content"],
    s1: "satisfied",
  },
  {
    id: "delighted",
    insert: "delighted",
    category: "emotionBasic",
    label: { en: "Delighted", de: "Hocherfreut" },
    description: {
      en: "Bright, wholehearted pleasure.",
      de: "Helle, uneingeschränkte Freude.",
    },
    aliases: ["hocherfreut", "begeistert"],
    s1: "delighted",
  },
  {
    id: "scared",
    insert: "scared",
    category: "emotionBasic",
    label: { en: "Scared", de: "Verängstigt" },
    description: {
      en: "Trembling, tense voice of fear.",
      de: "Zitternde, angespannte Stimme voller Angst.",
    },
    aliases: ["angst", "veraengstigt", "afraid"],
    s1: "scared",
  },
  {
    id: "worried",
    insert: "worried",
    category: "emotionBasic",
    label: { en: "Worried", de: "Besorgt" },
    description: {
      en: "Tense, uneasy tone of concern.",
      de: "Angespannter, unruhiger Ton der Sorge.",
    },
    aliases: ["besorgt", "concerned"],
    s1: "worried",
  },
  {
    id: "upset",
    insert: "upset",
    category: "emotionBasic",
    label: { en: "Upset", de: "Aufgebracht" },
    description: {
      en: "Agitated, hurt undertone.",
      de: "Aufgewühlter, verletzter Unterton.",
    },
    aliases: ["aufgebracht", "verstimmt"],
    s1: "upset",
  },
  {
    id: "nervous",
    insert: "nervous",
    category: "emotionBasic",
    label: { en: "Nervous", de: "Nervös" },
    description: {
      en: "Jittery, hesitant delivery.",
      de: "Zittrige, zögerliche Sprechweise.",
    },
    aliases: ["nervoes", "jittery"],
    s1: "nervous",
  },
  {
    id: "frustrated",
    insert: "frustrated",
    category: "emotionBasic",
    label: { en: "Frustrated", de: "Frustriert" },
    description: {
      en: "Tight, exasperated tone.",
      de: "Angespannter, genervter Ton.",
    },
    aliases: ["frustriert", "genervt"],
    s1: "frustrated",
  },
  {
    id: "depressed",
    insert: "depressed",
    category: "emotionBasic",
    label: { en: "Depressed", de: "Niedergeschlagen" },
    description: {
      en: "Flat, heavy, low-energy voice.",
      de: "Flache, schwere, energielose Stimme.",
    },
    aliases: ["niedergeschlagen", "deprimiert"],
    s1: "depressed",
  },
  {
    id: "empathetic",
    insert: "empathetic",
    category: "emotionBasic",
    label: { en: "Empathetic", de: "Einfühlsam" },
    description: {
      en: "Warm, understanding tone.",
      de: "Warmer, verständnisvoller Ton.",
    },
    aliases: ["einfuehlsam", "mitfuehlend"],
    s1: "empathetic",
  },
  {
    id: "embarrassed",
    insert: "embarrassed",
    category: "emotionBasic",
    label: { en: "Embarrassed", de: "Verlegen" },
    description: {
      en: "Halting, self-conscious delivery.",
      de: "Stockende, verlegene Sprechweise.",
    },
    aliases: ["verlegen", "peinlich beruehrt"],
    s1: "embarrassed",
  },
  {
    id: "disgusted",
    insert: "disgusted",
    category: "emotionBasic",
    label: { en: "Disgusted", de: "Angewidert" },
    description: {
      en: "Recoiling tone of revulsion.",
      de: "Zurückweichender Ton des Abscheus.",
    },
    aliases: ["angewidert", "angeekelt"],
    s1: "disgusted",
  },
  {
    id: "moved",
    insert: "moved",
    category: "emotionBasic",
    label: { en: "Moved", de: "Gerührt" },
    description: {
      en: "Soft, touched tone, close to tears.",
      de: "Weicher, gerührter Ton, den Tränen nah.",
    },
    aliases: ["geruehrt", "bewegt"],
    s1: "moved",
  },
  {
    id: "proud",
    insert: "proud",
    category: "emotionBasic",
    label: { en: "Proud", de: "Stolz" },
    description: {
      en: "Confident, chest-out tone of pride.",
      de: "Selbstbewusster, stolzer Ton.",
    },
    aliases: ["stolz"],
    s1: "proud",
  },
  {
    id: "relaxed",
    insert: "relaxed",
    category: "emotionBasic",
    label: { en: "Relaxed", de: "Entspannt" },
    description: {
      en: "Loose, unhurried, calm delivery.",
      de: "Lockere, entspannte, gelassene Sprechweise.",
    },
    aliases: ["entspannt", "gelassen", "calm"],
    s1: "relaxed",
  },
  {
    id: "grateful",
    insert: "grateful",
    category: "emotionBasic",
    label: { en: "Grateful", de: "Dankbar" },
    description: {
      en: "Warm, sincere tone of thanks.",
      de: "Warmer, aufrichtiger Ton der Dankbarkeit.",
    },
    aliases: ["dankbar", "thankful"],
    s1: "grateful",
  },
  {
    id: "confident",
    insert: "confident",
    category: "emotionBasic",
    label: { en: "Confident", de: "Selbstsicher" },
    description: {
      en: "Steady, assured delivery.",
      de: "Ruhige, sichere Sprechweise.",
    },
    aliases: ["selbstsicher", "selbstbewusst"],
    s1: "confident",
  },
  {
    id: "interested",
    insert: "interested",
    category: "emotionBasic",
    label: { en: "Interested", de: "Interessiert" },
    description: {
      en: "Engaged, curious lift in the voice.",
      de: "Aufmerksamer, neugieriger Ton.",
    },
    aliases: ["interessiert"],
    s1: "interested",
  },
  {
    id: "curious",
    insert: "curious",
    category: "emotionBasic",
    label: { en: "Curious", de: "Neugierig" },
    description: {
      en: "Inquisitive, probing tone.",
      de: "Fragender, neugieriger Ton.",
    },
    aliases: ["neugierig"],
    s1: "curious",
  },
  {
    id: "confused",
    insert: "confused",
    category: "emotionBasic",
    label: { en: "Confused", de: "Verwirrt" },
    description: {
      en: "Uncertain, searching delivery.",
      de: "Unsichere, suchende Sprechweise.",
    },
    aliases: ["verwirrt"],
    s1: "confused",
  },
  {
    id: "joyful",
    insert: "joyful",
    category: "emotionBasic",
    label: { en: "Joyful", de: "Freudig" },
    description: {
      en: "Light, buoyant happiness.",
      de: "Leichte, beschwingte Freude.",
    },
    aliases: ["freudig", "froehlich"],
    s1: "joyful",
  },

  // ---- Emotionen: Erweitert ---------------------------------------------
  {
    id: "disdainful",
    insert: "disdainful",
    category: "emotionAdvanced",
    label: { en: "Disdainful", de: "Verächtlich" },
    description: {
      en: "Cold, dismissive contempt.",
      de: "Kalte, abwertende Verachtung.",
    },
    aliases: ["veraechtlich", "contemptuous"],
    s1: "disdainful",
  },
  {
    id: "unhappy",
    insert: "unhappy",
    category: "emotionAdvanced",
    label: { en: "Unhappy", de: "Unglücklich" },
    description: {
      en: "Downcast, dissatisfied tone.",
      de: "Bedrückter, unzufriedener Ton.",
    },
    aliases: ["ungluecklich"],
    s1: "unhappy",
  },
  {
    id: "anxious",
    insert: "anxious",
    category: "emotionAdvanced",
    label: { en: "Anxious", de: "Ängstlich" },
    description: {
      en: "Tense, restless unease.",
      de: "Angespannte, ruhelose Unruhe.",
    },
    aliases: ["aengstlich", "unruhig"],
    s1: "anxious",
  },
  {
    id: "hysterical",
    insert: "hysterical",
    category: "emotionAdvanced",
    label: { en: "Hysterical", de: "Hysterisch" },
    description: {
      en: "Loud, uncontrolled emotional outburst.",
      de: "Lauter, unkontrollierter Gefühlsausbruch.",
    },
    aliases: ["hysterisch", "ausser sich"],
    s1: "hysterical",
  },
  {
    id: "indifferent",
    insert: "indifferent",
    category: "emotionAdvanced",
    label: { en: "Indifferent", de: "Gleichgültig" },
    description: {
      en: "Flat, uninvolved tone.",
      de: "Flacher, unbeteiligter Ton.",
    },
    aliases: ["gleichgueltig"],
    s1: "indifferent",
  },
  {
    id: "impatient",
    insert: "impatient",
    category: "emotionAdvanced",
    label: { en: "Impatient", de: "Ungeduldig" },
    description: {
      en: "Clipped, hurried irritation.",
      de: "Knappe, hastige Ungeduld.",
    },
    aliases: ["ungeduldig"],
    s1: "impatient",
  },
  {
    id: "guilty",
    insert: "guilty",
    category: "emotionAdvanced",
    label: { en: "Guilty", de: "Schuldbewusst" },
    description: {
      en: "Hesitant, apologetic undertone.",
      de: "Zögerlicher, entschuldigender Unterton.",
    },
    aliases: ["schuldbewusst"],
    s1: "guilty",
  },
  {
    id: "scornful",
    insert: "scornful",
    category: "emotionAdvanced",
    label: { en: "Scornful", de: "Höhnisch" },
    description: {
      en: "Mocking contempt.",
      de: "Spöttische Verachtung.",
    },
    aliases: ["hoehnisch"],
    s1: "scornful",
  },
  {
    id: "panicked",
    insert: "panicked",
    category: "emotionAdvanced",
    label: { en: "Panicked", de: "Panisch" },
    description: {
      en: "Frantic, breathless urgency.",
      de: "Hektische, atemlose Dringlichkeit.",
    },
    aliases: ["panisch"],
    s1: "panicked",
  },
  {
    id: "furious",
    insert: "furious",
    category: "emotionAdvanced",
    label: { en: "Furious", de: "Rasend" },
    description: {
      en: "Intense, barely controlled rage.",
      de: "Intensive, kaum kontrollierte Wut.",
    },
    aliases: ["rasend", "wutentbrannt"],
    s1: "furious",
  },
  {
    id: "reluctant",
    insert: "reluctant",
    category: "emotionAdvanced",
    label: { en: "Reluctant", de: "Widerwillig" },
    description: {
      en: "Dragging, unwilling delivery.",
      de: "Schleppende, widerwillige Sprechweise.",
    },
    aliases: ["widerwillig"],
    s1: "reluctant",
  },
  {
    id: "keen",
    insert: "keen",
    category: "emotionAdvanced",
    label: { en: "Keen", de: "Eifrig" },
    description: {
      en: "Eager, enthusiastic readiness.",
      de: "Eifrige, begeisterte Bereitschaft.",
    },
    aliases: ["eifrig", "eager"],
    s1: "keen",
  },
  {
    id: "disapproving",
    insert: "disapproving",
    category: "emotionAdvanced",
    label: { en: "Disapproving", de: "Missbilligend" },
    description: {
      en: "Cool, judging tone.",
      de: "Kühler, bewertender Ton.",
    },
    aliases: ["missbilligend"],
    s1: "disapproving",
  },
  {
    id: "negative",
    insert: "negative",
    category: "emotionAdvanced",
    label: { en: "Negative", de: "Negativ" },
    description: {
      en: "Generally downbeat, pessimistic tone.",
      de: "Allgemein niedergeschlagener, pessimistischer Ton.",
    },
    aliases: ["negativ"],
    s1: "negative",
  },
  {
    id: "denying",
    insert: "denying",
    category: "emotionAdvanced",
    label: { en: "Denying", de: "Verneinend" },
    description: {
      en: "Firm rejection in the voice.",
      de: "Bestimmte Ablehnung in der Stimme.",
    },
    aliases: ["verneinend"],
    s1: "denying",
  },
  {
    id: "astonished",
    insert: "astonished",
    category: "emotionAdvanced",
    label: { en: "Astonished", de: "Verblüfft" },
    description: {
      en: "Wide-eyed amazement.",
      de: "Staunende Verblüffung.",
    },
    aliases: ["verbluefft"],
    s1: "astonished",
  },
  {
    id: "serious",
    insert: "serious",
    category: "emotionAdvanced",
    label: { en: "Serious", de: "Ernst" },
    description: {
      en: "Grave, measured delivery.",
      de: "Ernste, gemessene Sprechweise.",
    },
    aliases: ["ernst"],
    s1: "serious",
  },
  {
    id: "sarcastic",
    insert: "sarcastic",
    category: "emotionAdvanced",
    label: { en: "Sarcastic", de: "Sarkastisch" },
    description: {
      en: "Dry, pointed irony.",
      de: "Trockene, spitze Ironie.",
    },
    aliases: ["sarkastisch"],
    s1: "sarcastic",
  },
  {
    id: "conciliative",
    insert: "conciliative",
    category: "emotionAdvanced",
    label: { en: "Conciliative", de: "Versöhnlich" },
    description: {
      en: "Soft, appeasing tone.",
      de: "Weicher, versöhnlicher Ton.",
    },
    aliases: ["versoehnlich", "conciliatory"],
    s1: "conciliative",
  },
  {
    id: "comforting",
    insert: "comforting",
    category: "emotionAdvanced",
    label: { en: "Comforting", de: "Tröstend" },
    description: {
      en: "Warm, reassuring tone.",
      de: "Warmer, beruhigender Ton.",
    },
    aliases: ["troestend"],
    s1: "comforting",
  },
  {
    id: "sincere",
    insert: "sincere",
    category: "emotionAdvanced",
    label: { en: "Sincere", de: "Aufrichtig" },
    description: {
      en: "Plain, heartfelt honesty.",
      de: "Schlichte, aufrichtige Ehrlichkeit.",
    },
    aliases: ["aufrichtig"],
    s1: "sincere",
  },
  {
    id: "sneering",
    insert: "sneering",
    category: "emotionAdvanced",
    label: { en: "Sneering", de: "Spöttisch" },
    description: {
      en: "Curled-lip mockery.",
      de: "Spöttischer Hohn.",
    },
    aliases: ["spoettisch"],
    s1: "sneering",
  },
  {
    id: "hesitating",
    insert: "hesitating",
    category: "emotionAdvanced",
    label: { en: "Hesitating", de: "Zögernd" },
    description: {
      en: "Halting, unsure delivery.",
      de: "Stockende, unsichere Sprechweise.",
    },
    aliases: ["zoegernd"],
    s1: "hesitating",
  },
  {
    id: "yielding",
    insert: "yielding",
    category: "emotionAdvanced",
    label: { en: "Yielding", de: "Nachgebend" },
    description: {
      en: "Soft, conceding tone.",
      de: "Weicher, nachgebender Ton.",
    },
    aliases: ["nachgebend"],
    s1: "yielding",
  },
  {
    id: "painful",
    insert: "painful",
    category: "emotionAdvanced",
    label: { en: "Painful", de: "Schmerzerfüllt" },
    description: {
      en: "Strained voice carrying pain.",
      de: "Angespannte, schmerzerfüllte Stimme.",
    },
    aliases: ["schmerzerfuellt"],
    s1: "painful",
  },
  {
    id: "awkward",
    insert: "awkward",
    category: "emotionAdvanced",
    label: { en: "Awkward", de: "Unbeholfen" },
    description: {
      en: "Stumbling, self-conscious delivery.",
      de: "Stolpernde, unbeholfene Sprechweise.",
    },
    aliases: ["unbeholfen"],
    s1: "awkward",
  },
  {
    id: "amused",
    insert: "amused",
    category: "emotionAdvanced",
    label: { en: "Amused", de: "Amüsiert" },
    description: {
      en: "Light, entertained tone.",
      de: "Leichter, amüsierter Ton.",
    },
    aliases: ["amuesiert"],
    s1: "amused",
  },

  // ---- Tonfall ------------------------------------------------------------
  {
    id: "laughing-tone",
    insert: "laughing tone",
    category: "tone",
    label: { en: "Laughing tone", de: "Lachender Tonfall" },
    description: {
      en: "Speaks with an audible smile, on the edge of laughter.",
      de: "Spricht mit hörbarem Lächeln, nah am Lachen.",
    },
    aliases: ["lachend"],
    s1: null,
  },
  {
    id: "excited-tone",
    insert: "excited tone",
    category: "tone",
    label: { en: "Excited tone", de: "Aufgeregter Tonfall" },
    description: {
      en: "Speaks quickly with rising energy.",
      de: "Spricht schnell mit steigender Energie.",
    },
    aliases: ["aufgeregter tonfall"],
    s1: null,
  },
  {
    id: "low-voice",
    insert: "low voice",
    category: "tone",
    label: { en: "Low voice", de: "Tiefe Stimme" },
    description: {
      en: "Speaks in a lowered, intimate register.",
      de: "Spricht in gesenktem, vertraulichem Register.",
    },
    aliases: ["tiefe stimme"],
    s1: null,
  },
  {
    id: "whisper",
    insert: "whisper",
    category: "tone",
    label: { en: "Whisper", de: "Flüstern" },
    description: {
      en: "Breathy, near-silent delivery.",
      de: "Hauchige, fast lautlose Sprechweise.",
    },
    aliases: ["fluestern", "leise"],
    s1: null,
  },
  {
    id: "screaming",
    insert: "screaming",
    category: "tone",
    label: { en: "Screaming", de: "Schreiend" },
    description: {
      en: "Full-throated, uncontrolled loudness.",
      de: "Voll aufgedrehte, unkontrollierte Lautstärke.",
    },
    aliases: ["schreiend"],
    s1: "screaming",
  },
  {
    id: "shouting",
    insert: "shouting",
    category: "tone",
    label: { en: "Shouting", de: "Rufend" },
    description: {
      en: "Raised, forceful volume, still controlled.",
      de: "Erhobene, kräftige Lautstärke, noch kontrolliert.",
    },
    aliases: ["rufend", "laut rufen"],
    s1: "shouting",
  },
  {
    id: "in-a-hurry-tone",
    insert: "in a hurry tone",
    category: "tone",
    label: { en: "In a hurry tone", de: "Gehetzter Tonfall" },
    description: {
      en: "Fast, pressured delivery as if short on time.",
      de: "Schnelle, gehetzte Sprechweise, als bliebe wenig Zeit.",
    },
    aliases: ["gehetzt", "in eile"],
    s1: "in a hurry tone",
  },
  {
    id: "whispering",
    insert: "whispering",
    category: "tone",
    label: { en: "Whispering", de: "Flüsternd" },
    description: {
      en: "Sustained hushed, breathy delivery.",
      de: "Anhaltend gedämpfte, hauchige Sprechweise.",
    },
    aliases: ["fluesternd"],
    s1: "whispering",
  },
  {
    id: "soft-tone",
    insert: "soft tone",
    category: "tone",
    label: { en: "Soft tone", de: "Sanfter Tonfall" },
    description: {
      en: "Gentle, muted delivery.",
      de: "Sanfte, gedämpfte Sprechweise.",
    },
    aliases: ["sanft"],
    s1: "soft tone",
  },

  // ---- Dynamik ------------------------------------------------------------
  {
    id: "volume-up",
    insert: "volume up",
    category: "dynamics",
    label: { en: "Volume up", de: "Lauter werdend" },
    description: {
      en: "Gradually increases in loudness.",
      de: "Wird allmählich lauter.",
    },
    aliases: ["lauter"],
    s1: null,
  },
  {
    id: "low-volume",
    insert: "low volume",
    category: "dynamics",
    label: { en: "Low volume", de: "Leise" },
    description: {
      en: "Quiet, restrained loudness throughout.",
      de: "Durchgehend leise, zurückhaltende Lautstärke.",
    },
    aliases: ["leise"],
    s1: null,
  },
  {
    id: "loud",
    insert: "loud",
    category: "dynamics",
    label: { en: "Loud", de: "Laut" },
    description: {
      en: "Consistently high volume.",
      de: "Durchgehend hohe Lautstärke.",
    },
    aliases: ["laut"],
    s1: null,
  },
  {
    id: "volume-down",
    insert: "volume down",
    category: "dynamics",
    label: { en: "Volume down", de: "Leiser werdend" },
    description: {
      en: "Gradually decreases in loudness.",
      de: "Wird allmählich leiser.",
    },
    aliases: ["leiser"],
    s1: null,
  },
  {
    id: "speaking-slowly",
    insert: "speaking slowly",
    category: "dynamics",
    label: { en: "Speaking slowly", de: "Langsam sprechend" },
    description: {
      en: "Deliberate, unhurried pace.",
      de: "Bedächtiges, ungehetztes Sprechtempo.",
    },
    aliases: ["langsam"],
    s1: null,
  },
  {
    id: "speaking-rapidly",
    insert: "speaking rapidly",
    category: "dynamics",
    label: { en: "Speaking rapidly", de: "Schnell sprechend" },
    description: {
      en: "Fast, tumbling pace.",
      de: "Schnelles, überstürztes Sprechtempo.",
    },
    aliases: ["schnell"],
    s1: null,
  },

  // ---- Effekte --------------------------------------------------------
  {
    id: "laughing",
    insert: "laughing",
    category: "effects",
    label: { en: "Laughing", de: "Lachen" },
    description: {
      en: "Audible laughter within the speech.",
      de: "Hörbares Lachen im Sprechfluss.",
    },
    aliases: ["lachen"],
    s1: "laughing",
  },
  {
    id: "inhale",
    insert: "inhale",
    category: "effects",
    label: { en: "Inhale", de: "Einatmen" },
    description: {
      en: "An audible in-breath.",
      de: "Ein hörbares Einatmen.",
    },
    aliases: ["einatmen"],
    s1: null,
  },
  {
    id: "chuckle",
    insert: "chuckle",
    category: "effects",
    label: { en: "Chuckle", de: "Kichern" },
    description: {
      en: "A single, brief laugh.",
      de: "Ein kurzes, leises Auflachen.",
    },
    aliases: ["kichern"],
    s1: null,
  },
  {
    id: "tsk",
    insert: "tsk",
    category: "effects",
    label: { en: "Tsk", de: "Zungenschnalzen" },
    description: {
      en: "A clicked-tongue sound of disapproval.",
      de: "Ein schnalzender Zungenlaut des Missfallens.",
    },
    aliases: ["zungenschnalzen"],
    s1: null,
  },
  {
    id: "interrupting",
    insert: "interrupting",
    category: "effects",
    label: { en: "Interrupting", de: "Unterbrechend" },
    description: {
      en: "Cuts in abruptly, as if breaking into speech.",
      de: "Bricht abrupt ein, wie ein Dazwischenreden.",
    },
    aliases: ["unterbrechend"],
    s1: null,
  },
  {
    id: "chuckling",
    insert: "chuckling",
    category: "effects",
    label: { en: "Chuckling", de: "Kichernd" },
    description: {
      en: "Sustained, quiet laughter.",
      de: "Anhaltendes, leises Lachen.",
    },
    aliases: ["kichernd"],
    s1: "chuckling",
  },
  {
    id: "sigh",
    insert: "sigh",
    category: "effects",
    label: { en: "Sigh", de: "Seufzer" },
    description: {
      en: "A single audible sigh.",
      de: "Ein einzelner hörbarer Seufzer.",
    },
    aliases: ["seufzer"],
    s1: null,
  },
  {
    id: "exhale",
    insert: "exhale",
    category: "effects",
    label: { en: "Exhale", de: "Ausatmen" },
    description: {
      en: "An audible out-breath.",
      de: "Ein hörbares Ausatmen.",
    },
    aliases: ["ausatmen"],
    s1: null,
  },
  {
    id: "panting",
    insert: "panting",
    category: "effects",
    label: { en: "Panting", de: "Keuchen" },
    description: {
      en: "Quick, heavy breathing.",
      de: "Schnelles, schweres Atmen.",
    },
    aliases: ["keuchen"],
    s1: "panting",
  },
  {
    id: "audience-laughter",
    insert: "audience laughter",
    category: "effects",
    label: { en: "Audience laughter", de: "Publikumsgelächter" },
    description: {
      en: "Laughter from an audience in the background.",
      de: "Gelächter eines Publikums im Hintergrund.",
    },
    aliases: ["publikumsgelaechter"],
    s1: null,
  },
  {
    id: "clearing-throat",
    insert: "clearing throat",
    category: "effects",
    label: { en: "Clearing throat", de: "Räuspern" },
    description: {
      en: "An audible throat-clear.",
      de: "Ein hörbares Räuspern.",
    },
    aliases: ["raeuspern"],
    s1: null,
  },
  {
    id: "moaning",
    insert: "moaning",
    category: "effects",
    label: { en: "Moaning", de: "Stöhnen" },
    description: {
      en: "A low, drawn-out moan.",
      de: "Ein tiefes, gezogenes Stöhnen.",
    },
    aliases: ["stoehnen"],
    s1: null,
  },
  {
    id: "sobbing",
    insert: "sobbing",
    category: "effects",
    label: { en: "Sobbing", de: "Schluchzen" },
    description: {
      en: "Broken, tearful crying.",
      de: "Gebrochenes, tränenreiches Weinen.",
    },
    aliases: ["schluchzen"],
    s1: "sobbing",
  },
  {
    id: "crying-loudly",
    insert: "crying loudly",
    category: "effects",
    label: { en: "Crying loudly", de: "Laut weinend" },
    description: {
      en: "Loud, unrestrained crying.",
      de: "Lautes, ungehemmtes Weinen.",
    },
    aliases: ["laut weinend"],
    s1: "crying loudly",
  },
  {
    id: "sighing",
    insert: "sighing",
    category: "effects",
    label: { en: "Sighing", de: "Seufzend" },
    description: {
      en: "Repeated or sustained sighing.",
      de: "Wiederholtes oder anhaltendes Seufzen.",
    },
    aliases: ["seufzend"],
    s1: "sighing",
  },
  {
    id: "groaning",
    insert: "groaning",
    category: "effects",
    label: { en: "Groaning", de: "Ächzen" },
    description: {
      en: "A low groan of discomfort.",
      de: "Ein tiefes Ächzen des Unbehagens.",
    },
    aliases: ["aechzen"],
    s1: "groaning",
  },
  {
    id: "crowd-laughing",
    insert: "crowd laughing",
    category: "effects",
    label: { en: "Crowd laughing", de: "Menge lacht" },
    description: {
      en: "Laughter from a crowd.",
      de: "Lachen einer größeren Menge.",
    },
    aliases: ["menge lacht"],
    s1: "crowd laughing",
  },
  {
    id: "background-laughter",
    insert: "background laughter",
    category: "effects",
    label: { en: "Background laughter", de: "Lachen im Hintergrund" },
    description: {
      en: "Faint laughter in the background.",
      de: "Leises Lachen im Hintergrund.",
    },
    aliases: ["lachen im hintergrund"],
    s1: "background laughter",
  },
  {
    id: "audience-laughing",
    insert: "audience laughing",
    category: "effects",
    label: { en: "Audience laughing", de: "Publikum lacht" },
    description: {
      en: "An audience laughing in real time.",
      de: "Ein Publikum lacht in Echtzeit mit.",
    },
    aliases: ["publikum lacht"],
    s1: "audience laughing",
  },

  // ---- Pausen -------------------------------------------------------------
  {
    id: "pause",
    insert: "pause",
    category: "pauses",
    label: { en: "Pause", de: "Pause" },
    description: {
      en: "A natural pause in speech — the app inserts these 0.5 s of silence itself, so it always works.",
      de: "Eine natürliche Sprechpause — die App fügt diese 0,5 Sekunden Stille selbst ein, die Pause wirkt also immer.",
    },
    s1: null,
  },
  {
    id: "short-pause",
    insert: "short pause",
    category: "pauses",
    label: { en: "Short pause", de: "Kurze Pause" },
    description: {
      en: "A brief beat of silence — the app inserts these 0.25 s itself, so it always works.",
      de: "Ein kurzer Moment Stille — die App fügt diese 0,25 Sekunden selbst ein, die Pause wirkt also immer.",
    },
    aliases: ["kurze pause"],
    s1: null,
  },
  {
    id: "long-pause",
    insert: "long pause",
    category: "pauses",
    label: { en: "Long pause", de: "Lange Pause" },
    description: {
      en: "An extended silence for emphasis — the app inserts this full second itself, so it works even though the model does not know this tag.",
      de: "Eine längere Stille zur Betonung — die App fügt diese ganze Sekunde selbst ein, die Pause wirkt also auch ohne dass das Modell das Tag kennt.",
    },
    aliases: ["lange pause"],
    s1: null,
  },
  {
    id: "break",
    insert: "break",
    category: "pauses",
    label: { en: "Break", de: "Sprechpause" },
    description: {
      en: "A clean break between phrases — the app inserts these 0.7 s of silence itself, so it works even though the model does not know this tag.",
      de: "Ein klarer Einschnitt zwischen Phrasen — die App fügt diese 0,7 Sekunden Stille selbst ein, die Pause wirkt also auch ohne dass das Modell das Tag kennt.",
    },
    aliases: ["einschnitt"],
    s1: null,
  },

  // ---- Spezial --------------------------------------------------------
  {
    id: "emphasis",
    insert: "emphasis",
    category: "special",
    label: { en: "Emphasis", de: "Betonung" },
    description: {
      en: "Stresses the following word or phrase.",
      de: "Betont das folgende Wort oder die Phrase.",
    },
    aliases: ["betonung"],
    s1: null,
  },
  {
    id: "singing",
    insert: "singing",
    category: "special",
    label: { en: "Singing", de: "Singend" },
    description: {
      en: "Switches to a sung melody.",
      de: "Wechselt zu gesungener Melodie.",
    },
    aliases: ["singend", "gesang"],
    s1: null,
  },
  {
    id: "echo",
    insert: "echo",
    category: "special",
    label: { en: "Echo", de: "Echo" },
    description: {
      en: "Adds a reverberating echo effect.",
      de: "Fügt einen nachhallenden Echo-Effekt hinzu.",
    },
    aliases: ["hall"],
    s1: null,
  },
  {
    id: "with-strong-accent",
    insert: "with strong accent",
    category: "special",
    label: { en: "With strong accent", de: "Mit starkem Akzent" },
    description: {
      en: "Speaks with a pronounced regional or foreign accent.",
      de: "Spricht mit deutlich hörbarem Akzent.",
    },
    aliases: ["akzent", "mit akzent"],
    s1: null,
  },
];

/** de -> deutsches Label, sonst englisches (Fallback fuer alle anderen 23 UI-Sprachen). */
export function localizedLabel(tag: TagDef, uiLang: string): string {
  return uiLang === "de" ? tag.label.de : tag.label.en;
}

const normalize = (value: string): string => value.trim().toLowerCase();

/**
 * Suche bleibt bewusst zweisprachig — wer auf Deutsch nach "whisper" tippt
 * (z. B. weil der Fish-Speech-Tag selbst englisch ist), soll trotzdem
 * treffen. `uiLang` entscheidet nur die REIHENFOLGE, nicht die Treffermenge:
 * ein Treffer in der aktiven UI-Sprache steht vor einem gleichwertigen
 * Treffer in der anderen Sprache.
 *
 * Fuenf Stufen, von genau zu lose (stabil sortiert innerhalb jeder Stufe —
 * die Registry-Reihenfolge bleibt erhalten, damit die Liste nicht bei jedem
 * Tastendruck neu durcheinanderspringt):
 *   0. Praefix in der aktiven Sprache (uiLang)
 *   1. Praefix in der jeweils anderen Sprache
 *   2. Substring in der aktiven Sprache
 *   3. Substring in der jeweils anderen Sprache
 *   4. Alias (Praefix oder Substring, sprachuebergreifend)
 *
 * `insert`/`id` sind der kanonische S2-Klammertext und immer englisch —
 * sie zaehlen deshalb zu den englischen Feldern, unabhaengig von `uiLang`.
 *
 * Beispiele (per manuellem tsx-Stichprobentest verifiziert, siehe
 * Fix-Bericht zu Paket A2):
 *   - searchTags("sp", "en") → "speaking-slowly"/"speaking-rapidly" (Praefix
 *     auf dem englischen Insert, Stufe 0) stehen VOR "break" (das englische
 *     Label ist "Break" — kein Praefix-Treffer; nur das deutsche Label
 *     "Sprechpause" beginnt mit "sp", Stufe 1).
 *   - searchTags("sp", "de") kehrt genau das um: "break" (deutsches Label
 *     "Sprechpause", jetzt Stufe 0) steht VOR "speaking-slowly"/
 *     "speaking-rapidly" (nur ueber das jetzt "andere" englische Insert,
 *     Stufe 1) — derselbe Suchbegriff, andere Reihenfolge, weil nur
 *     `uiLang` sich geaendert hat.
 *   - searchTags("whisper", "de") findet "whisper"/"whispering" trotzdem
 *     (ueber die englischen Felder, da kein deutsches Label mit "whisper"
 *     beginnt oder es enthaelt) — die Suche bleibt zweisprachig, `uiLang`
 *     versteckt nie einen Treffer, er sortiert ihn nur um.
 */
export function searchTags(query: string, uiLang: string): TagDef[] {
  const q = normalize(query);
  if (!q) return TAG_REGISTRY;

  type Ranked = { tag: TagDef; rank: number; index: number };
  const ranked: Ranked[] = [];

  TAG_REGISTRY.forEach((tag, index) => {
    const enFields = [tag.insert, tag.id, tag.label.en].map(normalize);
    const deFields = [tag.label.de].map(normalize);
    const activeFields = uiLang === "de" ? deFields : enFields;
    const otherFields = uiLang === "de" ? enFields : deFields;
    const aliases = (tag.aliases ?? []).map(normalize);

    let rank: number | null = null;
    if (activeFields.some((field) => field.startsWith(q))) {
      rank = 0;
    } else if (otherFields.some((field) => field.startsWith(q))) {
      rank = 1;
    } else if (activeFields.some((field) => field.includes(q))) {
      rank = 2;
    } else if (otherFields.some((field) => field.includes(q))) {
      rank = 3;
    } else if (
      aliases.some((field) => field.startsWith(q) || field.includes(q))
    ) {
      rank = 4;
    }

    if (rank !== null) ranked.push({ tag, rank, index });
  });

  ranked.sort((a, b) => a.rank - b.rank || a.index - b.index);
  return ranked.map((entry) => entry.tag);
}
