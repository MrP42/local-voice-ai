/**
 * Die eingebauten Assistenten-Vorlagen (nur lesbar, nie im Speicher des Nutzers). Quelle ist
 * `apps/shared/assistenten/vorlagen.json`; Vite lädt nicht aus Ordnern außerhalb von `apps/local-voice`, darum liegt der Text
 * als Zeichenkette in `vorlagenJson.ts` (erzeugt von `apps/apple/scripts/generate_assistant_templates.py`, ein Test hält beide
 * gleich). Gegenstück: `AssistantProfileTemplates.swift`.
 */
import { VORLAGEN_JSON } from "./vorlagenJson";
import {
  ordneFehler,
  parseJson,
  profilAusBaum,
  profilBaum,
  type Profil,
  type ProfilErgebnis,
} from "./profil";

function ladeVorlagen(): Profil[] {
  const baum = parseJson(VORLAGEN_JSON);
  if (baum === null || baum.art !== "liste") return [];
  const profile: Profil[] = [];
  for (const eintrag of baum.wert) {
    const ergebnis = profilAusBaum(eintrag);
    if (ergebnis.ok) profile.push(ergebnis.profil);
  }
  return profile;
}

/** Alle Vorlagen in der Reihenfolge der Datei; die erste ist `alles`. */
export const VORLAGEN: readonly Profil[] = ladeVorlagen();
/** Id der Vorlage `alles` (Standard, wenn nichts anderes gewählt ist). */
export const ALLES_ID = "alles";

export function vorlage(id: string): Profil | undefined {
  return VORLAGEN.find((v) => v.id === id);
}

export function istVorlage(id: string): boolean {
  return vorlage(id) !== undefined;
}

/** Vorbehalten für Vorlagen: ihre Ids und alles mit dem Vorsatz `vorlage-`. */
export function istVorbehalteneId(id: string): boolean {
  return istVorlage(id) || id.startsWith("vorlage-");
}

/** Eigene Kopie einer Vorlage mit neuer Id und neuem Namen; geprüft wie jede Eingabe. Eine vorbehaltene Id wird abgewiesen. */
export function alsEigenenKopieren(
  vorlageProfil: Profil,
  neueId: string,
  neuerName: string,
): ProfilErgebnis {
  const kopie = profilAusBaum(
    profilBaum({ ...vorlageProfil, id: neueId, name: neuerName }),
  );
  if (!istVorbehalteneId(neueId)) return kopie;
  const fehler = kopie.ok ? [] : kopie.fehler;
  return {
    ok: false,
    fehler: ordneFehler([...fehler, { pfad: "id", code: "wert" }]),
  };
}
