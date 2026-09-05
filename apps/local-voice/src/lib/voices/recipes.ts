/**
 * Startpunkte fuer den Stimmen-Baukasten.
 *
 * Ein Rezept ist KEINE fertige Stimme — Fish-Speech kennt keine
 * Konditionierung auf eine Beschreibung, die Stimmidentitaet haengt allein am
 * Seed. Ein Rezept fuellt deshalb vor, was die Beschreibung wirklich steuern
 * kann: den Probesatz (dessen Prosodie sich beim Klonen ueberträgt), die
 * Emotions-Tags und die Metadaten. Gewuerfelt und ausgesucht wird danach.
 *
 * `color` ist ein Palette-Key aus `registry.rs` (siehe `palette.ts`).
 */
export interface VoiceRecipe {
  id: string;
  /** Vorgeschlagener Anzeigename — frei aenderbar. */
  name: string;
  /** Geht als Beschreibung in die Stimme und steuert den Probesatz-Vorschlag. */
  description: string;
  /** Referenzsatz, hoechstens etwa 150 Zeichen, in der Rolle gesprochen. */
  probeText: string;
  tags: string[];
  color: string;
}

export const VOICE_RECIPES: VoiceRecipe[] = [
  {
    id: "pyrion",
    name: "Pyrion",
    description:
      "Sehr tiefe, erwachsene Männerstimme mit viel Resonanz und ruhiger Autorität. " +
      "Alt, mächtig und würdevoll, als hätte er Jahrhunderte erlebt. Langsames bis " +
      "mittleres Tempo, klare Artikulation, kaum Hektik. Klanglich warm und dunkel, " +
      "mit leicht rauer, steiniger Textur — nicht dämonisch, nicht monströs. Selbst " +
      "wütend bleibt die Stimme kontrolliert und schwer statt schrill. Grundstimmung: " +
      "majestätisch, ernst, geheimnisvoll, erfahren. Ein uralter Wächter oder König " +
      "aus einem Fantasyfilm — gewaltig und respekteinflößend, aber vertrauenswürdig. " +
      "Keine Karikatur, kein übertriebenes Bösewicht-Lachen, kein Brüllen.",
    probeText:
      "Ich habe Königreiche kommen und vergehen sehen. Hört mir gut zu, denn ich sage es nur einmal.",
    tags: ["slow", "serious"],
    color: "amber",
  },
];
