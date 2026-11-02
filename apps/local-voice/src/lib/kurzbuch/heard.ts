/**
 * Gehörte Kernaussagen eines Kurzbuchs (lva#205), je Dokument unter
 * `progress.kurzbuch:…` und damit dauerhaft im Kern gespiegelt.
 */
import { getLocal, setLocal } from "../library/progress";

const heardKey = (pageId: string, docId: string) =>
  `progress.kurzbuch:${pageId}:${docId}`;

export function loadHeard(pageId: string, docId: string): Set<string> {
  try {
    const v = JSON.parse(getLocal(heardKey(pageId, docId)) ?? "null") as {
      heard?: unknown;
    } | null;
    return new Set(
      Array.isArray(v?.heard)
        ? v.heard.filter((x) => typeof x === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

export function saveHeard(pageId: string, docId: string, heard: Set<string>) {
  setLocal(
    heardKey(pageId, docId),
    JSON.stringify({ heard: [...heard], updated: Date.now() }),
  );
}

export function removeHeard(pageId: string, docId: string) {
  setLocal(heardKey(pageId, docId), null);
}
