import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { countsAudio } from "../src/lib/meetingJobs";
import {
  canDetectSlides,
  marksBySegment,
  slideAt,
  slideFilePath,
  slideMarks,
  slidesErrorKey,
  visibleSlides,
} from "../src/lib/meetingSlides";
import { installLiveMock } from "./recLiveMock";
import {
  calls,
  emitMeeting,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// D4 (Goal Issues-Abschluss #70, M7 = #69): Folien aus dem Video in der
// Besprechungsmitte. Die Attrappe kennt die Befehle der Folienerkennung (D1:
// list_meeting_slides, meeting_slides_dir, set_meeting_slide_hidden,
// detect_meeting_slides); die Bilder liefert ein Routen-Handler als SVG.
//  1. Reine Logik: aktuelle Folie zur Abspielzeit, Marken, Fehlercodes.
//  2. Reiter "Folien": Leiste mit Vorschaubildern, Klick springt im Audio, die
//     aktuelle Folie folgt der Abspielzeit.
//  3. Folienmarken im Transkript.
//  4. Ausblenden und wieder Einblenden (Pflicht laut Risiko R1).
//  5. Grossansicht.
//  6. "Folien erkennen" im Menue samt Auftragsleiste und Fehlercodes.
//  7. Import mit der Option "Folien erkennen".

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../koordination/bild-video/screens",
);
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(SHOT_DIR, `${name}.png`),
    animations: "disabled",
  });
};

const VIDEO_ID = "m8";
const VIDEO_TITLE = "Vortrag Netzentgelte 2027 (Video)";

const slideRow = (
  number: number,
  occurrences: [number, number][],
  over: Record<string, unknown> = {},
) => {
  const name = String(number).padStart(4, "0");
  return {
    id: `s${number}`,
    meeting_id: VIDEO_ID,
    number,
    origin: "video",
    image_path: `slides/${name}.jpg`,
    thumb_path: `slides/${name}_t.jpg`,
    occurrences: occurrences.map(([a, b]) => ({
      start_ms: a * 1000,
      end_ms: b * 1000,
    })),
    ocr_text: null,
    ocr_engine: null,
    kind: null,
    description: null,
    description_model: null,
    hidden: false,
    ...over,
  };
};

/** Fuenf sichtbare Folien (Folie 3 kehrt zurueck) und ein Sprecherbild, das ausgeblendet ist. */
const SLIDES = [
  slideRow(1, [[0, 60]]),
  slideRow(2, [[60, 150]]),
  slideRow(3, [
    [150, 200],
    [400, 430],
  ]),
  slideRow(4, [[200, 400]]),
  slideRow(5, [[430, 480]], {
    ocr_text: "Netzentgelte 2027\nGrundpreis: 70 €\nArbeitspreis: 13,1 ct/kWh",
  }),
  slideRow(6, [[480, 540]], { hidden: true }),
];

interface Setup {
  /** Folien der Besprechung m8 (Standard: SLIDES). */
  slides?: typeof SLIDES;
  /** Den Player so ersetzen, dass Sprungziele und Abspielzeit pruefbar sind. */
  fakeAudio?: boolean;
}

/** Attrappe der Aufnahmen-Seite mit der Video-Besprechung m8 und den Folien-Befehlen. */
const setup = async (page: Page, opts: Setup = {}) => {
  await installLiveMock(page);
  await page.addInitScript(
    ({ slides, fakeAudio, id, title }) => {
      const w = window as any;
      if (fakeAudio) {
        w.__played = [];
        const times = new WeakMap<object, number>();
        Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
          configurable: true,
          get() {
            return times.get(this) ?? 0;
          },
          set(v: number) {
            times.set(this, v);
          },
        });
        HTMLMediaElement.prototype.play = function () {
          w.__played.push(times.get(this) ?? 0);
          return Promise.resolve();
        };
        HTMLMediaElement.prototype.pause = function () {};
      }
      const t0 = 1_790_000_000;
      w.__meetings.push({
        id,
        title,
        status: "ready",
        source: "import",
        started_at: t0 + 6 * 86400,
        ended_at: t0 + 6 * 86400 + 3540,
        language: "de",
        mic_audio_path: "C:/Users/patrick/Videos/Vortrag.wav",
        system_audio_path: null,
        duration_ms: 3_540_000,
        consent_confirmed_at: t0,
        audio_retention_until: null,
        created_at: t0,
        source_path: "C:/Users/patrick/Videos/Vortrag.mp4",
        description: null,
        deleted_at: null,
      });
      w.__slides = { [id]: slides };
      w.__detectError = null;
      w.__hideError = null;
      // Bilder: ein eigener Pfad, den der Routen-Handler beantwortet.
      w.__TAURI_INTERNALS__.convertFileSrc = (p: string) =>
        p.includes("/slides/") ? `/__slide?p=${encodeURIComponent(p)}` : p;
      const inner = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        switch (cmd) {
          case "list_meeting_slides":
            w.__calls.push({ cmd, args });
            return (w.__slides[args.meetingId] ?? []).map((s: any) => ({
              ...s,
            }));
          case "meeting_slides_dir":
            return `C:\\data\\meetings\\${args.meetingId}`;
          case "set_meeting_slide_hidden": {
            w.__calls.push({ cmd, args });
            if (w.__hideError) throw w.__hideError;
            for (const list of Object.values(w.__slides) as any[][]) {
              const slide = list.find((s) => s.id === args.slideId);
              if (slide) slide.hidden = args.hidden;
            }
            return null;
          }
          case "detect_meeting_slides":
            w.__calls.push({ cmd, args });
            if (w.__detectError) throw w.__detectError;
            return null;
        }
        return inner(cmd, args);
      };
    },
    {
      slides: opts.slides ?? SLIDES,
      fakeAudio: opts.fakeAudio ?? true,
      id: VIDEO_ID,
      title: VIDEO_TITLE,
    },
  );
  // Jede Folie ein 16:9-Bild mit Nummer und einer Tabelle (reicht fuer Bildschirmfotos).
  await page.route(/\/__slide\?/, (route) => {
    const file = decodeURIComponent(
      new URL(route.request().url()).searchParams.get("p") ?? "",
    );
    const number = Number(/slides\/(\d+)/.exec(file)?.[1] ?? 0);
    const hue = (number * 47) % 360;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="450" viewBox="0 0 800 450">
      <rect width="800" height="450" fill="hsl(${hue},45%,92%)"/>
      <rect x="0" y="0" width="800" height="70" fill="hsl(${hue},55%,35%)"/>
      <text x="32" y="46" font-family="Segoe UI, Arial" font-size="30" fill="white">Netzentgelte 2027</text>
      <text x="32" y="150" font-family="Segoe UI, Arial" font-size="64" font-weight="700" fill="hsl(${hue},55%,25%)">Folie ${number}</text>
      <g fill="hsl(${hue},30%,70%)">
        <rect x="32" y="200" width="736" height="28" rx="4"/>
        <rect x="32" y="244" width="560" height="28" rx="4"/>
        <rect x="32" y="288" width="640" height="28" rx="4"/>
        <rect x="32" y="332" width="420" height="28" rx="4"/>
      </g></svg>`;
    return route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: svg,
    });
  });
};

const content = (page: Page) => page.getByTestId("rec-content");
const centerTab = (page: Page, name: string) =>
  content(page).getByRole("tab", { name, exact: true });
const cards = (page: Page) => page.getByTestId("slide-card");
const card = (page: Page, number: number) =>
  page.locator(`[data-testid="slide-card"][data-slide-number="${number}"]`);

const openVideo = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, VIDEO_ID);
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

const openSlides = async (page: Page, width = 1366, height = 768) => {
  await openVideo(page, width, height);
  await centerTab(page, "Folien").click();
  await expect(page.getByTestId("slides-panel")).toBeVisible();
};

const chooseMenu = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

const toast = (page: Page) => page.locator("[data-sonner-toast]");

/** Die Abspielzeit des (falschen) Players setzen und die Wiedergabe melden. */
const playAt = (page: Page, seconds: number) =>
  page.evaluate((s) => {
    const audio = document.querySelector("audio") as HTMLAudioElement;
    audio.currentTime = s;
    audio.dispatchEvent(new Event("play"));
  }, seconds);

// ---------------------------------------------------------------------------
// 1. Reine Logik
// ---------------------------------------------------------------------------

test.describe("D4 Logik", () => {
  const slides = SLIDES as unknown as Parameters<typeof slideAt>[0];

  test("aktuelle Folie: Bereich, Luecke, Ruecksprung, vor der ersten, ausgeblendete zaehlen nicht", () => {
    expect(slideAt(slides, 0)).toBe("s1");
    expect(slideAt(slides, 59_999)).toBe("s1");
    expect(slideAt(slides, 60_000)).toBe("s2");
    expect(slideAt(slides, 170_000)).toBe("s3");
    expect(slideAt(slides, 300_000)).toBe("s4");
    // Ruecksprung: dieselbe Folie 3 in ihrem zweiten Bereich.
    expect(slideAt(slides, 410_000)).toBe("s3");
    expect(slideAt(slides, 450_000)).toBe("s5");
    // Folie 6 ist ausgeblendet: danach gilt weiter die letzte sichtbare.
    expect(slideAt(slides, 500_000)).toBe("s5");
    // Luecke zwischen zwei Bereichen: die zuletzt begonnene.
    const gap = [
      slideRow(1, [[10, 20]]),
      slideRow(2, [[40, 50]]),
    ] as unknown as typeof slides;
    expect(slideAt(gap, 30_000)).toBe("s1");
    expect(slideAt(gap, 5_000)).toBeNull();
    expect(slideAt([], 5_000)).toBeNull();
  });

  test("Marken: je Zeitbereich einer sichtbaren Folie, ueber dem ersten Satz ab diesem Moment", () => {
    const marks = slideMarks(slides);
    expect(marks.map((m) => [m.number, m.startMs / 1000])).toEqual([
      [1, 0],
      [2, 60],
      [3, 150],
      [4, 200],
      [3, 400],
      [5, 430],
    ]);
    expect(visibleSlides(slides).map((s) => s.number)).toEqual([1, 2, 3, 4, 5]);
    // Segmente wie in der Attrappe: Nr. i beginnt bei i * 29 s + 4 s.
    const segments = Array.from({ length: 20 }, (_, i) => ({
      segment_index: i,
      start_ms: i * 29_000 + 4_000,
    }));
    const placed = marksBySegment(segments, marks);
    expect(placed.get(0)?.map((m) => m.number)).toEqual([1]);
    // Folie 2 beginnt bei 60 s: das erste Segment ab da ist Nr. 2 (62 s).
    expect(placed.get(2)?.map((m) => m.number)).toEqual([2]);
    expect([...placed.keys()]).toEqual([0, 2, 6, 7, 14, 15]);
    // Marken nach dem letzten Segment haengen am letzten.
    const tail = marksBySegment(segments.slice(0, 2), marks);
    expect(tail.get(1)?.length).toBeGreaterThan(0);
    expect(marksBySegment([], marks).size).toBe(0);
  });

  test("Folienerkennung: nur bei Video-Quelle; Auftragsleiste zaehlt wie Audio; Fehlercodes", () => {
    expect(
      canDetectSlides({ source: "import", source_path: "C:/V/a.MP4" }),
    ).toBe(true);
    expect(
      canDetectSlides({ source: "import", source_path: "C:/V/a.mkv" }),
    ).toBe(true);
    expect(
      canDetectSlides({ source: "import", source_path: "C:/A/a.m4a" }),
    ).toBe(false);
    expect(canDetectSlides({ source: "import", source_path: null })).toBe(
      false,
    );
    expect(canDetectSlides({ source: "youtube", source_path: null })).toBe(
      false,
    );
    expect(canDetectSlides({ source: "recording", source_path: null })).toBe(
      false,
    );
    expect(countsAudio({ phase: "slides" })).toBe(true);
    expect(countsAudio({ phase: "notes" })).toBe(false);
    expect(slidesErrorKey("recording_active")).toBe(
      "meetings.slides.errors.recordingActive",
    );
    expect(slidesErrorKey("meeting_not_finished")).toBe(
      "meetings.slides.errors.notFinished",
    );
    expect(slidesErrorKey("slides_no_video: Pfad")).toBe(
      "meetings.slides.errors.noVideo",
    );
    expect(slidesErrorKey("etwas_unbekanntes")).toBe(
      "meetings.slides.errors.failed",
    );
    expect(slideFilePath("C:\\data\\meetings\\m8\\", "slides/0001.jpg")).toBe(
      "C:\\data\\meetings\\m8/slides/0001.jpg",
    );
  });
});

// ---------------------------------------------------------------------------
// 2. Reiter Folien
// ---------------------------------------------------------------------------

test.describe("Reiter Folien", () => {
  test("mit Folien erscheint der Reiter; sichtbar sind die nicht ausgeblendeten, mit Vorschaubild", async ({
    page,
  }) => {
    await setup(page);
    await openVideo(page);
    await expect(content(page).getByRole("tab")).toHaveText([
      "Transkript",
      "Protokoll",
      "Folien",
    ]);
    await centerTab(page, "Folien").click();
    await expect(centerTab(page, "Folien")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(cards(page)).toHaveCount(5);
    await expect(page.getByTestId("slides-summary")).toHaveText("5 Folien");
    await expect(card(page, 6)).toHaveCount(0);
    await expect(card(page, 3)).toContainText("Folie 3 · 2:30");
    // Das Vorschaubild kommt aus dem Besprechungsordner (thumb_path) und laedt.
    const img = card(page, 1).locator("img");
    await expect(img).toHaveAttribute("src", /_t\.jpg/);
    await expect
      .poll(() => img.evaluate((el) => (el as HTMLImageElement).naturalWidth))
      .toBeGreaterThan(0);
    // (React.StrictMode ruft Effekte im Entwicklungsbetrieb zweimal auf.)
    expect((await calls(page, "list_meeting_slides")).length).toBeGreaterThan(
      0,
    );
    await shoot(page, "d4-folienleiste");
  });

  test("ohne Folien (Audio-Besprechung, Video ohne Lauf) gibt es keinen Reiter", async ({
    page,
  }) => {
    await setup(page, { slides: [] });
    await openVideo(page);
    await expect(content(page).getByRole("tab")).toHaveText([
      "Transkript",
      "Protokoll",
    ]);
    await pickMeeting(page, "m2");
    await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
    await expect(content(page).getByRole("tab")).toHaveText([
      "Transkript",
      "Protokoll",
    ]);
  });

  test("ein gemerkter Reiter Folien faellt ohne Folien auf das Transkript zurueck", async ({
    page,
  }) => {
    await page.addInitScript(() =>
      localStorage.setItem("lva.ui.meetings.centerTab", "slides"),
    );
    await setup(page, { slides: [] });
    await openVideo(page);
    await expect(centerTab(page, "Transkript")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("ein gemerkter Reiter Folien bleibt bei einer Besprechung mit Folien stehen", async ({
    page,
  }) => {
    await page.addInitScript(() =>
      localStorage.setItem("lva.ui.meetings.centerTab", "slides"),
    );
    await setup(page);
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, VIDEO_ID);
    await expect(page.getByTestId("slides-panel")).toBeVisible();
    await expect(cards(page)).toHaveCount(5);
  });

  test("Klick auf eine Folie springt im Audio an die Startzeit und markiert sie", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 2).getByTestId("slide-thumb").click();
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([60]);
    await expect(card(page, 2)).toHaveAttribute("data-active", "true");
    // Folie 3 beginnt bei der ERSTEN Sichtung, auch wenn sie spaeter wiederkehrt.
    await card(page, 3).getByTestId("slide-thumb").click();
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([60, 150]);
  });

  test("die aktuelle Folie folgt der Abspielzeit (auch beim Ruecksprung); im Transkript leuchtet die Marke mit", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 1).getByTestId("slide-thumb").click();
    await expect(card(page, 1)).toHaveAttribute("data-active", "true");
    await playAt(page, 170);
    await expect(card(page, 3)).toHaveAttribute("data-active", "true");
    await expect(
      page.getByTestId("slides-panel").locator('[data-active="true"]'),
    ).toHaveCount(1);
    await playAt(page, 250);
    await expect(card(page, 4)).toHaveAttribute("data-active", "true");
    // Ruecksprung: Folie 3 in ihrem zweiten Bereich.
    await playAt(page, 410);
    await expect(card(page, 3)).toHaveAttribute("data-active", "true");
    await playAt(page, 460);
    await expect(card(page, 5)).toHaveAttribute("data-active", "true");
    await centerTab(page, "Transkript").click();
    await expect(
      page.locator('[data-testid="slide-mark"][data-active="true"]'),
    ).toHaveAttribute("data-slide-number", "5");
    await centerTab(page, "Folien").click();
    await shoot(page, "d4-aktuelle-folie");
  });

  test("schmales Fenster: die Leiste bricht um, nichts laeuft ueber", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page, 480, 800);
    await expect(cards(page).first()).toBeVisible();
    const over = await page.evaluate(
      () => document.scrollingElement!.scrollWidth > innerWidth + 1,
    );
    expect(over).toBe(false);
  });

  test("Barrierefreiheit: Leiste und Grossansicht ohne schwere axe-Befunde", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 2).getByTestId("slide-hide").click();
    const panel = await new AxeBuilder({ page })
      .include('[data-testid="slides-panel"]')
      .analyze();
    expect(
      panel.violations.filter(
        (v) => v.impact === "serious" || v.impact === "critical",
      ),
    ).toEqual([]);
    await card(page, 3).getByTestId("slide-open").click();
    const dialog = await new AxeBuilder({ page })
      .include('[role="dialog"]')
      .analyze();
    expect(
      dialog.violations.filter(
        (v) => v.impact === "serious" || v.impact === "critical",
      ),
    ).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// 3. Marken im Transkript
// ---------------------------------------------------------------------------

test.describe("Folienmarken im Transkript", () => {
  test("dezente Marke je Zeitbereich ueber dem ersten Satz nach dem Wechsel; Klick oeffnet die Grossansicht", async ({
    page,
  }) => {
    await setup(page);
    await openVideo(page);
    const marks = page.getByTestId("slide-mark");
    await expect(marks).toHaveCount(6);
    await expect(marks.first()).toHaveAttribute("data-slide-number", "1");
    // Folie 2 beginnt bei 60 s: sie steht ueber Segment 2 (Beginn 62 s).
    const before = page
      .locator('[data-segment-index="2"]')
      .locator("xpath=preceding-sibling::*[1]");
    await expect(before).toHaveAttribute("data-testid", "slide-mark");
    await expect(before).toHaveAttribute("data-slide-number", "2");
    await expect(before).toContainText("Folie 2");
    await shoot(page, "d4-folienmarken");
    await before.getByRole("button").click();
    await expect(page.getByTestId("slide-viewer")).toHaveAttribute(
      "data-slide-number",
      "2",
    );
  });

  test("ohne Folien bleibt das Transkript unveraendert", async ({ page }) => {
    await setup(page, { slides: [] });
    await openVideo(page);
    await expect(page.getByTestId("slide-mark")).toHaveCount(0);
  });

  test("Ausblenden nimmt die Marken der Folie mit", async ({ page }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 2).getByTestId("slide-hide").click();
    await centerTab(page, "Transkript").click();
    await expect(page.getByTestId("slide-mark")).toHaveCount(5);
    await expect(
      page.locator('[data-testid="slide-mark"][data-slide-number="2"]'),
    ).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// 4. Ausblenden
// ---------------------------------------------------------------------------

test.describe("Ausblenden", () => {
  test("Ausblenden meldet das Backend, die Folie verschwindet; Einblenden bringt sie zurueck", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 2).getByTestId("slide-hide").click();
    await expect
      .poll(async () => (await calls(page, "set_meeting_slide_hidden")).length)
      .toBe(1);
    expect((await calls(page, "set_meeting_slide_hidden"))[0].args).toEqual({
      slideId: "s2",
      hidden: true,
    });
    await expect(cards(page)).toHaveCount(4);
    await expect(page.getByTestId("slides-summary")).toHaveText("4 Folien");
    // Zwei ausgeblendete (Folie 2 und das Sprecherbild 6): ein Schalter zeigt sie.
    const toggle = page.getByTestId("slides-show-hidden");
    await expect(toggle).toBeVisible();
    await expect(page.getByText("Ausgeblendete anzeigen (2)")).toBeVisible();
    await toggle.check();
    await expect(cards(page)).toHaveCount(6);
    await expect(card(page, 2)).toHaveAttribute("data-hidden", "true");
    await card(page, 2).getByTestId("slide-unhide").click();
    await expect
      .poll(async () => (await calls(page, "set_meeting_slide_hidden")).length)
      .toBe(2);
    expect((await calls(page, "set_meeting_slide_hidden"))[1].args).toEqual({
      slideId: "s2",
      hidden: false,
    });
    await expect(card(page, 2)).not.toHaveAttribute("data-hidden", "true");
    await shoot(page, "d4-ausgeblendete");
  });

  test("der Zustand bleibt beim erneuten Oeffnen der Besprechung erhalten", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 4).getByTestId("slide-hide").click();
    await expect(cards(page)).toHaveCount(4);
    await pickMeeting(page, "m2");
    await expect(page.getByTestId("slides-panel")).toHaveCount(0);
    await pickMeeting(page, VIDEO_ID);
    await centerTab(page, "Folien").click();
    await expect(cards(page)).toHaveCount(4);
    await expect(card(page, 4)).toHaveCount(0);
  });

  test("scheitert das Ausblenden, kommt die Folie zurueck und eine Meldung erscheint", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await page.evaluate(
      () => ((window as any).__hideError = "slide_not_found"),
    );
    await card(page, 2).getByTestId("slide-hide").click();
    await expect(toast(page)).toContainText("Diese Folie gibt es nicht mehr.");
    await expect(cards(page)).toHaveCount(5);
    await expect(card(page, 2)).toBeVisible();
  });

  test("sind alle Folien ausgeblendet, sagt der Reiter es und bietet den Schalter an", async ({
    page,
  }) => {
    await setup(page, {
      slides: [slideRow(1, [[0, 60]], { hidden: true })] as typeof SLIDES,
    });
    await openSlides(page);
    await expect(page.getByTestId("slides-all-hidden")).toHaveText(
      "Alle Folien sind ausgeblendet.",
    );
    await page.getByTestId("slides-show-hidden").check();
    await expect(cards(page)).toHaveCount(1);
  });
});

// ---------------------------------------------------------------------------
// 5. Grossansicht
// ---------------------------------------------------------------------------

test.describe("Grossansicht", () => {
  test("oeffnet mit der Folie, blaettert mit Knoepfen und Pfeiltasten, springt im Audio", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 3).getByTestId("slide-open").click();
    const dialog = page.getByRole("dialog", { name: "Folie 3 von 6" });
    await expect(dialog).toBeVisible();
    const image = dialog.getByTestId("slide-viewer-image");
    await expect(image).toHaveAttribute("src", /0003\.jpg/);
    await expect
      .poll(() => image.evaluate((el) => (el as HTMLImageElement).naturalWidth))
      .toBeGreaterThan(0);
    await expect(dialog).toContainText("Zu sehen: 2:30–3:20, 6:40–7:10");
    await shoot(page, "d4-grossansicht");
    await dialog.getByTestId("slide-next").click();
    await expect(
      page.getByRole("dialog", { name: "Folie 4 von 6" }),
    ).toBeVisible();
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await expect(
      page.getByRole("dialog", { name: "Folie 2 von 6" }),
    ).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(
      page.getByRole("dialog", { name: "Folie 3 von 6" }),
    ).toBeVisible();
    // "Ab hier abspielen" springt an die erste Sichtung und schliesst.
    await page.getByTestId("slide-viewer-play").click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([150]);
    await expect(card(page, 3)).toHaveAttribute("data-active", "true");
  });

  test("erste Folie ohne Zurueck, letzte ohne Weiter; Escape schliesst", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 1).getByTestId("slide-open").click();
    await expect(page.getByTestId("slide-prev")).toBeDisabled();
    await expect(page.getByTestId("slide-next")).toBeEnabled();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await card(page, 5).getByTestId("slide-open").click();
    await expect(page.getByTestId("slide-next")).toBeDisabled();
  });

  test("Text der Folie steht darunter, sobald es ihn gibt", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 1).getByTestId("slide-open").click();
    await expect(page.getByTestId("slide-viewer-text")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await card(page, 5).getByTestId("slide-open").click();
    const text = page.getByTestId("slide-viewer-text");
    await text.locator("summary").click();
    await expect(text).toContainText("Grundpreis: 70 €");
  });

  test("Ausblenden in der Grossansicht gilt sofort; die Folie bleibt bis zum Blaettern im Dialog", async ({
    page,
  }) => {
    await setup(page);
    await openSlides(page);
    await card(page, 4).getByTestId("slide-open").click();
    await page.getByTestId("slide-viewer-hide").click();
    await expect(page.getByTestId("slide-viewer-hide")).toHaveText(
      /Wieder einblenden/,
    );
    await expect(page.getByTestId("slide-viewer")).toContainText(
      "ausgeblendet",
    );
    await page.getByTestId("slide-prev").click();
    await page.getByTestId("slide-next").click();
    // Nach dem Blaettern ist die ausgeblendete Folie nicht mehr dabei: von 3 geht es zu 5.
    await expect(
      page.getByRole("dialog", { name: "Folie 5 von 6" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(cards(page)).toHaveCount(4);
  });

  test("ohne Player springt ein Klick auf die Vorschau in die Grossansicht", async ({
    page,
  }) => {
    // Eine Besprechung ohne Audio: die Folien sind da, aber es gibt nichts zum Abspielen.
    await setup(page);
    await page.addInitScript(() => {
      const w = window as any;
      w.__meetings.find((m: any) => m.id === "m8").mic_audio_path = null;
    });
    await openSlides(page);
    await card(page, 2).getByTestId("slide-thumb").click();
    await expect(
      page.getByRole("dialog", { name: "Folie 2 von 6" }),
    ).toBeVisible();
    await expect(page.getByTestId("slide-viewer-play")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// 6. Folien erkennen im Menue
// ---------------------------------------------------------------------------

test.describe("Folien erkennen", () => {
  test("Video-Besprechung: Eintrag im Menue, Lauf mit Fortschritt, Pause und Stopp ueber die Auftragsleiste, am Ende der Reiter", async ({
    page,
  }) => {
    await setup(page, { slides: [] });
    await openVideo(page);
    await expect(centerTab(page, "Folien")).toHaveCount(0);
    await chooseMenu(page, "menu-detect-slides");
    await expect
      .poll(async () => (await calls(page, "detect_meeting_slides")).length)
      .toBe(1);
    expect((await calls(page, "detect_meeting_slides"))[0].args).toEqual({
      meetingId: VIDEO_ID,
      options: { video_path: null, sample_interval_s: null },
    });
    // Fortschritt kommt ueber die vorhandene Auftragsleiste.
    await emitMeeting(page, {
      kind: "progress",
      meeting_id: VIDEO_ID,
      phase: "slides",
      done: 1_180_000,
      total: 3_540_000,
      elapsed_ms: 40_000,
      eta_ms: 80_000,
      state: "running",
      pausable: true,
    });
    const panel = page.getByTestId("job-panel");
    await expect(panel).toHaveAttribute("data-phase", "slides");
    await expect(page.getByTestId("job-phase")).toHaveText("Folien");
    await expect(page.getByTestId("job-percent")).toHaveText("33 %");
    await expect(page.getByTestId("job-amount")).toHaveText(
      "19:40 von 59:00 Video",
    );
    await expect(page.getByTestId("job-pause")).toBeEnabled();
    await expect(page.getByTestId("job-stop")).toBeEnabled();
    // Waehrend des Laufs ist der Menue-Eintrag gesperrt und das Transkript waechst nicht mit.
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByTestId("menu-detect-slides")).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("autoscroll-toggle")).toHaveCount(0);
    // Stopp fragt nach und nennt, was bleibt.
    await page.getByTestId("job-stop").click();
    await expect(page.getByRole("dialog")).toContainText(
      "Die bisher erkannten Folien bleiben erhalten.",
    );
    await page.getByTestId("job-stop-cancel").click();
    await shoot(page, "d4-lauf");
    // Ende: Folien liegen vor, der Lauf meldet "done".
    await page.evaluate((slides) => {
      const w = window as any;
      w.__slides.m8 = slides;
      w.__emit("meeting-event", {
        kind: "job_ended",
        meeting_id: "m8",
        phase: "slides",
        stopped: false,
      });
      w.__emit("meeting-slides-event", {
        kind: "done",
        meeting_id: "m8",
        slides: 5,
        added: 5,
      });
    }, SLIDES);
    await expect(toast(page)).toContainText("5 Folien erkannt.");
    await expect(centerTab(page, "Folien")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(cards(page)).toHaveCount(5);
    await expect(page.getByTestId("job-panel")).toHaveCount(0);
  });

  test("ein gestoppter Lauf meldet, wie viele Folien bleiben, und holt den Reiter nicht nach vorn", async ({
    page,
  }) => {
    await setup(page, { slides: [] });
    await openVideo(page);
    await page.evaluate((slides) => {
      const w = window as any;
      w.__slides.m8 = slides.slice(0, 2);
      w.__emit("meeting-slides-event", {
        kind: "stopped",
        meeting_id: "m8",
        slides: 2,
        added: 2,
      });
    }, SLIDES);
    await expect(toast(page)).toContainText(
      "Folienerkennung gestoppt. 2 Folien bleiben erhalten.",
    );
    await expect(centerTab(page, "Folien")).toBeVisible();
    await expect(centerTab(page, "Transkript")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  test("Audio-Besprechung und YouTube-Besprechung haben den Eintrag nicht", async ({
    page,
  }) => {
    await setup(page);
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, "m2");
    await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByRole("menu")).toBeVisible();
    await expect(page.getByTestId("menu-retranscribe")).toBeVisible();
    await expect(page.getByTestId("menu-detect-slides")).toHaveCount(0);
  });

  for (const [code, text] of [
    [
      "recording_active",
      "Eine Aufnahme läuft. Die Folien lassen sich danach erkennen.",
    ],
    [
      "meeting_not_finished",
      "Die Besprechung wird noch verarbeitet. Die Folien lassen sich danach erkennen.",
    ],
    ["slides_no_video", "Zu dieser Besprechung gibt es keine Videodatei."],
    ["slides_video_missing", "Die Videodatei wurde nicht gefunden."],
    ["job_busy", "Für diese Besprechung läuft bereits eine Verarbeitung."],
  ] as const) {
    test(`Startfehler ${code} wird uebersetzt gemeldet`, async ({ page }) => {
      await setup(page, { slides: [] });
      await openVideo(page);
      await page.evaluate((c) => ((window as any).__detectError = c), code);
      await chooseMenu(page, "menu-detect-slides");
      await expect(toast(page)).toContainText(text);
      await expect(centerTab(page, "Folien")).toHaveCount(0);
    });
  }

  for (const [kind, code, text] of [
    [
      "skipped",
      "slides_no_video",
      "Zu dieser Besprechung gibt es keine Videodatei.",
    ],
    [
      "skipped",
      "slides_ffmpeg_missing",
      "Die Folienerkennung braucht ffmpeg, das nicht gefunden wurde.",
    ],
    [
      "failed",
      "slides_disk_full",
      "Der Speicherplatz reicht nicht für die Folienbilder.",
    ],
    ["failed", "slides_ffmpeg_failed", "Das Video ließ sich nicht lesen."],
    [
      "failed",
      "slides_too_long",
      "Das Video ist für die Folienerkennung zu lang.",
    ],
    ["failed", "slides_panic", "Die Folienerkennung ist fehlgeschlagen."],
  ] as const) {
    test(`Ende ${kind} (${code}) wird uebersetzt gemeldet`, async ({
      page,
    }) => {
      await setup(page, { slides: [] });
      await openVideo(page);
      await page.evaluate(
        ([k, c]) =>
          (window as any).__emit("meeting-slides-event", {
            kind: k,
            meeting_id: "m8",
            code: c,
          }),
        [kind, code],
      );
      await expect(toast(page)).toContainText(text);
    });
  }

  test("das Ende eines anderen Laufs meldet sich auch, wenn die Besprechung nicht gewaehlt ist", async ({
    page,
  }) => {
    await setup(page);
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, "m2");
    await page.evaluate(() =>
      (window as any).__emit("meeting-slides-event", {
        kind: "done",
        meeting_id: "m8",
        slides: 1,
        added: 1,
      }),
    );
    await expect(toast(page)).toContainText("1 Folie erkannt.");
  });
});

// ---------------------------------------------------------------------------
// 7. Import mit der Option "Folien erkennen"
// ---------------------------------------------------------------------------

test.describe("Import: Folien erkennen", () => {
  const importFile = async (page: Page, file: string) => {
    await openRecordings(page, 1366, 768);
    await page.evaluate((f) => ((window as any).__pick = f), file);
    await page.getByTestId("import-open").click();
    const dialog = page.getByTestId("import-dialog");
    await expect(dialog).toContainText(file.split("/").pop()!);
    return dialog;
  };

  test("bei einem Video fragt der Einwilligungsdialog danach; angekreuzt startet die Erkennung nach dem Import", async ({
    page,
  }) => {
    await setup(page);
    const dialog = await importFile(page, "C:/Videos/Vortrag-neu.mp4");
    const option = dialog.getByTestId("import-detect-slides");
    await expect(option).toBeVisible();
    await expect(option).not.toBeChecked();
    await expect(dialog).toContainText("Folien erkennen");
    await option.check();
    await page.getByTestId("import-confirm").click();
    // Der Import beginnt, danach (Status "fertig") startet die Erkennung mit der Quelle des Imports.
    await expect
      .poll(async () => (await calls(page, "meetings_import_file")).length)
      .toBe(1);
    await expect
      .poll(async () => (await calls(page, "detect_meeting_slides")).length)
      .toBe(1);
    const [started] = await calls(page, "detect_meeting_slides");
    const [imported] = await calls(page, "meetings_import_file");
    expect(imported.args.path).toBe("C:/Videos/Vortrag-neu.mp4");
    expect(started.args.options).toEqual({
      video_path: null,
      sample_interval_s: null,
    });
    expect(typeof started.args.meetingId).toBe("string");
    // Vorgemerkt ist danach nichts mehr.
    expect(
      await page.evaluate(() =>
        localStorage.getItem("lva.meetings.slidesPending"),
      ),
    ).toBe("[]");
  });

  test("ohne Haken startet keine Erkennung", async ({ page }) => {
    await setup(page);
    await importFile(page, "C:/Videos/Vortrag-neu.mp4");
    await page.getByTestId("import-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_import_file")).length)
      .toBe(1);
    await page.waitForTimeout(400);
    expect(await calls(page, "detect_meeting_slides")).toHaveLength(0);
  });

  test("bei einer Audiodatei gibt es die Option nicht", async ({ page }) => {
    await setup(page);
    const dialog = await importFile(page, "C:/Audio/Kunde.m4a");
    await expect(dialog.getByTestId("import-detect-slides")).toHaveCount(0);
  });

  test("solange der Import laeuft, wartet die Erkennung; ein Abbruch verwirft den Wunsch", async ({
    page,
  }) => {
    await setup(page);
    await page.addInitScript(() => (window as any).__holdImport());
    const dialog = await importFile(page, "C:/Videos/Vortrag-neu.mp4");
    await dialog.getByTestId("import-detect-slides").check();
    await page.getByTestId("import-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_import_file")).length)
      .toBe(1);
    await page.waitForTimeout(400);
    expect(await calls(page, "detect_meeting_slides")).toHaveLength(0);
    expect(
      await page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("lva.meetings.slidesPending") ?? "[]")
            .length,
      ),
    ).toBe(1);
    // Der Import endet gescheitert: die Erkennung entfaellt.
    await page.evaluate(() => {
      const w = window as any;
      const id = w.__queue.running[0];
      w.__queue.running = [];
      w.__queueSetStatus(id, "failed");
    });
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            JSON.parse(
              localStorage.getItem("lva.meetings.slidesPending") ?? "[]",
            ).length,
        ),
      )
      .toBe(0);
    expect(await calls(page, "detect_meeting_slides")).toHaveLength(0);
  });

  test("scheitert der Start nach dem Import, kommt die Meldung und der Wunsch ist weg", async ({
    page,
  }) => {
    await setup(page);
    await page.addInitScript(
      () => ((window as any).__detectError = "slides_no_video"),
    );
    const dialog = await importFile(page, "C:/Videos/Vortrag-neu.mp4");
    await dialog.getByTestId("import-detect-slides").check();
    await page.getByTestId("import-confirm").click();
    await expect(toast(page)).toContainText(
      "Zu dieser Besprechung gibt es keine Videodatei.",
    );
    expect(
      await page.evaluate(() =>
        localStorage.getItem("lva.meetings.slidesPending"),
      ),
    ).toBe("[]");
  });
});

// ---------------------------------------------------------------------------
// 8. D5: Folien als Belege (Protokoll `[F5]`, KI-Notizen `F5`, Herkunft, Chat)
// ---------------------------------------------------------------------------

const MINUTES_BODY =
  "# Protokoll: Vortrag\n\n## Zusammenfassung\n\n- Grundpreis 70 € [F5]\n- Erfundener Beleg [F99]\n";

const ENTRY_FLAGS = {
  unsupported: false,
  dropped_sources: 0,
  placed_by_fallback: false,
  edited: false,
};

/** KI-Notizen, deren einziger Beleg die Folie 5 ist (kein Segment). */
const NOTES_WITH_SLIDE = {
  format: "enhanced@1",
  template_id: "builtin:allgemein",
  template_title: "Allgemein",
  segment_epoch: 0,
  sections: [
    {
      id: "summary",
      title: "Zusammenfassung",
      kind: "text",
      entries: [
        {
          id: "E1",
          origin: "ai",
          text: "Arbeitspreis 13,1 ct/kWh",
          note_id: null,
          source_segment_ids: [],
          source_slide_ids: [5],
          assignee: null,
          due: null,
          flags: ENTRY_FLAGS,
        },
      ],
    },
  ],
  stats: {
    user_notes_total: 0,
    user_notes_by_model: 0,
    user_notes_by_fallback: 0,
    ai_entries: 1,
    ai_entries_sourced: 1,
    dropped_source_ids: 0,
    chunks_total: 1,
    chunks_failed: [],
    single_pass: true,
  },
};

/** Besprechung m8 mit Protokoll und KI-Notizen, die auf Folie 5 verweisen, samt Herkunft des Protokolls. */
const setupRefs = async (page: Page) => {
  await setup(page);
  await page.addInitScript(
    ({ id, minutes, notes }) => {
      const w = window as any;
      w.__documents.push(
        {
          id: "pm1",
          meeting_id: id,
          kind: "minutes",
          body_format: "markdown@1",
          body: minutes,
          version: 1,
          created_at: 1790000900,
          template_id: null,
          updated_at: 301,
        },
        {
          id: "pn1",
          meeting_id: id,
          kind: "enhanced_notes",
          body_format: "enhanced@1",
          body: JSON.stringify(notes),
          version: 1,
          created_at: 1790000700,
          template_id: "builtin:allgemein",
          updated_at: 100,
        },
      );
      const inner = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        if (cmd === "provenance_get") {
          w.__calls.push({ cmd, args });
          if (args.id !== "pm1") return [];
          return [
            {
              id: "pv1",
              subject_kind: "document",
              subject_id: "pm1",
              subject_revision: null,
              created_at: 1790000900000,
              operation: "minutes",
              actor_kind: "user",
              actor_ref: null,
              provider: "local",
              locality: "local",
              model_id: "gemma",
              model_label: "Gemma 4 E4B",
              usage_event_id: null,
              prompt_tokens: 1200,
              completion_tokens: 300,
              duration_ms: 5000,
              sources: [
                {
                  kind: "transcript",
                  ref: id,
                  title: "Vortrag",
                  url: null,
                },
                {
                  kind: "slide",
                  ref: "s5",
                  title: "Folie 5 · 07:10",
                  url: null,
                },
              ],
              confidence: null,
              params_json: null,
              origin: "recorded",
            },
          ];
        }
        return inner(cmd, args);
      };
    },
    { id: VIDEO_ID, minutes: MINUTES_BODY, notes: NOTES_WITH_SLIDE },
  );
};

test.describe("D5 Folien als Belege", () => {
  test("Protokoll: [F5] ist eine Marke und springt zur Folie, [F99] bleibt Text", async ({
    page,
  }) => {
    await setupRefs(page);
    await openVideo(page);
    await centerTab(page, "Protokoll").click();
    const doc = page.getByTestId("minutes-doc");
    await expect(doc).toContainText("Grundpreis 70 €");
    const refs = doc.getByTestId("slide-ref");
    await expect(refs).toHaveCount(1);
    await expect(refs).toHaveAttribute("data-slide-ref", "5");
    await expect(refs).toHaveText("F5");
    // Eine Folie, die es nicht gibt, bleibt Text.
    await expect(doc).toContainText("[F99]");
    await refs.click();
    await expect(centerTab(page, "Folien")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(card(page, 5)).toHaveAttribute("data-active", "true");
    // Im Audio an die erste Sichtung der Folie (430 s).
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([430]);
    await shoot(page, "d5-protokoll-beleg");
  });

  test("KI-Notizen: eine Folie allein ist ein Beleg (kein 'ohne Beleg'), der Chip springt zur Folie", async ({
    page,
  }) => {
    await setupRefs(page);
    await openVideo(page);
    await page
      .getByRole("tab", { name: "KI-Notizen", exact: true })
      .first()
      .click();
    const entry = page.locator(
      '[data-testid="enhanced-entry"][data-entry-id="E1"]',
    );
    await expect(entry).toContainText("Arbeitspreis 13,1 ct/kWh");
    await expect(entry.getByTestId("no-evidence")).toHaveCount(0);
    const chip = entry.getByTestId("source-slide");
    await expect(chip).toHaveText("F5");
    await expect(chip).toHaveAttribute("title", "Zu Folie 5 springen (07:10)");
    await chip.click();
    await expect(centerTab(page, "Folien")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(card(page, 5)).toHaveAttribute("data-active", "true");
    await expect
      .poll(() => page.evaluate(() => (window as any).__played))
      .toEqual([430]);
  });

  test("Herkunft: die Quelle 'Folie 5 · 07:10' ist anklickbar und fuehrt zur Folie", async ({
    page,
  }) => {
    await setupRefs(page);
    await openVideo(page);
    await centerTab(page, "Protokoll").click();
    await page
      .getByTestId("prov-area-minutes")
      .click({ button: "right", position: { x: 12, y: 12 } });
    await page
      .getByRole("menuitem", { name: "Herkunft" })
      .evaluate((el) => (el as HTMLElement).click());
    const dialog = page.getByTestId("provenance-dialog");
    await expect(dialog.getByTestId("prov-sources")).toContainText(
      "Folie 5 · 07:10",
    );
    // Die Transkript-Quelle bleibt Text, nur Folien sind Schaltflaechen.
    await expect(dialog.getByTestId("prov-slide-source")).toHaveCount(1);
    await dialog.getByTestId("prov-slide-source").click();
    await expect(dialog).toBeHidden();
    await expect(centerTab(page, "Folien")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(card(page, 5)).toHaveAttribute("data-active", "true");
  });

  test("Protokoll ohne Folien: [F5] bleibt Text, es gibt keine Marke", async ({
    page,
  }) => {
    await setupRefs(page);
    // Besprechung ohne erkannte Folien (der Beleg verweist ins Leere).
    await page.addInitScript(() => {
      (window as any).__slides = { m8: [] };
    });
    await openVideo(page);
    await centerTab(page, "Protokoll").click();
    const doc = page.getByTestId("minutes-doc");
    await expect(doc).toContainText("[F5]");
    await expect(doc.getByTestId("slide-ref")).toHaveCount(0);
  });
});
