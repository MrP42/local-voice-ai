import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

// #848: Anzeige "Meeting-Aufnahme laeuft" im Overlay-Fenster gegen eine kleine
// Tauri-Attrappe. Eine Pille ohne sichtbaren Text, je Quelle eine Pegelspur in
// eigener Farbe, Tooltip nur beim Darueberfahren, aria-label, und bei
// `prefers-reduced-motion` ein statisches Bild.

// Abnahmebilder nur auf Wunsch (wie die anderen Bildstrecken).
const SHOTS = process.env.SCREENS_DIR ?? "";

const MIC = "rgb(179, 134, 0)"; // #b38600, eigenes Mikrofon (hell)
const SYS = "rgb(15, 108, 189)"; // #0f6cbd, Systemaudio (hell)

async function install(page: Page) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    const listeners: Record<string, number[]> = {};
    let callback = 0;
    w.__listening = (event: string) => (listeners[event] ?? []).length;
    w.__emit = (event: string, payload: unknown) =>
      (listeners[event] ?? []).forEach((h) =>
        callbacks.get(h)?.({ event, id: 0, payload }),
      );
    Object.assign(window, {
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "recording_overlay" },
          currentWebview: { label: "recording_overlay" },
        },
        transformCallback: (fn: (e: unknown) => void) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (cmd: string, args: Record<string, any> = {}) => {
          switch (cmd) {
            case "get_app_settings":
              return { app_language: "de", overlay_position: "bottom" };
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:event|listen":
              (listeners[args.event as string] ??= []).push(
                args.handler as number,
              );
              return ++callback;
            default:
              return null;
          }
        },
      },
    });
  });
}

async function showMeeting(
  page: Page,
  sources: { mic: boolean; system: boolean },
) {
  await page.goto("/src/overlay/index.html");
  await page.waitForFunction(
    () =>
      (window as any).__listening("show-overlay") > 0 &&
      (window as any).__listening("meeting-event") > 0,
  );
  await page.evaluate((s) => {
    const w = window as any;
    w.__emit("meeting-indicator", s);
    w.__emit("show-overlay", "meeting");
  }, sources);
  const pill = page.getByTestId("meeting-indicator");
  await expect(pill).toBeVisible();
  return pill;
}

async function levels(page: Page, mic: number, system: number, n = 6) {
  for (let i = 0; i < n; i++) {
    await page.evaluate(
      ([m, s]) =>
        (window as any).__emit("meeting-event", {
          kind: "levels",
          mic: m,
          system: s,
        }),
      [mic, system],
    );
    await page.waitForTimeout(60);
  }
  await page.waitForTimeout(300); // Uebergang der Balken abwarten
}

const barScales = (page: Page, source: string) =>
  page
    .locator(`.smeet-track[data-source="${source}"] i`)
    .evaluateAll((bars) =>
      bars.map((b) => new DOMMatrix(getComputedStyle(b).transform).d),
    );

test.describe("Meeting-Aufnahme im Overlay (#848)", () => {
  test.use({ viewport: { width: 264, height: 88 }, colorScheme: "light" });

  test("Pille ohne sichtbaren Text, zwei Farbspuren, aria-label", async ({
    page,
  }) => {
    await install(page);
    const pill = await showMeeting(page, { mic: true, system: true });

    await expect(pill).toHaveAttribute("role", "status");
    await expect(pill).toHaveAttribute(
      "aria-label",
      "Meeting-Aufnahme läuft – eigenes Mikrofon und Systemaudio",
    );
    // Kein sichtbarer Text: der Tooltip ist ohne Hover verborgen.
    await expect(page.locator(".smeet-tip")).toBeHidden();
    expect((await pill.innerText()).trim()).toBe("");

    // Zwei Spuren in zwei Farben, je drei Balken.
    const mic = page.locator('.smeet-track[data-source="mic"] i');
    const sys = page.locator('.smeet-track[data-source="system"] i');
    await expect(mic).toHaveCount(3);
    await expect(sys).toHaveCount(3);
    await expect(mic.first()).toHaveCSS("background-color", MIC);
    await expect(sys.first()).toHaveCSS("background-color", SYS);

    // Die Pille ist klein und sitzt unten in der Mitte.
    const box = (await pill.boundingBox())!;
    expect(box.height).toBeLessThanOrEqual(28);
    expect(box.width).toBeLessThanOrEqual(72);
    expect(Math.abs(box.x + box.width / 2 - 132)).toBeLessThanOrEqual(1);
    expect(Math.abs(box.y + box.height - 88)).toBeLessThanOrEqual(1);
  });

  test("der Pegel bewegt die sprechende Quelle, die stille bleibt ruhig", async ({
    page,
  }) => {
    await install(page);
    await showMeeting(page, { mic: true, system: true });
    // Nutzerin hoert zu: Mikrofon stumm, Gegenseite spricht.
    await levels(page, 0, 0.12);

    await expect(page.locator(".smeet-track.mic")).toHaveClass(/calm/);
    await expect(page.locator(".smeet-track.system")).not.toHaveClass(/calm/);
    const mic = await barScales(page, "mic");
    const sys = await barScales(page, "system");
    expect(mic.every((s) => Math.abs(s - 0.286) < 0.01)).toBe(true);
    expect(Math.max(...sys)).toBeGreaterThan(0.6);
    if (SHOTS) {
      mkdirSync(SHOTS, { recursive: true });
      await page.screenshot({ path: `${SHOTS}/hell-beide-quellen.png` });
    }
  });

  test("nur Mikrofon: eine Spur, passender Tooltip und aria-label", async ({
    page,
  }) => {
    await install(page);
    const pill = await showMeeting(page, { mic: true, system: false });
    await expect(page.locator(".smeet-track")).toHaveCount(1);
    await expect(pill).toHaveAttribute(
      "aria-label",
      "Meeting-Aufnahme läuft – eigenes Mikrofon",
    );
    await levels(page, 0.08, 0);
    if (SHOTS)
      await page.screenshot({ path: `${SHOTS}/hell-nur-mikrofon.png` });

    await pill.hover();
    const tip = page.locator(".smeet-tip");
    await expect(tip).toBeVisible();
    await expect(tip).toHaveCSS("opacity", "1");
    await expect(tip).toContainText("Meeting-Aufnahme läuft");
    await expect(tip).toContainText("Eigenes Mikrofon");
    await expect(tip).not.toContainText("Systemaudio");
  });

  test("Tooltip beim Darueberfahren zeigt beide Quellen und passt ins Fenster", async ({
    page,
  }) => {
    await install(page);
    const pill = await showMeeting(page, { mic: true, system: true });
    await levels(page, 0.05, 0.1);
    await pill.hover();
    const tip = page.locator(".smeet-tip");
    await expect(tip).toHaveCSS("opacity", "1");
    await expect(tip).toContainText("Eigenes Mikrofon");
    await expect(tip).toContainText("Systemaudio");
    const box = (await tip.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(264);
    if (SHOTS)
      await page.screenshot({ path: `${SHOTS}/hell-hover-tooltip.png` });
  });

  test.describe("dunkel", () => {
    test.use({ colorScheme: "dark" });
    test("dunkles Thema: eigene Farben, Gelb und Blau", async ({ page }) => {
      await install(page);
      await showMeeting(page, { mic: true, system: true });
      await levels(page, 0.06, 0.12);
      await expect(
        page.locator('.smeet-track[data-source="mic"] i').first(),
      ).toHaveCSS("background-color", "rgb(255, 221, 0)");
      await expect(
        page.locator('.smeet-track[data-source="system"] i').first(),
      ).toHaveCSS("background-color", "rgb(71, 158, 245)");
      if (SHOTS) {
        await page.screenshot({ path: `${SHOTS}/dunkel-beide-quellen.png` });
        await page.getByTestId("meeting-indicator").hover();
        await expect(page.locator(".smeet-tip")).toHaveCSS("opacity", "1");
        await page.screenshot({ path: `${SHOTS}/dunkel-hover-tooltip.png` });
      }
    });
  });

  test.describe("reduzierte Bewegung", () => {
    test("statisches Bild: der Pegel bewegt keine Balken", async ({ page }) => {
      // Ueber die Seite statt `test.use`: die Kontextoption kam im Lauf gegen
      // ein fremdes Chromium nicht an (matchMedia blieb false).
      await page.emulateMedia({ reducedMotion: "reduce" });
      await install(page);
      await showMeeting(page, { mic: true, system: true });
      const before = [
        ...(await barScales(page, "mic")),
        ...(await barScales(page, "system")),
      ];
      await levels(page, 0.2, 0.2);
      const after = [
        ...(await barScales(page, "mic")),
        ...(await barScales(page, "system")),
      ];
      expect(after).toEqual(before);
      await expect(page.locator(".smeet-track i").first()).toHaveCSS(
        "transition-duration",
        "0s",
      );
    });
  });
});
