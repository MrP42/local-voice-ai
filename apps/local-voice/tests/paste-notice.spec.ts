import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { pasteNoticeKeys } from "../src/lib/utils/pasteNotice";

// Reine Logik, kein Browser: Welche Texte der Einfuege-Hinweis zeigt.

test("a whole-dictation fallback keeps the existing wording", () => {
  expect(pasteNoticeKeys(false, true)).toEqual({
    title: "overlay.notice.title",
    action: "overlay.notice.inClipboard",
  });
  expect(pasteNoticeKeys(undefined, false)).toEqual({
    title: "overlay.notice.title",
    action: "overlay.notice.inHistory",
  });
});

test("a partial fallback never claims the full text is in the clipboard", () => {
  expect(pasteNoticeKeys(true, true)).toEqual({
    title: "overlay.notice.titlePartial",
    action: "overlay.notice.partialInClipboard",
  });
  expect(pasteNoticeKeys(true, false)).toEqual({
    title: "overlay.notice.titlePartial",
    action: "overlay.notice.partialInHistory",
  });
});

for (const lang of ["de", "en"]) {
  test(`every notice key exists in the ${lang} translation`, () => {
    const json = JSON.parse(
      readFileSync(`src/i18n/locales/${lang}/translation.json`, "utf8"),
    );
    const notice = json.overlay.notice;
    for (const partial of [false, true]) {
      for (const inClipboard of [false, true]) {
        const keys = pasteNoticeKeys(partial, inClipboard);
        for (const key of [keys.title, keys.action]) {
          const leaf = key.replace("overlay.notice.", "");
          expect(typeof notice[leaf], `${lang}: ${key}`).toBe("string");
        }
      }
    }
    for (const reason of ["live_no_target", "live_focus_changed"]) {
      expect(typeof notice.reason[reason], `${lang}: ${reason}`).toBe("string");
    }
  });
}
