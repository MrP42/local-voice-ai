import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import {
  installGespraechMock,
  installProjekteKontextMock,
} from "./projekteMock";
import { chatSeitenleiste, raumOeffnen } from "./projektRaumHelfer";

// lva#826: Projekte als Räume im Chat (Start). Seitenleiste „Projekte“ über dem Verlauf, Projektraum
// mit Frage, Gesprächen und Quellen; der Menüpunkt „Projekte“ entfällt.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "docs/vorschau/projekt-raeume/umsetzung",
);

const jetzt = new Date("2026-11-25T09:00:00+01:00").getTime();
const iso = (minuten: number) =>
  new Date(jetzt - minuten * 60000).toISOString();

const setup = async (page: Page, width = 1440, height = 900, dark = false) => {
  await page.addInitScript(
    ([a, b, c, d]) => {
      (window as any).__seedConvs = [
        {
          id: "conv_a",
          title: "Haushalt 2027 grob planen",
          updated_at: a,
          last_text: "Die Rücklage reicht für drei Jahre.",
          scope: { project_id: "p-fin" },
        },
        {
          id: "conv_b",
          title: "Kennzahlen Kämmerei",
          updated_at: b,
          last_text: "Liquidität 1. Grades liegt bei 1,4.",
          scope: { project_id: "p-fin" },
        },
        {
          id: "conv_c",
          title: "Wetter am Wochenende",
          updated_at: c,
          last_text: "",
          scope: null,
        },
        {
          id: "conv_d",
          title: "Use Cases priorisieren",
          updated_at: d,
          last_text: "",
          scope: { project_id: "p-ki" },
        },
      ];
    },
    [iso(30), iso(60 * 26), iso(10), iso(60 * 50)],
  );
  await installLiveMock(page);
  await installProjekteKontextMock(page);
  await installGespraechMock(page);
  if (dark)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  await page.setViewportSize({ width, height });
  await page.goto("/");
};

/** Schmal liegt die Seitenleiste des Chats in einer Schublade. */
const seitenleiste = async (page: Page) => {
  // Das Symbol „Gesprächsverlauf“ steht nur da, solange die Spalte zu ist (schmal: Schublade).
  await expect(page.getByTestId("chat-input")).toBeVisible();
  const auf = page.locator(
    'button[aria-label="Gesprächsverlauf"][aria-expanded="false"]',
  );
  if ((await auf.count()) > 0) await auf.first().click();
  await expect(page.getByTestId("projekt-zeile").first()).toBeVisible();
};

const raum = (page: Page) => page.getByTestId("projekt-raum");

const aufnehmen = async (page: Page, name: string) => {
  fs.mkdirSync(SHOT_DIR, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(SHOT_DIR, `${name}.png`) });
};

test.describe("Projekte als Räume im Chat", () => {
  test("Menüpunkt Projekte entfällt, Projekte stehen im Chat", async ({
    page,
  }) => {
    await setup(page);
    await expect(
      page.getByRole("button", { name: "Projekte", exact: true }),
    ).toHaveCount(0);
    await seitenleiste(page);
    const zeilen = page.getByTestId("projekt-zeile");
    await expect(zeilen).toHaveCount(4);
    await expect(zeilen.first()).toContainText("Finanzwissen");
    await expect(zeilen.first().getByTestId("projekt-zahl")).toHaveText("9");
    // Projektgespräche stehen nicht im allgemeinen Verlauf.
    await expect(page.getByText("Wetter am Wochenende")).toBeVisible();
    await expect(page.getByText("Kennzahlen Kämmerei")).toHaveCount(0);
    // Aufklappen zeigt die Gespräche des Projekts.
    await page
      .getByRole("button", { name: "Gespräche von Finanzwissen zeigen" })
      .click();
    await expect(page.getByTestId("projekt-raum-gespraech")).toHaveCount(2);
  });

  test("Projektraum: Kopf, Gespräche, Quellen; erste Frage beginnt das Gespräch im Projekt", async ({
    page,
  }) => {
    await setup(page);
    await seitenleiste(page);
    await page.getByTestId("projekt-zeile").first().click();
    await expect(raum(page)).toBeVisible();
    await expect(page.getByTestId("projekt-raum-titel")).toHaveText(
      "Finanzwissen",
    );
    await expect(page.getByTestId("projekt-raum-eintrag")).toHaveCount(2);
    await expect(page.getByTestId("projekte-bereich")).toBeVisible();
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
    // Genau ein h1 auf der Seite, keins im Raum.
    await expect(page.locator("h1")).toHaveCount(1);
    await expect(raum(page).locator("h1")).toHaveCount(0);

    await page
      .getByTestId("projekt-raum-eingabe")
      .fill("Wie hoch ist die Rücklage?");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await expect(raum(page)).toHaveCount(0);
    await expect(page.getByTestId("projekt-chip")).toBeVisible();
    const angelegt = await page.evaluate(() =>
      (window as any).__calls.filter(
        (c: any) => c.cmd === "conversation_create",
      ),
    );
    expect(angelegt.at(-1).args.projectId).toBe("p-fin");
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__calls.some((c: any) => c.cmd === "voice_call_chat"),
        ),
      )
      .toBe(true);
  });

  test("Chip im Projektgespräch öffnet den Raum, Schließen kehrt zum Chat zurück", async ({
    page,
  }) => {
    await setup(page);
    await seitenleiste(page);
    await page
      .getByRole("button", { name: "Gespräche von Finanzwissen zeigen" })
      .click();
    await page.getByTestId("projekt-raum-gespraech").first().click();
    await expect(page.getByTestId("projekt-chip")).toBeVisible();
    await page.getByTestId("projekt-chip").click();
    await expect(raum(page)).toBeVisible();
    await page.getByTestId("projekt-raum-schliessen").click();
    await expect(raum(page)).toHaveCount(0);
    await expect(page.getByTestId("projekt-chip")).toBeVisible();
  });

  test("Neues Projekt aus der Seitenleiste öffnet seinen Raum", async ({
    page,
  }) => {
    await setup(page);
    await seitenleiste(page);
    await page.getByTestId("projekt-neu").click();
    await page.getByRole("textbox", { name: "Name" }).fill("Bauamt");
    await page.getByTestId("projekt-anlegen").click();
    await expect(
      page.evaluate(() =>
        (window as any).__calls.some((c: any) => c.cmd === "projekt_anlegen"),
      ),
    ).resolves.toBe(true);
  });

  test("Entwurf bleibt im Raum: nach Wechsel zu B ist das Feld leer, gesendet wird in B", async ({
    page,
  }) => {
    await setup(page);
    await raumOeffnen(page, "Finanzwissen");
    await page.getByTestId("projekt-raum-eingabe").fill("Frage für A");
    await raumOeffnen(page, "KI-Strategie LRA");
    await expect(page.getByTestId("projekt-raum-titel")).toHaveText(
      "KI-Strategie LRA",
    );
    await expect(page.getByTestId("projekt-raum-eingabe")).toHaveValue("");
    await page.getByTestId("projekt-raum-eingabe").fill("Frage für B");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await expect(page.getByTestId("projekt-chip")).toBeVisible();
    const angelegt = await page.evaluate(() =>
      (window as any).__calls.filter(
        (c: any) => c.cmd === "conversation_create",
      ),
    );
    expect(angelegt.map((c: any) => c.args.projectId)).toEqual(["p-ki"]);
  });

  test("Überholt: Neues Gespräch während der ersten Frage legt nichts im Projekt an", async ({
    page,
  }) => {
    await setup(page);
    await raumOeffnen(page, "Finanzwissen");
    await page.evaluate(() => {
      const w = window as any;
      w.__uebersichtHalt = new Promise<void>((r) => (w.__weiter = r));
    });
    await page.getByTestId("projekt-raum-eingabe").fill("Späte Frage");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await page.getByRole("button", { name: "Neues Gespräch" }).first().click();
    await expect(page.getByTestId("projekt-raum")).toHaveCount(0);
    await page.evaluate(() => (window as any).__weiter());
    await page.waitForTimeout(500);
    const calls = await page.evaluate(() =>
      (window as any).__calls.map((c: any) => c.cmd),
    );
    expect(calls).not.toContain("conversation_create");
    expect(calls).not.toContain("voice_call_chat");
  });

  test("Überholt: Seite verlassen, während die Übersicht lädt, legt nichts an", async ({
    page,
  }) => {
    await setup(page);
    await raumOeffnen(page, "Finanzwissen");
    await page.evaluate(() => {
      const w = window as any;
      w.__uebersichtHalt = new Promise<void>((r) => (w.__weiter = r));
    });
    await page.getByTestId("projekt-raum-eingabe").fill("Späte Frage");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
    await page.evaluate(() => (window as any).__weiter());
    await page.waitForTimeout(500);
    const calls = await page.evaluate(() =>
      (window as any).__calls.map((c: any) => c.cmd),
    );
    expect(calls).not.toContain("conversation_create");
    expect(calls).not.toContain("voice_call_chat");
  });

  test("Überholt: Raum B öffnen, während das Gespräch in A angelegt wird", async ({
    page,
  }) => {
    await setup(page);
    await raumOeffnen(page, "Finanzwissen");
    await page.evaluate(() => {
      const w = window as any;
      w.__createHalt = new Promise<void>((r) => (w.__weiter = r));
    });
    await page.getByTestId("projekt-raum-eingabe").fill("Frage für A");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as any).__calls.some(
            (c: any) => c.cmd === "conversation_create",
          ),
        ),
      )
      .toBe(true);
    await raumOeffnen(page, "KI-Strategie LRA");
    await page.getByTestId("projekt-raum-eingabe").fill("Entwurf für B");
    await page.evaluate(() => (window as any).__weiter());
    await page.waitForTimeout(500);
    // B bleibt offen mit seinem Entwurf; A wird nicht gesendet und das leere A-Gespräch gelöscht.
    await expect(page.getByTestId("projekt-raum-titel")).toHaveText(
      "KI-Strategie LRA",
    );
    await expect(page.getByTestId("projekt-raum-eingabe")).toHaveValue(
      "Entwurf für B",
    );
    const calls = await page.evaluate(() => (window as any).__calls);
    expect(calls.map((c: any) => c.cmd)).not.toContain("voice_call_chat");
    expect(
      calls.filter((c: any) => c.cmd === "conversation_delete"),
    ).toHaveLength(1);
  });

  test("Anlegen scheitert: Meldung im Raum, die Frage geht nirgendwo hin", async ({
    page,
  }) => {
    await setup(page);
    await raumOeffnen(page, "Finanzwissen");
    await page.evaluate(() => {
      (window as any).__createFehler = "Das Projekt ist archiviert.";
    });
    await page.getByTestId("projekt-raum-eingabe").fill("Frage");
    await page.getByTestId("projekt-raum-eingabe").press("Enter");
    await expect(
      page.getByTestId("projekt-raum").getByRole("alert"),
    ).toBeVisible();
    await expect(page.getByTestId("projekt-raum-eingabe")).toHaveValue("Frage");
    const calls = await page.evaluate(() =>
      (window as any).__calls.map((c: any) => c.cmd),
    );
    expect(calls).not.toContain("voice_call_chat");
  });

  test("Altes Sprungziel mit Projekt öffnet den Raum genau dieses Projekts", async ({
    page,
  }) => {
    await setup(page);
    await chatSeitenleiste(page);
    await page.evaluate(() =>
      window.dispatchEvent(
        new CustomEvent("lv-navigate", {
          detail: { section: "projekte", projektId: "p-post" },
        }),
      ),
    );
    await expect(page.getByTestId("projekt-raum-titel")).toHaveText(
      "Digitaler Posteingang",
    );
  });

  for (const [w, h] of [
    [1440, 900],
    [390, 844],
  ] as const)
    for (const dark of [false, true])
      test(`Vorschau ${w} ${dark ? "dunkel" : "hell"}`, async ({ page }) => {
        const art = dark ? "dunkel" : "hell";
        await setup(page, w, h, dark);
        await seitenleiste(page);
        await page
          .getByRole("button", { name: "Gespräche von Finanzwissen zeigen" })
          .click();
        await aufnehmen(page, `1-seitenleiste-${w}-${art}`);
        await page.getByTestId("projekt-zeile").first().click();
        await expect(raum(page)).toBeVisible();
        await aufnehmen(page, `2-raum-${w}-${art}`);
      });
});
