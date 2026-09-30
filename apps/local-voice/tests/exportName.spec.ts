import { test, expect } from "@playwright/test";
import {
  audioExportName,
  exportFileName,
  splitFileNameTail,
} from "../src/lib/utils/exportName";

const AT = new Date(2026, 8, 8, 14, 5); // 08.09.2026, 14:05

test("the export carries the worksheet title and the time", () => {
  expect(exportFileName("Der Sturm", "Vorlesen", "wav", AT)).toBe(
    "Der-Sturm_2026-09-08_1405.wav",
  );
});

test("two exports of the same worksheet do not collide", () => {
  const first = exportFileName("Hörspiel", "Vorlesen", "mp3", AT);
  const later = exportFileName(
    "Hörspiel",
    "Vorlesen",
    "mp3",
    new Date(2026, 8, 8, 14, 6),
  );
  expect(first).not.toBe(later);
});

test("characters Windows refuses never reach the file name", () => {
  const name = exportFileName(
    'Kapitel 1: "Nacht" <2/3>',
    "Vorlesen",
    "wav",
    AT,
  );
  expect(name).toBe("Kapitel-1-Nacht-23_2026-09-08_1405.wav");
  expect(name).not.toMatch(/[\/:*?"<>|]/);
});

test("a worksheet without a title falls back instead of starting with the stamp", () => {
  expect(exportFileName("   ", "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
  expect(exportFileName(null, "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
});

test("a title of nothing but forbidden characters still yields a name", () => {
  expect(exportFileName("***", "Vorlesen", "wav", AT)).toBe(
    "Vorlesen_2026-09-08_1405.wav",
  );
});

// --- Audio-Standardname: Stimme statt Seitentitel -------------------------

const AUDIO_AT = new Date(2026, 8, 29, 17, 36); // 29.09.2026, 17:36

test("a single voice names the audio file", () => {
  expect(
    audioExportName({
      voice: "Patrick",
      fallback: "Vorlesen",
      ext: "wav",
      now: AUDIO_AT,
    }),
  ).toBe("Patrick_2026-09-29_1736.wav");
});

test("script voices, translation and summary get their suffix", () => {
  const base = { fallback: "Vorlesen", ext: "mp3", now: AUDIO_AT };
  expect(audioExportName({ ...base, voice: "Skript" })).toBe(
    "Skript_2026-09-29_1736.mp3",
  );
  expect(audioExportName({ ...base, voice: "Patrick", suffix: "EN" })).toBe(
    "Patrick-EN_2026-09-29_1736.mp3",
  );
  expect(
    audioExportName({ ...base, voice: "Skript", suffix: "Zusammenfassung" }),
  ).toBe("Skript-Zusammenfassung_2026-09-29_1736.mp3");
});

test("a long voice is shortened, the suffix stays whole", () => {
  const name = audioExportName({
    voice: "Erzählerin mit sehr langem Anzeigenamen",
    suffix: "Zusammenfassung",
    fallback: "Vorlesen",
    ext: "wav",
    now: AUDIO_AT,
  });
  const stem = name.replace(/_2026-09-29_1736\.wav$/, "");
  expect(stem.length).toBeLessThanOrEqual(24);
  expect(stem.endsWith("-Zusammenfassung")).toBe(true);
  expect(stem.startsWith("Erzähl")).toBe(true);
  // Ohne Zusatz bekommt die Stimme den ganzen Stamm.
  const alone = audioExportName({
    voice: "Erzählerin mit sehr langem Anzeigenamen",
    fallback: "Vorlesen",
    ext: "wav",
    now: AUDIO_AT,
  });
  expect(alone.replace(/_2026-09-29_1736\.wav$/, "").length).toBe(24);
});

test("audio names never carry characters Windows refuses", () => {
  const name = audioExportName({
    voice: 'Leo: "Maus" <1/2> a\\b',
    suffix: "E*N?",
    fallback: "Vorlesen",
    ext: "wav",
    now: AUDIO_AT,
  });
  expect(name).toBe("Leo-Maus-12-ab-EN_2026-09-29_1736.wav");
  expect(name).not.toMatch(/[\\/:*?"<>|]/);
});

test("an empty voice falls back", () => {
  for (const voice of ["", "   ", null, undefined, "***"]) {
    expect(
      audioExportName({
        voice,
        fallback: "Vorlesen",
        ext: "wav",
        now: AUDIO_AT,
      }),
    ).toBe("Vorlesen_2026-09-29_1736.wav");
  }
});

test("the file list keeps the stamp when it shortens a name", () => {
  expect(
    splitFileNameTail("CASE-GESPRÄCH-IE2S-·-28.09.2026_2026-09-28_1736.wav"),
  ).toEqual({
    head: "CASE-GESPRÄCH-IE2S-·-28.09.2026",
    tail: "_2026-09-28_1736.wav",
  });
  // Ohne Zeitstempel bleiben die letzten Zeichen samt Endung stehen.
  expect(splitFileNameTail("ein-sehr-langer-name-ohne-stempel.mp3")).toEqual({
    head: "ein-sehr-langer-name-ohne",
    tail: "-stempel.mp3",
  });
  // Steht der Stempel nicht am Ende, bleibt der Schwanz kurz.
  expect(
    splitFileNameTail("take_2026-09-28_1736-mit-sehr-langem-rest.wav").tail,
  ).toBe("gem-rest.wav");
});
