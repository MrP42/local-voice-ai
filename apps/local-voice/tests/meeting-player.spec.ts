import { test, expect } from "@playwright/test";
import {
  MEETINGS,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
const wav = (seconds: number) => {
  const rate = 8000;
  const n = rate * seconds;
  const b = Buffer.alloc(44 + n, 128);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + n, 4);
  b.write("WAVE", 8);
  b.write("fmt ", 12);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate, 28);
  b.writeUInt16LE(1, 32);
  b.writeUInt16LE(8, 34);
  b.write("data", 36);
  b.writeUInt32LE(n, 40);
  return b;
};
/** Stille WAV (8 kHz, 8 bit, mono) von 35:14. */
const AUDIO = wav(2114);
// Spur-Player im Bedienfeld (lva#293): Zeitstrahl per Tastatur. Die Attrappe
// liefert eine stille WAV-Datei mit Range-Antworten, sonst laesst Chromium
// nicht springen.
test("Zeitstrahl: Bild-Tasten 30 s, Pfeile 5 s, Pos1/Ende; Wert als Text", async ({
  page,
}) => {
  const m2 = MEETINGS.find((m) => m.id === "m2") as Record<string, unknown>;
  const before = { ...m2 };
  Object.assign(m2, {
    source: "recording",
    mic_audio_path: "/vorschau-audio/mic.wav",
    system_audio_path: "/vorschau-audio/system.wav",
  });
  try {
    await page.route("**/vorschau-audio/*", (r) => {
      const range = r.request().headers()["range"];
      if (range) {
        const m = /bytes=(\d+)-(\d*)/.exec(range)!;
        const s = +m[1];
        const e = m[2] ? +m[2] : AUDIO.length - 1;
        return r.fulfill({
          status: 206,
          body: AUDIO.subarray(s, e + 1),
          headers: {
            "Content-Type": "audio/wav",
            "Accept-Ranges": "bytes",
            "Content-Range": `bytes ${s}-${e}/${AUDIO.length}`,
          },
        });
      }
      return r.fulfill({
        body: AUDIO,
        headers: { "Content-Type": "audio/wav", "Accept-Ranges": "bytes" },
      });
    });
    await installRecMock(page);
    await openRecordings(page, 1440, 900);
    await pickMeeting(page, "m2");
    const player = page.getByTestId("rec-player");
    const slider = player.getByRole("slider");
    await expect(slider).toHaveAttribute("aria-valuetext", "0:00 von 35:14");
    await slider.focus();
    await page.keyboard.press("PageUp");
    await page.keyboard.press("ArrowRight");
    await expect(slider).toHaveAttribute("aria-valuetext", "0:35 von 35:14");
    await page.keyboard.press("ArrowLeft");
    await expect(slider).toHaveAttribute("aria-valuetext", "0:30 von 35:14");
    await page.keyboard.press("End");
    await expect(slider).toHaveAttribute("aria-valuetext", "35:14 von 35:14");
    await page.keyboard.press("Home");
    await expect(slider).toHaveAttribute("aria-valuetext", "0:00 von 35:14");
    // Alt+Pfeil gehoert dem System und springt nicht.
    await page.keyboard.press("Alt+ArrowRight");
    await expect(slider).toHaveAttribute("aria-valuetext", "0:00 von 35:14");
  } finally {
    Object.keys(m2).forEach((k) => delete m2[k]);
    Object.assign(m2, before);
  }
});
