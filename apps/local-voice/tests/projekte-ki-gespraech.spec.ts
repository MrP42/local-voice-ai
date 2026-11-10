import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import {
  installGespraechMock,
  installProjekteKontextMock,
} from "./projekteMock";

// lva#365 P4c-2: Gespräch im Projekt (Chat und Sprache über das Versandtor). Die Regeln des Kerns
// (Kontrolltext, Widerruf, Register, Budget, Injektion) prüfen die Rust-Tests in
// `src-tauri/src/voice_call/projekt/tests.rs`; hier die Oberfläche gegen die Attrappe: Start des
// Gesprächs im Projekt, Chip, Hinweisleiste (Budget, Stand veraltet, Antwort entfernt) und die
// Bindung. Seit P4c-4 ist die KI im Projekt freigeschaltet (`PROJEKT_KI_FREIGEGEBEN = true`); die
// Oberfläche kennt keinen Schalter und fragt nichts ab, jede Entscheidung trifft das Versandtor.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "docs/vorschau/projekte-p4c2/umsetzung",
);

const BUDGET = {
  fenster: 128000,
  benoetigt: 150000,
  system: 400,
  kontext: 140000,
  historie: 3000,
  auszuege: 0,
  zwischen: 0,
  frage: 100,
  puffer: 600,
  ausgabe: 4096,
};

const setup = async (
  page: Page,
  opts: {
    width?: number;
    height?: number;
    dark?: boolean;
  } = {},
) => {
  const { width = 1440, height = 900, dark = false } = opts;
  page.on("pageerror", (error) => {
    throw error;
  });
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

const geheZu = async (page: Page, name: string) => {
  const nav = page.getByRole("button", { name, exact: true });
  if (!(await nav.isVisible())) {
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await nav.click();
};

const aufrufe = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: any) => x.cmd === c),
    cmd,
  );

const waehleProjekt = async (page: Page, titel: string) => {
  await page
    .getByTestId("projekt-zeile")
    .filter({ hasText: titel })
    .first()
    .click();
};

/** Öffnet „Projekte“, wählt das Projekt und beginnt das Gespräch im Projekt. */
const beginne = async (page: Page) => {
  await geheZu(page, "Projekte");
  await waehleProjekt(page, "Finanzwissen");
  const knopf = await knopfSichtbar(page);
  await expect(knopf).toBeEnabled();
  await knopf.click();
  await expect(page.getByTestId("projekt-chip")).toBeVisible();
};

/** Der Knopf steht in der rechten Spalte; schmal (< 999 px) im Dialog „Details“. */
const knopfSichtbar = async (page: Page) => {
  const knopf = page
    .getByTestId("projekt-gespraech-beginnen")
    .filter({ visible: true });
  if ((await knopf.count()) === 0)
    await page.getByRole("button", { name: "Details", exact: true }).click();
  await expect(knopf.first()).toBeVisible();
  return knopf.first();
};

const nachricht = (page: Page) =>
  page.getByRole("textbox", { name: "Nachricht" });

/** Sendet eine Frage und gibt die Kennung der Anfrage zurück; der Sendeweg bleibt offen. */
const frage = async (page: Page, text: string): Promise<string> => {
  await page.evaluate(() => ((window as any).__chat = null));
  await nachricht(page).fill(text);
  await nachricht(page).press("Enter");
  await expect
    .poll(() => page.evaluate(() => (window as any).__chat !== null))
    .toBe(true);
  await page.waitForFunction(() =>
    (window as any).__calls.some((c: any) => c.cmd === "voice_call_chat"),
  );
  return page.evaluate(() => (window as any).__chat.args.requestId as string);
};

const hinweis = (page: Page) => page.getByTestId("projekt-hinweis");

const aufnehmen = async (page: Page, name: string) => {
  fs.mkdirSync(SHOT_DIR, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(SHOT_DIR, name),
    animations: "disabled",
  });
};

test.describe("Gespräch im Projekt", () => {
  test("Der Knopf ist aktiv, ohne Sperrtext, und erklärt, was er tut", async ({
    page,
  }) => {
    await setup(page);
    await geheZu(page, "Projekte");
    await waehleProjekt(page, "Finanzwissen");
    const knopf = await knopfSichtbar(page);
    await expect(knopf).toBeEnabled();
    const bereich = page.getByTestId("ki-im-projekt").filter({ visible: true });
    await expect(bereich.getByText(/Gesperrt/)).toHaveCount(0);
    await expect(bereich).toContainText(
      "Öffnet ein neues Gespräch, das nur diese Auswahl als Kontext nutzt.",
    );
    // Beschreiben allein legt nichts an: erst der Klick erzeugt das Gespräch.
    expect(await aufrufe(page, "conversation_create")).toHaveLength(0);
  });

  test("Start bindet das Gespräch an das Projekt; Chip statt Berechtigungs-Chip", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    const erzeugt = await aufrufe(page, "conversation_create");
    expect(erzeugt).toHaveLength(1);
    // Die Bindung kommt aus dem Anlegen, nicht aus einer späteren Eingabe.
    expect(erzeugt[0].args.projectId).toBe("p-fin");
    await expect(page.getByTestId("projekt-chip")).toContainText(
      "Projekt: Finanzwissen",
    );
    // Der Datenbereich (Notizbuch, App-Daten) gilt im Projekt nicht: kein Berechtigungs-Chip.
    await expect(page.getByTestId("permission-chip")).toHaveCount(0);
    await expect(hinweis(page)).toHaveCount(0);
  });

  test("Chip öffnet das Projekt", async ({ page }) => {
    await setup(page);
    await beginne(page);
    await page.getByTestId("projekt-chip").click();
    await expect(page.getByTestId("projekte-bereich")).toBeVisible();
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "9 Einträge",
    );
  });

  test("Frage geht mit dem gesehenen Stand und ohne App-Kontext", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    await frage(page, "Was sagt das Buch zu ETF?");
    const gesendet = await aufrufe(page, "voice_call_chat");
    expect(gesendet).toHaveLength(1);
    const a = gesendet[0].args;
    expect(a.expectedStand).toMatch(/^s\d+$/);
    // Im Projekt gibt es keinen App-Kontext von der Oberfläche; die Kontextnachricht baut das Tor.
    expect(a.context ?? null).toBeNull();
    expect(JSON.stringify(a.messages)).toContain("Was sagt das Buch zu ETF?");
  });

  test("Budgetstopp zeigt die Anteile und sendet nichts", async ({ page }) => {
    await setup(page);
    await beginne(page);
    const id = await frage(page, "Fasse alles zusammen.");
    await page.evaluate(
      ([rid, budget]) => {
        const w = window as any;
        w.__emit("voice-call", {
          kind: "projekt",
          request_id: rid,
          code: "projekt.budget",
          budget,
        });
        w.__emit("voice-call", { kind: "failed", request_id: rid });
        w.__chat.reject("projekt.budget: Zu viel Kontext für das Modell.");
      },
      [id, BUDGET] as const,
    );
    const leiste = hinweis(page);
    await expect(leiste).toBeVisible();
    await expect(leiste).toHaveAttribute("data-code", "projekt.budget");
    await expect(leiste).toContainText("Nicht gesendet");
    await expect(leiste).toContainText("Nötig sind 150.000 von 128.000 Token");
    await expect(leiste).toContainText("Es wurde nichts gesendet");
    // Schließen entfernt die Leiste.
    await page.getByTestId("projekt-hinweis-schliessen").click();
    await expect(leiste).toHaveCount(0);
  });

  test("Jeder Stoppcode hat einen eigenen verständlichen Text ohne Kennung", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    const faelle: [string, string][] = [
      ["projekt.ungueltig", "Das Projekt ist gerade nicht verfügbar."],
      ["projekt.anfrage", "Die Anfrage war unvollständig."],
      ["projekt.fremdkontext", "nur Inhalte aus dem Projekt zulässig"],
      ["projekt.register", "Das Antwortregister des Projekts"],
      ["projekt.ablage", "Die Dokumentenablage ist gerade nicht lesbar."],
      ["projekt.bindung", "nicht mehr sicher zuordnen"],
      ["projekt.kein_modell", "Es ist kein zulässiges Modell gewählt."],
      ["projekt.ziel_compliance", "nach den Regeln der Organisation"],
      ["projekt.ziel_fenster", "Größe des Kontextfensters"],
      ["projekt.ziel_lokal", "Das lokale Modell ist gerade nicht bereit."],
      ["projekt.werkzeuge_aus", "Werkzeuge ausgeschaltet"],
      ["projekt.klasse_ziel", "zu schutzbedürftig"],
      // Rückbau (eine Konstante in Rust): Das Tor lehnt mit diesem Code ab, in Klartext.
      ["projekt.ki_gesperrt", "Die KI im Projekt ist ausgeschaltet."],
    ];
    for (const [code, text] of faelle) {
      await frage(page, `Frage zu ${code}`);
      await page.evaluate(
        (c) => (window as any).__chat.reject(`${c}: Technischer Text.`),
        code,
      );
      const leiste = hinweis(page);
      await expect(leiste, code).toHaveAttribute("data-code", code);
      await expect(leiste, code).toContainText(text);
      await expect(leiste, code).not.toContainText("Kennung");
      await expect(leiste, code).not.toContainText("projekt.");
      await expect(leiste, code).not.toContainText("Technischer Text");
      // Der Hinweis sagt höchstens einmal, dass nichts gesendet wurde.
      const inhalt = (await leiste.textContent()) ?? "";
      expect(inhalt.split("nichts gesendet").length - 1, code).toBeLessThan(2);
      await page.getByTestId("projekt-hinweis-schliessen").click();
      await expect(leiste).toHaveCount(0);
    }
  });

  test("Unbekannter Code: eigener Text mit Kennung, ohne doppeltes „nichts gesendet“", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    await frage(page, "Frage mit neuem Code");
    await page.evaluate(() =>
      (window as any).__chat.reject("projekt.neu_unbekannt: Text."),
    );
    const leiste = hinweis(page);
    await expect(leiste).toHaveAttribute("data-code", "projekt.neu_unbekannt");
    await expect(leiste).toContainText("unerwarteter Fehler");
    await expect(leiste).toContainText("Kennung: projekt.neu_unbekannt");
    await expect(leiste).not.toContainText("nichts gesendet");
  });

  test("Fehler des Modells: fester Text, nichts vom Anbieter", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    const id = await frage(page, "Was kostet ein ETF?");
    await page.evaluate((rid) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "projekt",
        request_id: rid,
        code: "projekt.modell",
      });
      w.__emit("voice-call", { kind: "failed", request_id: rid });
      w.__chat.reject(
        "projekt.modell: Das Modell hat nicht geantwortet. Es wurde keine Antwort übernommen.",
      );
    }, id);
    const leiste = hinweis(page);
    await expect(leiste).toHaveAttribute("data-code", "projekt.modell");
    await expect(leiste).toContainText("Keine Antwort");
    await expect(leiste).toContainText("Das Modell hat nicht geantwortet.");
    await expect(leiste).not.toContainText("Kennung");
  });

  test("Abgelehnt im Projekt: nur die Hinweisleiste, keine Vorlagenzeile und kein Eintrag", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    const vorlage = "Das kann ich gerade nicht sicher beantworten.";
    for (const code of ["projekt.stand_veraltet", "projekt.register"]) {
      await frage(page, `Frage mit Stopp ${code}`);
      await page.evaluate(
        (c) => (window as any).__chat.reject(`${c}: Technischer Text.`),
        code,
      );
      await expect(hinweis(page)).toHaveAttribute("data-code", code);
      // Weder in der Anzeige noch im gespeicherten Gespräch steht die Vorlage.
      await expect(page.getByText(vorlage)).toHaveCount(0);
      await expect(
        page.getByRole("log").getByText("sicher beantworten"),
      ).toHaveCount(0);
      await page.getByTestId("projekt-hinweis-schliessen").click();
    }
    const gespeichert = await aufrufe(page, "conversation_append_entry");
    expect(JSON.stringify(gespeichert)).not.toContain("sicher beantworten");
    expect(
      gespeichert.filter((c: any) => c.args.entry?.role === "assistant"),
    ).toHaveLength(0);
  });

  test("Ohne Projekt bleibt die Vorlage bei einem Fehler (Gegenprobe)", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    await page
      .getByRole("button", { name: "Neues Gespräch", exact: true })
      .click();
    await expect(page.getByTestId("projekt-chip")).toHaveCount(0);
    await frage(page, "Ganz gewöhnliche Frage");
    await page.evaluate(() => (window as any).__chat.reject("Netzwerkfehler"));
    await expect(
      page
        .getByRole("log")
        .getByText("Das kann ich gerade nicht sicher beantworten."),
    ).toBeVisible();
    await expect(hinweis(page)).toHaveCount(0);
    // Ohne Projekt wird die Vorlage wie bisher als Antwort gespeichert (die Probe misst also etwas).
    await expect
      .poll(async () =>
        JSON.stringify(await aufrufe(page, "conversation_append_entry")),
      )
      .toContain("sicher beantworten");
  });

  test("Stand veraltet: Auswahl übernehmen holt den aktuellen Stand", async ({
    page,
  }) => {
    await setup(page);
    await beginne(page);
    const erster = (await aufrufe(page, "projekt_kontext_uebersicht")).length;
    await frage(page, "Erste Frage");
    await page.evaluate(() =>
      (window as any).__chat.reject(
        "projekt.stand_veraltet: Die Auswahl hat sich geändert.",
      ),
    );
    const leiste = hinweis(page);
    await expect(leiste).toHaveAttribute("data-code", "projekt.stand_veraltet");
    await expect(leiste).toContainText("Die Auswahl im Projekt hat sich");
    const stand1 = (await aufrufe(page, "voice_call_chat"))[0].args
      .expectedStand;
    await page.getByTestId("projekt-stand-uebernehmen").click();
    await expect(leiste).toHaveCount(0);
    expect(
      (await aufrufe(page, "projekt_kontext_uebersicht")).length,
    ).toBeGreaterThan(erster);
    // Die nächste Frage geht mit dem neuen Stand.
    await frage(page, "Zweite Frage");
    const stand2 = (await aufrufe(page, "voice_call_chat"))[1].args
      .expectedStand;
    expect(stand2).toMatch(/^s\d+$/);
    expect(stand2).not.toBe(stand1);
  });

  test("Widerruf entfernt die Antwort und sagt es", async ({ page }) => {
    await setup(page);
    await beginne(page);
    const id = await frage(page, "Was kostet ein ETF?");
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    await page.evaluate((rid) => {
      (window as any).__emit("voice-call", {
        kind: "sentence",
        request_id: rid,
        index: 0,
        text: "Ein ETF kostet je nach Anbieter wenig.",
      });
    }, id);
    await expect(
      page.getByRole("log").getByText("Ein ETF kostet je nach Anbieter wenig."),
    ).toBeVisible();
    await page.evaluate((rid) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "revoked",
        request_id: rid,
      });
      w.__chat.resolve("");
    }, id);
    await expect(
      page.getByText("Ein ETF kostet je nach Anbieter wenig."),
    ).toHaveCount(0);
    const leiste = hinweis(page);
    await expect(leiste).toHaveAttribute("data-code", "projekt.widerrufen");
    await expect(leiste).toContainText("Antwort entfernt");
    await expect(leiste).toContainText("Die Antwort wurde entfernt.");
  });

  // lva#459: Das Tor puffert die ganze Antwort. Die Frist von 30 s gilt im Projekt nur ohne
  // Lebenszeichen („laeuft“, ohne Inhalt); dazu eine harte Obergrenze von fünf Minuten.
  const vorlageText = "Das kann ich gerade nicht sicher beantworten.";
  const laeuft = (page: Page, id: string) =>
    page.evaluate(
      (rid) =>
        (window as any).__emit("voice-call", {
          kind: "laeuft",
          request_id: rid,
        }),
      id,
    );

  test("Projekt: Lebenszeichen verlängern die Frist über 30 s, die Antwort kommt an", async ({
    page,
  }) => {
    await page.clock.install();
    await setup(page);
    await beginne(page);
    const id = await frage(page, "Was kostet ein ETF?");
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    // 90 s gepuffert, alle 10 s ein Lebenszeichen: weder Abbruch noch Vorlage noch Hinweis.
    for (let i = 0; i < 9; i++) {
      await page.clock.fastForward(10_000);
      await laeuft(page, id);
    }
    expect(await aufrufe(page, "voice_call_cancel")).toHaveLength(0);
    await expect(page.getByText(vorlageText)).toHaveCount(0);
    await expect(hinweis(page)).toHaveCount(0);
    await page.evaluate((rid) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "sentence",
        request_id: rid,
        index: 0,
        text: "Ein ETF kostet je nach Anbieter wenig.",
      });
      w.__emit("voice-call", { kind: "done", request_id: rid });
      w.__chat.resolve("Ein ETF kostet je nach Anbieter wenig.");
    }, id);
    await expect(
      page.getByRole("log").getByText("Ein ETF kostet je nach Anbieter wenig."),
    ).toBeVisible();
    await expect(hinweis(page)).toHaveCount(0);
  });

  test("Projekt: ohne Lebenszeichen nach 30 s und trotz Lebenszeichen nach fünf Minuten kein Vorlagentext, Hinweis mit eigenem Code", async ({
    page,
  }) => {
    await page.clock.install();
    await setup(page);
    await beginne(page);

    // (a) Das Modell meldet sich gar nicht: nach der Frist verwerfen.
    const erste = await frage(page, "Frage ohne Lebenszeichen");
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    // Ein Lebenszeichen einer fremden Anfrage verlängert diese Frist nicht.
    await page.clock.fastForward(20_000);
    await laeuft(page, "vc-fremd-0");
    await page.clock.fastForward(11_000);
    let leiste = hinweis(page);
    await expect(leiste).toHaveAttribute(
      "data-code",
      "projekt.zeitueberschreitung",
    );
    await expect(leiste).toContainText("Keine Antwort");
    await expect(leiste).toContainText("Das Modell hat zu lange gebraucht");
    await expect(leiste).not.toContainText("Kennung");
    await expect(page.getByText(vorlageText)).toHaveCount(0);
    expect(await aufrufe(page, "voice_call_cancel")).toHaveLength(1);
    // Ein spätes Stück und `done` nach dem Fristablauf erscheinen nirgends.
    await page.evaluate((rid) => {
      const w = window as any;
      w.__emit("voice-call", {
        kind: "sentence",
        request_id: rid,
        index: 0,
        text: "SPÄTE-ANTWORT-4711",
      });
      w.__emit("voice-call", { kind: "done", request_id: rid });
      w.__chat.resolve("SPÄTE-ANTWORT-4711");
    }, erste);
    await page.waitForTimeout(300);
    await expect(page.getByText("SPÄTE-ANTWORT-4711")).toHaveCount(0);
    await expect(page.getByText(vorlageText)).toHaveCount(0);
    expect(
      JSON.stringify(await aufrufe(page, "conversation_append_entry")),
    ).not.toContain("SPÄTE-ANTWORT");
    await page.getByTestId("projekt-hinweis-schliessen").click();

    // (b) Es meldet sich dauernd, antwortet aber nie: die harte Obergrenze greift.
    const id = await frage(page, "Frage mit Dauer-Lebenszeichen");
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    for (let i = 0; i < 55; i++) {
      await page.clock.fastForward(5_000);
      await laeuft(page, id);
    }
    // 275 s: noch kein Abbruch.
    expect(await aufrufe(page, "voice_call_cancel")).toHaveLength(1);
    await expect(hinweis(page)).toHaveCount(0);
    await page.clock.fastForward(30_000);
    leiste = hinweis(page);
    await expect(leiste).toHaveAttribute(
      "data-code",
      "projekt.zeitueberschreitung",
    );
    await expect(page.getByText(vorlageText)).toHaveCount(0);
    expect(await aufrufe(page, "voice_call_cancel")).toHaveLength(2);
    // Weder Vorlage noch Antwort im gespeicherten Gespräch.
    const gespeichert = await aufrufe(page, "conversation_append_entry");
    expect(JSON.stringify(gespeichert)).not.toContain("sicher beantworten");
    expect(
      gespeichert.filter((c: any) => c.args.entry?.role === "assistant"),
    ).toHaveLength(0);
  });

  test("Ohne Projekt gilt die feste Frist weiter: Vorlage nach 30 s, Lebenszeichen ändern nichts (Gegenprobe)", async ({
    page,
  }) => {
    await page.clock.install();
    await setup(page);
    await beginne(page);
    await page
      .getByRole("button", { name: "Neues Gespräch", exact: true })
      .click();
    await expect(page.getByTestId("projekt-chip")).toHaveCount(0);
    const id = await frage(page, "Ganz gewöhnliche Frage");
    await page.waitForFunction(
      () => (window as any).__listening?.("voice-call") !== 0,
    );
    await page.clock.fastForward(20_000);
    await laeuft(page, id);
    await page.clock.fastForward(11_000);
    await expect(page.getByRole("log").getByText(vorlageText)).toBeVisible();
    await expect(hinweis(page)).toHaveCount(0);
    expect(await aufrufe(page, "voice_call_cancel")).toHaveLength(1);
  });

  test("Ein neuer Verlauf trägt die Bindung nicht weiter", async ({ page }) => {
    await setup(page);
    await beginne(page);
    await page
      .getByRole("button", { name: "Neues Gespräch", exact: true })
      .click();
    await expect(page.getByTestId("projekt-chip")).toHaveCount(0);
    await expect(page.getByTestId("permission-chip")).toBeVisible();
  });

  for (const [breite, dark, name] of [
    [1440, false, "01-gespraech-1440-hell.png"],
    [1440, true, "01-gespraech-1440-dunkel.png"],
    [390, false, "01-gespraech-390-hell.png"],
  ] as const) {
    test(`Bild ${name} mit Budgetstopp (LVA_SCREENSHOTS=1)`, async ({
      page,
    }) => {
      test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur auf Wunsch");
      await setup(page, {
        width: breite,
        height: breite === 390 ? 844 : 900,
        dark,
      });
      await beginne(page);
      const id = await frage(page, "Fasse alles zusammen.");
      await page.evaluate(
        ([rid, budget]) => {
          const w = window as any;
          w.__emit("voice-call", {
            kind: "projekt",
            request_id: rid,
            code: "projekt.budget",
            budget,
          });
          w.__chat.reject("projekt.budget: Zu viel Kontext für das Modell.");
        },
        [id, BUDGET] as const,
      );
      await expect(hinweis(page)).toBeVisible();
      if (breite === 390) {
        const breiteOk = await page.evaluate(
          () =>
            document.documentElement.scrollWidth <=
            document.documentElement.clientWidth,
        );
        expect(breiteOk).toBe(true);
      }
      await aufnehmen(page, name);
    });
  }

  test("Bild des aktiven Knopfs (LVA_SCREENSHOTS=1)", async ({ page }) => {
    test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur auf Wunsch");
    await setup(page);
    await geheZu(page, "Projekte");
    await waehleProjekt(page, "Finanzwissen");
    await expect(await knopfSichtbar(page)).toBeEnabled();
    await aufnehmen(page, "02-projekt-knopf-aktiv-1440-hell.png");
  });
});
