import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import {
  installProjekteKontextMock,
  installProjekteMock,
} from "./projekteMock";
import { calls, openRecordings, pickMeeting } from "./recLayoutMock";

// lva#365 P4c-3: Recherche mit Projektkontext (W3) und KI-Dokument im Projekt (W4a). Die Regeln des
// Kerns (Halt immer, Kontrolltext, Widerruf, Mindestklasse, Stapel) prüfen die Rust-Tests in
// `src-tauri/src/projekte/tests_p4c3.rs`; hier die Oberfläche gegen die Attrappe. Seit P4c-4 ist die KI im
// Projekt frei (`PROJEKT_KI_FREIGEGEBEN = true`); die Oberfläche kennt keinen Schalter. Nur der Rückbau
// lässt sich in der Attrappe nachstellen (`window.__kiAus`): Dann lehnt das Tor mit `projekt.ki_gesperrt` ab.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(process.cwd(), "docs/vorschau/projekte-p4c3");
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const BILDER = !!process.env.LVA_SCREENSHOTS;
const DUNKEL = process.env.LVA_THEME === "dark";

const aufnehmen = async (page: Page, name: string) => {
  if (!BILDER) return;
  fs.mkdirSync(SHOT_DIR, { recursive: true });
  // Kein Knopf liegt unter dem Zeiger (sonst zeigt das Bild einen Hover-Zustand).
  await page.mouse.move(0, 0);
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(
      SHOT_DIR,
      `${name}${DUNKEL ? "-dunkel" : ""}-${page.viewportSize()?.width}.png`,
    ),
    animations: "disabled",
  });
};

const FRAGE = "Wie werden Netzentgelte für die Lastgangmessung berechnet?";

// ---------------------------------------------------------------- Chat-Spalte: Recherche mit Projektkontext

const chatSetup = async (page: Page, breite: number, hoehe: number) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await installLiveMock(page);
  await installProjekteMock(page);
  if (DUNKEL)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  await openRecordings(page, breite, hoehe);
  // m2 liegt in zwei Ordnern mit je einem Suite-Projekt: die Person wählt das Projekt.
  await pickMeeting(page, "m2");
  await page
    .getByRole(breite < 620 ? "tab" : "button", {
      name: "Fragen",
      exact: true,
    })
    .click();
  return page.getByTestId("chat-panel");
};

for (const [breite, hoehe] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Chat-Spalte: Projektkontext ist wählbar, ohne Sperrtext, und standardmäßig aus (${breite} px)`, async ({
    page,
  }) => {
    const panel = await chatSetup(page, breite, hoehe);
    const schalter = panel.getByTestId("chat-recherche-kontext");
    await expect(schalter).toBeEnabled();
    await expect(schalter).not.toBeChecked();
    await expect(panel.getByText(/Gesperrt/)).toHaveCount(0);
    await expect(
      panel.getByTestId("chat-recherche-kontext-zeile"),
    ).toContainText("Vor der Suche zeigt ein Dialog sie zur Freigabe.");
    await aufnehmen(page, "chat-kontext-aus");
    // Ohne den Haken geht die Recherche wie bisher an das Backend, ohne Kontext.
    const eingabe = panel.locator("textarea");
    await eingabe.fill(FRAGE);
    await panel.getByTestId("chat-recherche").click();
    await panel.getByRole("button", { name: "Kunde Stadtwerke" }).click();
    const start = await calls(page, "recherche_starten");
    expect(start).toHaveLength(1);
    expect(start[0].args.mitKontext ?? null).toBeNull();
  });

  test(`Chat-Spalte: mit Projektkontext hält die Recherche an und zeigt, woher die Anfragen kommen (${breite} px)`, async ({
    page,
  }) => {
    const panel = await chatSetup(page, breite, hoehe);
    const schalter = panel.getByTestId("chat-recherche-kontext");
    await expect(schalter).toBeEnabled();
    await schalter.check();
    await expect(schalter).toBeChecked();
    await aufnehmen(page, "chat-kontext-an");
    const eingabe = panel.locator("textarea");
    await eingabe.fill(FRAGE);
    await panel.getByTestId("chat-recherche").click();
    await panel.getByRole("button", { name: "Geschäftlich" }).click();
    const start = await calls(page, "recherche_starten");
    expect(start).toHaveLength(1);
    expect(start[0].args.suiteProjekt).toBe("sp1");
    expect(start[0].args.mitKontext).toBe(true);
    // Auch ein internes Projekt hält mit Kontext vor der Suche an.
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText(FRAGE);
    await expect(dialog).toContainText(
      "Die Anfragen entstanden mit Inhalten aus Ihrem Projekt",
    );
    // Kopf und Anfragehinweise nennen dieselbe Klasse (das Projekt ist intern).
    await expect(dialog).toContainText("(intern)");
    await expect(
      dialog.getByText("Kontext intern: Freigabe nötig"),
    ).toHaveCount(2);
    await expect(dialog.getByText(/personenbezogen/)).toHaveCount(0);
    await expect(eingabe).toHaveValue("");
    await aufnehmen(page, "freigabe-mit-kontext");
    // Gesendet wird nur, was die Person ankreuzt.
    await dialog.getByRole("checkbox").nth(0).check();
    await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
    await expect(panel.getByTestId("recherche-karte")).toContainText(FRAGE);
    const entscheidung = await calls(page, "recherche_freigabe_entscheiden");
    expect(entscheidung).toHaveLength(1);
    expect(entscheidung[0].args.auswahl).toEqual(["q1"]);
  });
}

test("Chat-Spalte: Der Rückbau des Backends gilt auch, wenn die Oberfläche ihn nicht kennt", async ({
  page,
}) => {
  const panel = await chatSetup(page, 1440, 900);
  await panel.getByTestId("chat-recherche-kontext").check();
  // Das Backend sperrt inzwischen: Der Lauf geht nicht weiter, die Frage bleibt stehen.
  await page.evaluate(() => ((window as any).__kiAus = true));
  const eingabe = panel.locator("textarea");
  await eingabe.fill(FRAGE);
  await panel.getByTestId("chat-recherche").click();
  await panel.getByRole("button", { name: "Geschäftlich" }).click();
  await expect(panel).toContainText(
    "Die KI im Projekt ist in dieser Version abgeschaltet. Es wurde nichts gesendet.",
  );
  await expect(eingabe).toHaveValue(FRAGE);
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
});

// ---------------------------------------------------------------- Spalte „KI im Projekt“: KI-Dokument

const dokSetup = async (
  page: Page,
  opts: { width?: number; height?: number } = {},
) => {
  const { width = 1440, height = 900 } = opts;
  page.on("pageerror", (error) => {
    throw error;
  });
  await installLiveMock(page);
  await installProjekteKontextMock(page);
  if (DUNKEL)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  await page.setViewportSize({ width, height });
  await page.goto("/");
  const nav = page.getByRole("button", { name: "Projekte", exact: true });
  if (!(await nav.isVisible())) {
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await nav.click();
  await page
    .getByTestId("projekt-zeile")
    .filter({ hasText: "Finanzwissen" })
    .first()
    .click();
};

/** Der Knopf steht in der rechten Spalte; schmal (< 999 px) im Dialog „Details“. */
const dokKnopf = async (page: Page) => {
  const knopf = page
    .getByTestId("projekt-dokument-erstellen")
    .filter({ visible: true });
  if ((await knopf.count()) === 0)
    await page.getByRole("button", { name: "Details", exact: true }).click();
  await expect(knopf.first()).toBeVisible();
  return knopf.first();
};

const bereich = (page: Page) =>
  page.getByTestId("ki-im-projekt").filter({ visible: true });

for (const [breite, hoehe] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`KI-Dokument: Der Knopf ist aktiv, ohne Sperrtext, und das Formular öffnet erst auf Klick (${breite} px)`, async ({
    page,
  }) => {
    await dokSetup(page, { width: breite, height: hoehe });
    const knopf = await dokKnopf(page);
    await expect(knopf).toBeEnabled();
    await expect(knopf).toHaveAttribute("aria-expanded", "false");
    await expect(bereich(page).getByText(/Gesperrt/)).toHaveCount(0);
    await expect(page.getByTestId("projekt-dokument-formular")).toHaveCount(0);
    await knopf.scrollIntoViewIfNeeded();
    await aufnehmen(page, "dokument-knopf");
    expect(await calls(page, "projekt_ki_dokument_erstellen")).toHaveLength(0);
  });

  test(`KI-Dokument: Auftrag und Name, Stand der Übersicht, Entwurf bleibt im Projekt aus (${breite} px)`, async ({
    page,
  }) => {
    await dokSetup(page, { width: breite, height: hoehe });
    const knopf = await dokKnopf(page);
    await expect(knopf).toBeEnabled();
    await knopf.click();
    const formular = page.getByTestId("projekt-dokument-formular");
    await expect(formular).toBeVisible();
    const senden = formular.getByTestId("projekt-dokument-senden");
    await expect(senden).toBeDisabled();
    await formular
      .getByTestId("projekt-dokument-auftrag")
      .fill("Fasse die Auswahl im Projekt in einer Seite zusammen.");
    await formular.getByTestId("projekt-dokument-name").fill("Zusammenfassung");
    await expect(senden).toBeEnabled();
    // Senden und Abbrechen sind Touchziele; bei 390 px stehen sie in einer Zeile.
    const abbrechen = formular.getByRole("button", { name: "Abbrechen" });
    await senden.scrollIntoViewIfNeeded();
    const [bs, ba] = [
      await senden.boundingBox(),
      await abbrechen.boundingBox(),
    ];
    expect(bs && ba).toBeTruthy();
    // Die schmale Spalte bei 1440 px darf umbrechen; der Dialog bei 390 px hat Platz für eine Zeile.
    if (breite === 390) expect(Math.abs(bs!.y - ba!.y)).toBeLessThan(2);
    expect(ba!.height).toBeGreaterThanOrEqual(44);
    await senden.scrollIntoViewIfNeeded();
    await aufnehmen(page, "dokument-formular");
    await senden.click();
    const fertig = page.getByTestId("projekt-dokument-fertig");
    await expect(fertig).toContainText("Zusammenfassung.md");
    await expect(fertig).toContainText("ausgeschaltet");
    await expect(formular).toHaveCount(0);
    const a = await calls(page, "projekt_ki_dokument_erstellen");
    expect(a).toHaveLength(1);
    // Klasse und Quellen gibt die Oberfläche nicht vor; sie kennt nur Auftrag, Name und den Stand.
    expect(Object.keys(a[0].args).sort()).toEqual(
      ["auftrag", "expectedStand", "name", "ordner", "suiteId"].sort(),
    );
    expect(a[0].args.suiteId).toBe("p-fin");
    expect(a[0].args.name).toBe("Zusammenfassung");
    expect(a[0].args.ordner).toBeNull();
    expect(typeof a[0].args.expectedStand).toBe("string");
    await fertig.scrollIntoViewIfNeeded();
    await aufnehmen(page, "dokument-fertig");
  });
}

test("KI-Dokument: Fehler des Backends erscheinen mit festem Text, ohne Inhalte", async ({
  page,
}) => {
  await dokSetup(page);
  await page.evaluate(
    () =>
      ((window as any).__kontext.dokumentFehler =
        "projekt.dokument_leer: Das Modell hat keinen Text geliefert."),
  );
  await (await dokKnopf(page)).click();
  const formular = page.getByTestId("projekt-dokument-formular");
  await formular
    .getByTestId("projekt-dokument-auftrag")
    .fill("Schreibe etwas.");
  await formular.getByTestId("projekt-dokument-name").fill("Notiz");
  await formular.getByTestId("projekt-dokument-senden").click();
  await expect(page.getByTestId("projekt-dokument-fehler")).toContainText(
    "Das Modell hat keinen Text geliefert. Es wurde kein Entwurf angelegt.",
  );
  // Das Formular bleibt offen: Die Eingaben sind nicht verloren.
  await expect(formular.getByTestId("projekt-dokument-auftrag")).toHaveValue(
    "Schreibe etwas.",
  );
  // Ein Code des Versandtors wird ebenfalls übersetzt.
  await page.evaluate(
    () =>
      ((window as any).__kontext.dokumentFehler =
        "projekt.klasse_ziel: roher Text"),
  );
  await formular.getByTestId("projekt-dokument-senden").click();
  await expect(page.getByTestId("projekt-dokument-fehler")).toContainText(
    "Ein Projektinhalt ist für dieses Modell zu schutzbedürftig.",
  );
});

test("KI-Dokument: Mitgliedschaft gescheitert wird gemeldet, der Entwurf bleibt in der Ablage", async ({
  page,
}) => {
  await dokSetup(page);
  await page.evaluate(
    () => ((window as any).__kontext.dokumentImProjekt = false),
  );
  await (await dokKnopf(page)).click();
  const formular = page.getByTestId("projekt-dokument-formular");
  await formular
    .getByTestId("projekt-dokument-auftrag")
    .fill("Schreibe etwas.");
  await formular.getByTestId("projekt-dokument-name").fill("Notiz");
  await formular.getByTestId("projekt-dokument-senden").click();
  await expect(page.getByTestId("projekt-dokument-fertig")).toContainText(
    "ließ sich aber dem Projekt nicht zuordnen",
  );
});

test("KI-Dokument: Doppelklick sendet einmal, während der Lauf dauert", async ({
  page,
}) => {
  await dokSetup(page);
  await page.evaluate(() => ((window as any).__kontext.dokumentMs = 400));
  await (await dokKnopf(page)).click();
  const formular = page.getByTestId("projekt-dokument-formular");
  await formular
    .getByTestId("projekt-dokument-auftrag")
    .fill("Schreibe etwas.");
  await formular.getByTestId("projekt-dokument-name").fill("Notiz");
  const senden = formular.getByTestId("projekt-dokument-senden");
  await senden.dblclick();
  await expect(page.getByTestId("projekt-dokument-fertig")).toBeVisible();
  expect(await calls(page, "projekt_ki_dokument_erstellen")).toHaveLength(1);
});

test("Barrierefreiheit: Formular KI-Dokument und Schalter ohne serious/critical", async ({
  page,
}) => {
  await dokSetup(page);
  await (await dokKnopf(page)).click();
  await expect(page.getByTestId("projekt-dokument-formular")).toBeVisible();
  const r = await new AxeBuilder({ page })
    .include('[data-testid="ki-im-projekt"]')
    .analyze();
  const schwer = r.violations.filter((v) =>
    ["serious", "critical"].includes(v.impact ?? ""),
  );
  expect(schwer.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});

test("390 px: Touchziele des KI-Dokuments mindestens 44 px", async ({
  page,
}) => {
  await dokSetup(page, { width: 390, height: 844 });
  await (await dokKnopf(page)).click();
  const formular = page.getByTestId("projekt-dokument-formular");
  await expect(formular).toBeVisible();
  const ziele = [
    page.getByTestId("projekt-dokument-erstellen").filter({ visible: true }),
    formular.getByTestId("projekt-dokument-auftrag"),
    formular.getByTestId("projekt-dokument-name"),
    formular.getByTestId("projekt-dokument-senden"),
    formular.getByRole("button", { name: "Abbrechen" }),
  ];
  for (const z of ziele) {
    const box = await z.first().boundingBox();
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
  }
  // Keine waagerechte Rollleiste der Seite.
  const waagerecht = await page.evaluate(
    () =>
      document.documentElement.scrollWidth >
      document.documentElement.clientWidth,
  );
  expect(waagerecht).toBe(false);
});
