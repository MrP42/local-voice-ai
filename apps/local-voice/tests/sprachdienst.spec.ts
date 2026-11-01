import { test, expect, type Page } from "@playwright/test";
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { calls, installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";
import {
  ENDPUNKT,
  installSprachdienstMock,
  type SprachdienstMockOptions,
} from "./sprachdienstMock";
import { geheZu } from "./profilmenue";

// ADR-0021 (S2): der Sprachdienst auf der Seite Integrationen (Opt-in, gekoppelte Apps mit
// Rechten, Widerruf) und der globale Kopplungsdialog (Code, Frist, Bestätigen/Ablehnen).
// Gegen die Tauri-Attrappe; Handschlag, Frist, Ratenbremse und Rechte prüfen die Rust-Tests
// (`cargo test --lib sprachdienst::`). Bild des Dialogs nur mit LVA_SCREENS=1.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (page: Page, options: SprachdienstMockOptions = {}) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, { integrations: [] });
  await installSprachdienstMock(page, NOW, options);
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.goto("/");
};

const openIntegrations = async (page: Page) => {
  await geheZu(page, "Integrationen");
  await expect(page.getByTestId("sprachdienst")).toBeVisible();
  await page.getByTestId("sprachdienst").scrollIntoViewIfNeeded();
};

const APPS = [
  {
    zugang: "z-1",
    anzeige: "Corporate LLM",
    hoeren: true,
    sprechen: false,
    zuletzt_ms: NOW - 1_800_000,
  },
  {
    zugang: "z-2",
    anzeige: "Altes Skript",
    widerrufen_ms: NOW - 3_600_000,
  },
];

test("der Dienst ist standardmäßig aus und lässt sich einschalten", async ({
  page,
}) => {
  await setup(page);
  await openIntegrations(page);
  const schalter = page.getByTestId("sprachdienst-switch");
  await expect(schalter).not.toBeChecked();
  await expect(page.getByTestId("sprachdienst-status")).toContainText(
    "Der Sprachdienst ist aus",
  );
  await expect(page.getByTestId("sprachdienst-empty")).toBeVisible();

  await schalter.click();
  await expect(schalter).toBeChecked();
  await expect(page.getByTestId("sprachdienst-status")).toContainText(
    "Der Sprachdienst läuft",
  );
  await expect(page.getByTestId("sprachdienst-status")).toContainText(ENDPUNKT);
  const set = await calls(page, "sprachdienst_aktiv_setzen");
  expect(set.map((c) => c.args)).toEqual([{ aktiv: true }]);
});

test("ein Startfehler steht im Status", async ({ page }) => {
  await setup(page, {
    aktiv: true,
    fehler: "Es gibt keinen sicheren Laufzeitordner.",
  });
  await openIntegrations(page);
  await expect(page.getByTestId("sprachdienst-status")).toContainText(
    "Der Sprachdienst läuft nicht",
  );
  await expect(page.getByTestId("sprachdienst-error")).toContainText(
    "Laufzeitordner",
  );
});

test("gekoppelte Apps: Rechte setzen, widerrufen, entfernen", async ({
  page,
}) => {
  await setup(page, { aktiv: true, zugaenge: APPS });
  await openIntegrations(page);
  const app = page.locator(
    '[data-testid="sprachdienst-zugang"][data-zugang="z-1"]',
  );
  await expect(app).toContainText("Corporate LLM");
  await expect(app.getByTestId("sprachdienst-recht-hoeren")).toBeChecked();
  await expect(
    app.getByTestId("sprachdienst-recht-sprechen"),
  ).not.toBeChecked();

  await app.getByTestId("sprachdienst-recht-sprechen").check();
  await expect(app.getByTestId("sprachdienst-recht-sprechen")).toBeChecked();
  const rechte = await calls(page, "sprachdienst_recht_setzen");
  expect(rechte.map((c) => c.args)).toEqual([
    { zugang: "z-1", recht: "sprechen", an: true },
  ]);

  await app.getByTestId("sprachdienst-revoke").click();
  await expect(app.getByTestId("sprachdienst-confirm")).toContainText(
    "Corporate LLM widerrufen?",
  );
  await app.getByTestId("sprachdienst-confirm-yes").click();
  await expect(app).toHaveAttribute("data-revoked", "true");
  await expect(app.getByTestId("sprachdienst-zugang-status")).toHaveText(
    "Widerrufen",
  );
  await expect(app.getByTestId("sprachdienst-rechte")).toHaveCount(0);

  const alt = page.locator(
    '[data-testid="sprachdienst-zugang"][data-zugang="z-2"]',
  );
  await alt.getByTestId("sprachdienst-delete").click();
  await alt.getByTestId("sprachdienst-confirm-yes").click();
  await expect(alt).toHaveCount(0);
});

test("Kopplungsdialog: Code vergleichen und bestätigen", async ({ page }) => {
  await setup(page, {
    kopplungen: [{ id: "K-1", anzeige: "Corporate LLM", code: "123456" }],
  });
  await expect(page.getByTestId("kopplung-dialog")).toHaveCount(0);

  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  const dialog = page.getByTestId("kopplung-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Corporate LLM");
  await expect(page.getByTestId("kopplung-code")).toHaveText("123 456");
  await expect(page.getByTestId("kopplung-restzeit")).toContainText("100 s");

  await page.getByTestId("kopplung-confirm").click();
  await expect(page.getByTestId("kopplung-dialog")).toHaveCount(0);
  const entscheidungen = await calls(page, "sprachdienst_kopplung_entscheiden");
  expect(entscheidungen.map((c) => c.args)).toEqual([
    { id: "K-1", bestaetigen: true },
  ]);
});

test("Kopplungsdialog: Ablehnen koppelt nichts", async ({ page }) => {
  await setup(page, {
    kopplungen: [
      { id: "K-1", anzeige: "Fremde App", code: "000111" },
      { id: "K-2", anzeige: "Zweite App", code: "222333" },
    ],
  });
  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  await expect(page.getByTestId("kopplung-weitere")).toContainText("1 weitere");
  await page.getByTestId("kopplung-reject").click();
  await expect(page.getByTestId("kopplung-code")).toHaveText("222 333");
  await page.getByTestId("kopplung-reject").click();
  await expect(page.getByTestId("kopplung-dialog")).toHaveCount(0);
  const z = await page.evaluate(() => (window as any).__sprache.zugaenge);
  expect(z).toEqual([]);
});

test("Kopplungsdialog zeigt die Kennung, den Namen in <bdi> und sperrt Koppeln kurz", async ({
  page,
}) => {
  await setup(page, {
    kopplungen: [
      {
        id: "K-1",
        client: "corporate-llm",
        anzeige: "Corporate LLM",
        code: "123456",
      },
    ],
  });
  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  await expect(page.getByTestId("kopplung-client")).toHaveText("corporate-llm");
  await expect(page.getByTestId("kopplung-dialog").locator("bdi")).toHaveText(
    "Corporate LLM",
  );
  await expect(page.getByTestId("kopplung-warnungen")).toHaveCount(0);
  // Erst nach der Sperre lässt sich bestätigen.
  await expect(page.getByTestId("kopplung-confirm")).toBeDisabled();
  await expect(page.getByTestId("kopplung-confirm")).toBeEnabled();
});

test("Kopplungsdialog warnt bei doppelten Namen oder Kennungen", async ({
  page,
}) => {
  await setup(page, {
    zugaenge: [
      {
        zugang: "z-1",
        anzeige: "Corporate LLM",
        client: "corporate-llm",
        hoeren: true,
      },
    ],
    kopplungen: [
      {
        id: "K-1",
        client: "corporate-llm",
        anzeige: "Meine Sprach-App",
        code: "123456",
      },
    ],
  });
  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  const w = page.getByTestId("kopplung-warnungen");
  await expect(w.locator('[data-warnung="duplicateClient"]')).toBeVisible();
  await expect(w.locator('[data-warnung="duplicateName"]')).toHaveCount(0);
  await page.getByTestId("kopplung-reject").click();

  await page.evaluate(() => {
    const st = (window as any).__sprache;
    st.kopplungen.push({
      id: "K-2",
      client: "andere-app",
      anzeige: "corporate llm",
      code: "654321",
      bis_ms: st.zugaenge[0].erstellt_ms + 200_000_000,
    });
    (window as any).__emit("sprachdienst-kopplung", null);
  });
  await expect(
    page
      .getByTestId("kopplung-warnungen")
      .locator('[data-warnung="duplicateName"]'),
  ).toBeVisible();
});

test("Kopplungsdialog bindet an die gezeigte Anfrage, auch wenn die Liste sich ändert", async ({
  page,
}) => {
  await setup(page, {
    kopplungen: [{ id: "K-1", anzeige: "Erste App", code: "111111" }],
  });
  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  await expect(page.getByTestId("kopplung-dialog")).toHaveAttribute(
    "data-kopplung-id",
    "K-1",
  );
  await expect(page.getByTestId("kopplung-confirm")).toBeEnabled();
  // Eine neue Anfrage drängt sich an die Spitze der Liste.
  await page.evaluate(() => {
    const st = (window as any).__sprache;
    st.kopplungen.unshift({
      id: "K-0",
      client: "dreist",
      anzeige: "Dreiste App",
      code: "000000",
      bis_ms: st.kopplungen[0].bis_ms,
    });
    (window as any).__emit("sprachdienst-kopplung", null);
  });
  await expect(page.getByTestId("kopplung-weitere")).toBeVisible();
  await expect(page.getByTestId("kopplung-dialog")).toHaveAttribute(
    "data-kopplung-id",
    "K-1",
  );
  await expect(page.getByTestId("kopplung-code")).toHaveText("111 111");
  await page.getByTestId("kopplung-confirm").click();
  const entscheidungen = await calls(page, "sprachdienst_kopplung_entscheiden");
  expect(entscheidungen.map((c) => c.args)).toEqual([
    { id: "K-1", bestaetigen: true },
  ]);
  // Die nächste Anfrage erscheint; Koppeln ist erst nach der Sperre möglich.
  await expect(page.getByTestId("kopplung-dialog")).toHaveAttribute(
    "data-kopplung-id",
    "K-0",
  );
  await expect(page.getByTestId("kopplung-confirm")).toBeDisabled();
  await expect(page.getByTestId("kopplung-confirm")).toBeEnabled();
});

test("Bild: Kopplungsdialog", async ({ page }) => {
  test.skip(!process.env.LVA_SCREENS, "Bild nur mit LVA_SCREENS=1");
  const out =
    process.env.LVA_SCREENS_DIR ?? "/mnt/project-files/sprachgespraech/s2";
  if (!existsSync(out)) mkdirSync(out, { recursive: true });
  await setup(page, {
    zugaenge: [
      {
        zugang: "z-1",
        anzeige: "Corporate LLM",
        client: "corporate-llm",
        hoeren: true,
      },
    ],
    kopplungen: [
      {
        id: "K-1",
        client: "corporate-llm",
        anzeige: "Corporate LLM",
        code: "482913",
      },
    ],
  });
  await page.evaluate(() =>
    (window as any).__emit("sprachdienst-kopplung", null),
  );
  await expect(page.getByTestId("kopplung-dialog")).toBeVisible();
  await expect(page.getByTestId("kopplung-confirm")).toBeEnabled();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: resolve(out, "kopplungsdialog.png"),
    animations: "disabled",
  });
});
