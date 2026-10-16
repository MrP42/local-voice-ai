import type { DocTypeInfo } from "@/bindings";

/** Schlüssel des Protokolls im Katalog (`doc_types::PROTOCOL_KEY`). */
export const PROTOCOL_KEY = "protokoll";
/** Zuletzt gewählter Typ im Reiter "Dokumente" (für alle Besprechungen gleich). */
export const DOC_TYPE_STORAGE_KEY = "lva.ui.meetings.docType";

export const readDocType = (): string => {
  try {
    return window.localStorage.getItem(DOC_TYPE_STORAGE_KEY) ?? PROTOCOL_KEY;
  } catch {
    return PROTOCOL_KEY;
  }
};

export const writeDocType = (key: string): void => {
  try {
    window.localStorage.setItem(DOC_TYPE_STORAGE_KEY, key);
  } catch {
    // Ohne Speicher gilt die Wahl nur bis zum nächsten Öffnen.
  }
};

/**
 * Die Chips der Typleiste in Katalogreihenfolge: immer das Protokoll, jeder Typ mit
 * einem Dokument zu dieser Besprechung und der gerade gewählte. Der Follow-up-Typ
 * hat kein Dokument (eigener Dialog) und erscheint nie als Chip.
 */
export const typesToShow = (
  types: DocTypeInfo[],
  kinds: string[],
  selected: string,
): DocTypeInfo[] =>
  types.filter(
    (type) =>
      type.engine !== "followup" &&
      (type.key === PROTOCOL_KEY ||
        type.key === selected ||
        kinds.includes(type.kind)),
  );
