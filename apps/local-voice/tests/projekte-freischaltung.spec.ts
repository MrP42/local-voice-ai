import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import {
  installGespraechMock,
  installProjekteKontextMock,
  installProjekteMock,
} from "./projekteMock";
import { calls, openRecordings, pickMeeting } from "./recLayoutMock";

// lva#365 P4c-4: Freischaltung der KI im Projekt. Die Abnahme der Regeln (Kontrolltext, Kurzbuch,
// Widerruf, Budget, Klasse) leisten die Rust-Tests; hier der Weg der Person über die Oberfläche, je Weg
// vom Start bis zur Antwort gegen die Attrappe: Gespräch im Projekt (Chat und Sprache laufen über
// denselben Sendeweg `voice_call_chat`), KI-Dokument, Recherche mit Projektkontext. Der Stapel hat keine
// Oberfläche (Rust-Tests). Die Oberfläche kennt keinen Schalter und fragt nichts ab; nur der Rückbau
// (`PROJEKT_KI_FREIGEGEBEN = false`) lässt sich in der Attrappe nachstellen (`window.__kiAus`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(process.cwd(), "docs/vorschau/projekte-p4c4");
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
      `${name}-${DUNKEL ? "dunkel" : "hell"}-${page.viewportSize()?.width}.png`,
    ),
    animations: "disabled",
  });
};

const FRAGE = "Wie werden Netzentgelte für die Lastgangmessung berechnet?";
/** Was nach der Freischaltung nirgends mehr stehen darf. */
const SPERRTEXT =
  /Gesperrt|noch nicht freigegeben|folgen mit dem nächsten Schritt/;

const BREITEN = [
  [1440, 900],
  [390, 844],
] as const;

// ---------------------------------------------------------------- Projektseite: Gespräch und KI-Dokument

const projektSetup = async (page: Page, breite: number, hoehe: number) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await installLiveMock(page);
  await installProjekteKontextMock(page);
  await installGespraechMock(page);
  if (DUNKEL)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  await page.setViewportSize({ width: breite, height: hoehe });
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

/** Der Bereich steht in der rechten Spalte; schmal (< 999 px) im Dialog „Details“. */
const sichtbar = async (page: Page, testId: string) => {
  const ziel = page.getByTestId(testId).filter({ visible: true });
  if ((await ziel.count()) === 0)
    await page.getByRole("button", { name: "Details", exact: true }).click();
  await expect(ziel.first()).toBeVisible();
  return ziel.first();
};

const kiBereich = (page: Page) =>
  page.getByTestId("ki-im-projekt").filter({ visible: true });

const nachricht = (page: Page) =>
  page.getByRole("textbox", { name: "Nachricht" });

for (const [breite, hoehe] of BREITEN) {
  test(`Projektseite: alle Einstiege sind aktiv und nennen keinen Sperrgrund (${breite} px)`, async ({
    page,
  }) => {
    await projektSetup(page, breite, hoehe);
    const gespraech = await sichtbar(page, "projekt-gespraech-beginnen");
    const dokument = await sichtbar(page, "projekt-dokument-erstellen");
    await expect(gespraech).toBeEnabled();
    await expect(dokument).toBeEnabled();
    const bereich = kiBereich(page);
    await expect(bereich).not.toContainText(SPERRTEXT);
    await expect(bereich).toContainText("KI im Projekt");
    await expect(bereich).toContainText("Gespräch im Projekt beginnen");
    await expect(bereich).toContainText("KI-Dokument erstellen");
    await expect(page.locator("body")).not.toContainText(SPERRTEXT);
    await bereich.scrollIntoViewIfNeeded();
    await aufnehmen(page, "01-projektseite-ki-aktiv");
    // Das Anschauen sendet nichts.
    expect(await calls(page, "conversation_create")).toHaveLength(0);
    expect(await calls(page, "voice_call_chat")).toHaveLength(0);
    expect(await calls(page, "projekt_ki_dokument_erstellen")).toHaveLength(0);
  });

  test(`Gespräch im Projekt: Start, Frage mit gesehenem Stand, freigegebene Antwort (${breite} px)`, async ({
    page,
  }) => {
    await projektSetup(page, breite, hoehe);
    await (await sichtbar(page, "projekt-gespraech-beginnen")).click();
    await expect(page.getByTestId("projekt-chip")).toContainText(
      "Projekt: Finanzwissen",
    );
    const erzeugt = await calls(page, "conversation_create");
    expect(erzeugt).toHaveLength(1);
    expect(erzeugt[0].args.projectId).toBe("p-fin");

    await nachricht(page).fill("Was sagt das Buch zu ETF?");
    await nachricht(page).press("Enter");
    await page.waitForFunction(() => (window as any).__chat !== null);
    const gesendet = await calls(page, "voice_call_chat");
    expect(gesendet).toHaveLength(1);
    // Der Stand der Übersicht reist mit; im Projekt kommt kein App-Kontext von der Oberfläche.
    expect(gesendet[0].args.expectedStand).toMatch(/^s\d+$/);
    expect(gesendet[0].args.context ?? null).toBeNull();

    const id = await page.evaluate(
      () => (window as any).__chat.args.requestId as string,
    );
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    // Das Tor puffert die ganze Antwort und gibt sie als Ganzes frei; die Sprachausgabe folgt demselben Text.
    await page.evaluate((rid) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "sentence",
        request_id: rid,
        index: 0,
        text: "Laut Buch kosten breit streuende ETFs wenig.",
      });
      w.__emit("voice-call", { kind: "done", request_id: rid });
      w.__chat.resolve("Laut Buch kosten breit streuende ETFs wenig.");
    }, id);
    const antwort = page
      .getByRole("log")
      .getByText("Laut Buch kosten breit streuende ETFs wenig.");
    await expect(antwort).toBeVisible();
    await expect(page.getByTestId("projekt-hinweis")).toHaveCount(0);
    await expect(page.locator("body")).not.toContainText(SPERRTEXT);
    if (breite === 390)
      expect(
        await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
            document.documentElement.clientWidth,
        ),
      ).toBe(true);
    await aufnehmen(page, "02-gespraech-antwort");
  });

  test(`KI-Dokument: Auftrag, Entwurf in der Ablage, im Projekt ausgeschaltet (${breite} px)`, async ({
    page,
  }) => {
    await projektSetup(page, breite, hoehe);
    await (await sichtbar(page, "projekt-dokument-erstellen")).click();
    const formular = page.getByTestId("projekt-dokument-formular");
    await expect(formular).toBeVisible();
    await formular
      .getByTestId("projekt-dokument-auftrag")
      .fill("Fasse die Auswahl im Projekt in einer Seite zusammen.");
    await formular.getByTestId("projekt-dokument-name").fill("Zusammenfassung");
    await formular
      .getByTestId("projekt-dokument-senden")
      .scrollIntoViewIfNeeded();
    await aufnehmen(page, "03-dokument-formular");
    await formular.getByTestId("projekt-dokument-senden").click();
    const fertig = page.getByTestId("projekt-dokument-fertig");
    await expect(fertig).toContainText("Zusammenfassung.md");
    await expect(fertig).toContainText("ausgeschaltet");
    const a = await calls(page, "projekt_ki_dokument_erstellen");
    expect(a).toHaveLength(1);
    expect(a[0].args.suiteId).toBe("p-fin");
    expect(typeof a[0].args.expectedStand).toBe("string");
    // Breit ist die rechte Spalte mit der Rückmeldung höher als das Fenster; fürs Bild bekommt es etwas
    // mehr Höhe, damit ihr Kopf (die Summenleiste) nicht angeschnitten ist. Schmal scrollt der Dialog.
    if (breite >= 1000) {
      await page.setViewportSize({ width: breite, height: hoehe + 160 });
      await fertig.evaluate((el) => {
        let sc = el.parentElement;
        while (sc && !/(auto|scroll)/.test(getComputedStyle(sc).overflowY))
          sc = sc.parentElement;
        if (sc) sc.scrollTop = 0;
      });
    }
    await fertig.scrollIntoViewIfNeeded();
    await expect(fertig).toBeInViewport();
    await aufnehmen(page, "04-dokument-fertig");
  });
}

// ---------------------------------------------------------------- Recherche mit Projektkontext (Chat-Spalte der Besprechung)

const chatSetup = async (page: Page, breite: number, hoehe: number) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await installLiveMock(page);
  await installProjekteMock(page);
  // Ein Sprachmodell ist eingerichtet: Die Fußzeile zeigt es, statt „Kein Sprachmodell“.
  await page.addInitScript(() => {
    const w = window as any;
    w.__settings.llm_connections = [
      { id: "c1", kind: "api", enabled: true, label: "Test" },
    ];
    w.__settings.llm_models = [
      {
        id: "m-terra",
        connection_id: "c1",
        remote_id: "gpt-6.1-terra",
        label: "GPT-6.1-Terra",
        enabled: true,
        context_limit: 128000,
      },
    ];
    w.__settings.llm_active_model_id = "m-terra";
  });
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

for (const [breite, hoehe] of BREITEN) {
  test(`Recherche mit Projektkontext: Start, Halt mit Herkunft, Freigabe, Ergebnis (${breite} px)`, async ({
    page,
  }) => {
    const panel = await chatSetup(page, breite, hoehe);
    const schalter = panel.getByTestId("chat-recherche-kontext");
    await expect(schalter).toBeEnabled();
    await expect(panel).not.toContainText(SPERRTEXT);
    await schalter.check();
    await panel.locator("textarea").fill(FRAGE);
    await panel.getByTestId("chat-recherche").click();
    await panel.getByRole("button", { name: "Geschäftlich" }).click();
    const start = await calls(page, "recherche_starten");
    expect(start).toHaveLength(1);
    expect(start[0].args.mitKontext).toBe(true);
    // Auch ein internes Projekt hält mit Kontext vor der Suche an; der Dialog sagt, woher die Anfragen kommen.
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText(FRAGE);
    await expect(dialog).toContainText(
      "Die Anfragen entstanden mit Inhalten aus Ihrem Projekt",
    );
    await aufnehmen(page, "05-recherche-halt");
    await dialog.getByRole("checkbox").nth(0).check();
    await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
    await expect(panel.getByTestId("recherche-karte")).toContainText(FRAGE);
    const e = await calls(page, "recherche_freigabe_entscheiden");
    expect(e).toHaveLength(1);
    expect(e[0].args.auswahl).toEqual(["q1"]);
    // Der Hinweis „Die Recherche ist fertig …“ liegt kurz über dem Eingabefeld: fürs Bild abwarten.
    await expect(page.getByText("Die Recherche ist fertig")).toBeHidden({
      timeout: 15000,
    });
    await aufnehmen(page, "06-recherche-ergebnis");
  });
}

// ---------------------------------------------------------------- Rückbau: jeder Weg sagt es in Klartext

test("Rückbau: Recherche und KI-Dokument melden die Sperre in Klartext, ohne Kennung und ohne Versand", async ({
  page,
}) => {
  const panel = await chatSetup(page, 1440, 900);
  await panel.getByTestId("chat-recherche-kontext").check();
  await page.evaluate(() => ((window as any).__kiAus = true));
  await panel.locator("textarea").fill(FRAGE);
  await panel.getByTestId("chat-recherche").click();
  await panel.getByRole("button", { name: "Geschäftlich" }).click();
  await expect(panel).toContainText(
    "Die KI im Projekt ist in dieser Version abgeschaltet. Es wurde nichts gesendet.",
  );
  await expect(panel).not.toContainText("projekt.ki_gesperrt");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
});

test("Rückbau: das KI-Dokument zeigt den Klartext und behält die Eingaben", async ({
  page,
}) => {
  await projektSetup(page, 1440, 900);
  await page.evaluate(() => ((window as any).__kiAus = true));
  await (await sichtbar(page, "projekt-dokument-erstellen")).click();
  const formular = page.getByTestId("projekt-dokument-formular");
  await formular
    .getByTestId("projekt-dokument-auftrag")
    .fill("Schreibe etwas.");
  await formular.getByTestId("projekt-dokument-name").fill("Notiz");
  await formular.getByTestId("projekt-dokument-senden").click();
  const fehler = page.getByTestId("projekt-dokument-fehler");
  await expect(fehler).toContainText(
    "Die KI im Projekt ist in dieser Version abgeschaltet. Es wurde nichts gesendet.",
  );
  await expect(fehler).not.toContainText("projekt.ki_gesperrt");
  await expect(formular.getByTestId("projekt-dokument-auftrag")).toHaveValue(
    "Schreibe etwas.",
  );
  await expect(page.getByTestId("projekt-dokument-fertig")).toHaveCount(0);
});
