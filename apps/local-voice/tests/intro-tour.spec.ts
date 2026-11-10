import { oeffneProfilmenue } from "./profilmenue";
import { test, expect } from "@playwright/test";
import {
  NEMOTRON,
  commands,
  emit,
  finishDownload,
  setup,
} from "./introTourMock";

// Erstinstallation: Diktatmodell (Nemotron 3.5) lädt ohne Auswahlbildschirm,
// die Einführungstour zeigt Fortschritt und Seiten.
test("Erstinstallation lädt Nemotron 3.5 und startet die Einführungstour", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");

  const tour = page.getByTestId("intro-tour");
  const dialog = tour.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
  // Kein Auswahlbildschirm: Download ohne Rückfrage gestartet.
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "download_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);

  // Fortschritt als progressbar; die Tour beginnt erst mit bereitem Modell.
  await emit(page, "model-download-progress", {
    model_id: NEMOTRON,
    downloaded: 40,
    total: 100,
    percentage: 40,
  });
  const bar = dialog.getByRole("progressbar");
  await expect(bar).toHaveAttribute("aria-valuenow", "40");
  await expect(bar).toHaveAttribute(
    "aria-valuetext",
    /Nemotron 3.5 ASR Streaming wird eingerichtet … 40 %/,
  );
  const start = dialog.getByRole("button", { name: "Tour starten" });
  await expect(start).toBeDisabled();
  await page.keyboard.press("ArrowRight");
  await expect(
    dialog.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();

  // Download fertig: Modell wird zum Diktatmodell, Bereitschaft wird angesagt.
  await finishDownload(page);
  await expect
    .poll(async () =>
      (await commands(page)).some(
        (c) =>
          c.cmd === "set_active_model" &&
          (c.args as { modelId?: string }).modelId === NEMOTRON,
      ),
    )
    .toBe(true);
  await expect(tour.getByRole("alert")).toContainText("ist bereit");
  await expect(start).toBeEnabled();
  await expect(start).toBeFocused();

  // Diktieren mit Kürzel aus den Einstellungen; Schrittwechsel wird angesagt.
  await start.click();
  await expect(
    tour.getByRole("heading", { name: "In Textfelder diktieren" }),
  ).toBeVisible();
  await expect(tour).toContainText("Strg + Leertaste gedrückt halten");
  await expect(tour.getByRole("status")).toHaveText(
    "2/6: In Textfelder diktieren",
  );
  // Fokus liegt auf „Weiter“: Pfeiltasten schalten dann nicht weiter.
  const weiter = tour.getByRole("button", { name: "Weiter" });
  await expect(weiter).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(
    tour.getByRole("heading", { name: "In Textfelder diktieren" }),
  ).toBeVisible();

  // Übungsfeld: beschriftet, freigegeben, fokussiert.
  await weiter.click();
  const field = tour.getByRole("textbox", { name: "Übungsfeld zum Diktieren" });
  await expect(field).toBeEnabled();
  await expect(field).toBeFocused();
  // Während einer Aufnahme bricht Esc die Aufnahme ab, die Tour bleibt.
  await page.evaluate(() => {
    (window as unknown as { __recording: boolean }).__recording = true;
  });
  await page.keyboard.press("Escape");
  await expect
    .poll(async () =>
      (await commands(page)).some((c) => c.cmd === "is_recording"),
    )
    .toBe(true);
  await expect(field).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { __recording: boolean }).__recording = false;
  });

  // Fokusfalle: Tab bleibt im Dialog.
  for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
  expect(
    await page.evaluate(() =>
      document.querySelector("[role=dialog]")?.contains(document.activeElement),
    ),
  ).toBe(true);

  // Seiten im Spotlight.
  await tour.getByRole("button", { name: "Weiter" }).click();
  await expect(tour).toHaveAttribute("data-mode", "spot");
  await expect(
    tour.getByRole("heading", { name: "Alles auf einen Blick" }),
  ).toBeVisible();

  // Esc schließt die Tour; Fokus landet auf einem sinnvollen Ziel.
  await page.keyboard.press("Escape");
  await expect(tour).toHaveCount(0);
  // lva#394: Startseite ist der Chat, der Fokus landet im Eingabefeld.
  await expect(page.getByTestId("chat-input")).toBeFocused();
  const again = {
    click: async () => {
      await oeffneProfilmenue(page);
      await page.getByRole("button", { name: /^Hilfe/ }).click();
      await page.getByTestId("titlebar-start-tour").click();
    },
  };

  // Erneut starten löst keinen weiteren Download aus.
  const downloads = (await commands(page)).filter(
    (c) => c.cmd === "download_model",
  ).length;
  await again.click();
  await expect(
    page.getByRole("heading", { name: "Schön, dass du da bist" }),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("intro-tour")
      .getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
  expect(
    (await commands(page)).filter((c) => c.cmd === "download_model").length,
  ).toBe(downloads);
});

test("Gescheiterter Download: Wiederholen in der Tour", async ({ page }) => {
  await setup(page, { downloadFails: 1 });
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await expect(tour.getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  await expect(
    tour.getByRole("button", { name: "Tour starten" }),
  ).toBeDisabled();
  await tour.getByRole("button", { name: "Download wiederholen" }).click();
  await expect(tour.getByRole("progressbar")).toBeVisible();
  expect(
    (await commands(page)).filter((c) => c.cmd === "download_model").length,
  ).toBe(2);
  await finishDownload(page);
  await expect(
    tour.getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
});

test("Bewegung reduzieren zeigt das Diktat als Endzustand", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await setup(page);
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await finishDownload(page);
  await tour.getByRole("button", { name: "Tour starten" }).click();
  await expect(tour).toHaveAttribute("data-reduced", "true");
  await expect(tour.locator(".lv-tour-anim__field")).toHaveText(
    "Sehr geehrte Frau Berger, vielen Dank für Ihre Nachricht.",
  );
});

test("Installer ohne Diktatmodell zeigt die Modellauswahl", async ({
  page,
}) => {
  await setup(page, { erststart: "[diktat]\r\nmodell=aus\r\n" });
  await page.goto("/");
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
  expect((await commands(page)).some((c) => c.cmd === "download_model")).toBe(
    false,
  );
});

test("Bestehende Installation startet ohne Tour", async ({ page }) => {
  await setup(page, { onboardingCompleted: true });
  await page.goto("/");
  await expect(page.getByTestId("chat-greeting")).toBeVisible();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
});

test("Anderes Modell auswählen führt zur Modellauswahl", async ({ page }) => {
  await setup(page, { downloadFails: 5 });
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await tour.getByRole("button", { name: "Anderes Modell auswählen" }).click();
  await expect(page.getByTestId("intro-tour")).toHaveCount(0);
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
});

// lva#394 C2: „Loslegen“ übergibt an die Führung im Chat. Text und Rahmen
// wechseln gemeinsam, jeder Schritt wird gesprochen, Esc beendet sofort.
test("Loslegen startet die Führung im Chat", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await finishDownload(page);
  await tour.getByRole("button", { name: "Tour starten" }).click();
  for (let i = 0; i < 6; i++) {
    const go = tour.getByRole("button", { name: /^(Weiter|Loslegen)$/ });
    if (!(await go.count())) break;
    const name = await go.innerText();
    await go.click();
    if (name === "Loslegen") break;
  }
  await expect(tour).toHaveCount(0);
  const guide = page.getByTestId("chat-guide");
  await expect(guide).toBeVisible();
  await expect(guide).toHaveAttribute("data-step", "welcome");
  await expect(guide.getByRole("status")).toHaveText("1 von 7");
  await expect(guide.getByRole("button", { name: "Weiter" })).toBeFocused();
  await expect(page.getByTestId("chat-greeting")).toHaveCount(0);

  await guide.getByRole("button", { name: "Weiter" }).click();
  await expect(guide).toHaveAttribute("data-step", "talk");
  await expect(guide).toContainText("wie am Telefon");
  // Rahmen liegt auf dem Gesprächsknopf.
  const ring = page.locator(".lv-guide__ring");
  const call = (await page.getByTestId("call-button").boundingBox())!;
  const box = (await ring.boundingBox())!;
  expect(Math.abs(box.x + 4 - call.x)).toBeLessThan(2);
  expect(Math.abs(box.y + 4 - call.y)).toBeLessThan(2);
  await page.waitForTimeout(1000);
  await page.screenshot({ path: "test-results/chat-guide-talk.png" });
  await expect
    .poll(async () =>
      (await commands(page))
        .filter((c) => c.cmd === "tts_speak_text")
        .map((c) => (c.args as { text: string }).text)
        .some((t) => t.includes("wie am Telefon")),
    )
    .toBe(true);

  await page.keyboard.press("Escape");
  await expect(guide).toHaveCount(0);
  await expect(ring).toHaveCount(0);
  await expect(page.getByTestId("chat-greeting")).toBeVisible();
  expect((await commands(page)).some((c) => c.cmd === "tts_cancel")).toBe(true);
});

test("Führung: letzter Schritt schickt die Geschichte in den Chat", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await setup(page);
  await page.goto("/");
  const tour = page.getByTestId("intro-tour");
  await finishDownload(page);
  await tour.getByRole("button", { name: "Tour starten" }).click();
  for (let i = 0; i < 6; i++) {
    const go = tour.getByRole("button", { name: /^(Weiter|Loslegen)$/ });
    if (!(await go.count())) break;
    const name = await go.innerText();
    await go.click();
    if (name === "Loslegen") break;
  }
  const guide = page.getByTestId("chat-guide");
  await guide.getByRole("switch", { name: "Mit Sprache" }).click();
  for (let i = 0; i < 6; i++)
    await guide.getByRole("button", { name: "Weiter" }).click();
  await expect(guide).toHaveAttribute("data-step", "story");
  // Bewegung reduziert: kein Zeiger.
  await expect(page.locator(".lv-guide__cursor")).toHaveCount(0);
  await page.screenshot({ path: "test-results/chat-guide-story.png" });
  await guide
    .getByRole("textbox", { name: "Thema der Geschichte" })
    .fill("Eine Bäckerin und ein Drache");
  await guide.getByRole("button", { name: "Geschichte erzählen" }).click();
  await expect(guide).toHaveCount(0);
  await expect(
    page.getByText("Erzähl mir eine Geschichte: Eine Bäckerin und ein Drache"),
  ).toBeVisible();
});

// Codex lva#411: Nachweise zu Abbruch, Fokus, Rahmen, Englisch und Neustart.
const openGuide = async (
  page: import("@playwright/test").Page,
  l = { start: "Tour starten", go: /^(Weiter|Loslegen)$/, finish: "Loslegen" },
) => {
  const tour = page.getByTestId("intro-tour");
  await finishDownload(page);
  await tour.getByRole("button", { name: l.start }).click();
  for (let i = 0; i < 6; i++) {
    const go = tour.getByRole("button", { name: l.go });
    if (!(await go.count())) break;
    const name = await go.innerText();
    await go.click();
    if (name === l.finish) break;
  }
  await expect(tour).toHaveCount(0);
  const guide = page.getByTestId("chat-guide");
  await expect(guide).toBeVisible();
  return guide;
};

test("Führung: Esc während Sprache und wartender Sprecherliste schickt nichts", async ({
  page,
}) => {
  await setup(page, { holdSpeakers: true, speakers: ["Anton", "Emma"] });
  await page.goto("/");
  const guide = await openGuide(page);
  for (let i = 0; i < 6; i++)
    await guide.getByRole("button", { name: "Weiter" }).click();
  await expect(guide).toHaveAttribute("data-step", "story");
  await guide.getByRole("button", { name: "Geschichte erzählen" }).click();
  // Sprecherliste hängt noch; Ansage läuft (Sprache an).
  await expect
    .poll(async () =>
      (await commands(page)).some((c) => c.cmd === "tts_provider_speakers"),
    )
    .toBe(true);
  await page.keyboard.press("Escape");
  await expect(guide).toHaveCount(0);
  await expect(page.getByTestId("chat-input")).toBeFocused();
  await page.evaluate(() =>
    (window as unknown as { __releaseSpeakers: () => void }).__releaseSpeakers(),
  );
  await page.waitForTimeout(500);
  await expect(page.getByText(/Erzähl mir eine Geschichte/)).toHaveCount(0);
  expect((await commands(page)).some((c) => c.cmd === "tts_cancel")).toBe(true);
  await expect(page.getByTestId("chat-greeting")).toBeVisible();
});

test("Führung: Überspringen setzt den Fokus in die Eingabe", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  const guide = await openGuide(page);
  await guide.getByRole("button", { name: "Überspringen" }).click();
  await expect(guide).toHaveCount(0);
  await expect(page.getByTestId("chat-input")).toBeFocused();
});

test("Führung: Rahmen sitzt beim Schrittwechsel sofort auf dem Ziel", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  const guide = await openGuide(page);
  const ring = page.locator(".lv-guide__ring");
  const next = guide.getByRole("button", { name: "Weiter" });
  const steps: [string, string][] = [
    ["talk", "[data-testid='call-button']"],
    ["plus", "[data-guide='plus']"],
    ["model", "[data-testid='model-picker']"],
    ["knowledge", "[data-testid='permission-chip']"],
  ];
  for (const [id, sel] of steps) {
    await next.click();
    await expect(guide).toHaveAttribute("data-step", id);
    const target = (await page.locator(sel).first().boundingBox())!;
    // Ohne Übergang: die erste Messung nach dem Wechsel stimmt schon.
    await expect
      .poll(
        async () => {
          const box = await ring.boundingBox();
          return (
            !!box &&
            Math.abs(box.x + 4 - target.x) < 2 &&
            Math.abs(box.y + 4 - target.y) < 2 &&
            Math.abs(box.width - 8 - target.width) < 2
          );
        },
        { timeout: 500 },
      )
      .toBe(true);
  }
});

// Codex lva#411 c: unmittelbar nach dem Klick (ohne Warten) liegt der Rahmen am
// neuen Ziel oder ist weg, wenn der Schritt keins hat.
test("Führung: Rahmen wechselt im selben Bild wie der Text", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  const guide = await openGuide(page);
  const next = guide.getByRole("button", { name: "Weiter" });
  const targets: Record<string, string | null> = {
    talk: "[data-testid='call-button']",
    plus: "[data-guide='plus']",
    model: "[data-testid='model-picker']",
    knowledge: "[data-testid='permission-chip']",
    pages: "[data-tour='nav-primary']",
    story: null,
  };
  for (let i = 0; i < 6; i++) {
    await next.click();
    const r = await page.evaluate((map) => {
      const step = document
        .querySelector("[data-testid='chat-guide']")
        ?.getAttribute("data-step") as string;
      const ring = document.querySelector(".lv-guide__ring");
      const sel = map[step];
      const t = sel ? document.querySelector(sel)?.getBoundingClientRect() : null;
      const b = ring?.getBoundingClientRect() ?? null;
      return {
        step,
        hasTarget: !!sel,
        ring: !!b,
        ok:
          !!b &&
          !!t &&
          Math.abs(b.x + 4 - t.x) < 2 &&
          Math.abs(b.y + 4 - t.y) < 2 &&
          Math.abs(b.width - 8 - t.width) < 2,
      };
    }, targets);
    if (r.hasTarget) expect(r.ok, r.step).toBe(true);
    else expect(r.ring, r.step).toBe(false);
  }
  await expect(guide).toHaveAttribute("data-step", "story");
});

test("Führung auf Englisch bis zur Geschichte", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await setup(page, { language: "en", speakers: ["Anton", "Emma"] });
  await page.goto("/");
  const guide = await openGuide(page, {
    start: "Start tour",
    go: /^(Next|Get started)$/,
    finish: "Get started",
  });
  await expect(guide.getByRole("status")).toHaveText("1 of 7");
  await guide.getByRole("switch", { name: "With voice" }).click();
  for (let i = 0; i < 6; i++)
    await guide.getByRole("button", { name: "Next" }).click();
  await expect(guide).toContainText("several voices");
  await guide.getByRole("button", { name: "Tell the story" }).click();
  await expect(guide).toHaveCount(0);
  await expect(
    page.getByText("Tell me a story: A little dragon who can't sleep"),
  ).toBeVisible();
  const prompt = (await commands(page))
    .map((c) => JSON.stringify(c.args ?? ""))
    .find((a) => a.includes("speaker marker"));
  expect(prompt).toContain("<Anton>");
});

test("Führung erscheint nach Neustart der App nicht von selbst", async ({
  page,
}) => {
  await setup(page, { onboardingCompleted: true });
  await page.goto("/");
  await expect(page.getByTestId("chat-input")).toBeVisible();
  await page.waitForTimeout(800);
  await expect(page.getByTestId("chat-guide")).toHaveCount(0);
  await expect(page.getByTestId("chat-greeting")).toBeVisible();
});
