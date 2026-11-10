import { test, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import { installProjekteKontextMock } from "./projekteMock";

// lva#365 P4c-2: Gespräch im Projekt (Chat und Sprache über das Versandtor). Die Regeln des Kerns
// (Kontrolltext, Widerruf, Register, Budget, Injektion) prüfen die Rust-Tests in
// `src-tauri/src/voice_call/projekt/tests.rs`; hier die Oberfläche gegen die Attrappe: Start des
// Gesprächs im Projekt, Chip, Hinweisleiste (Budget, Stand veraltet, Antwort entfernt) und die
// Bindung. Die Sperre der KI im Projekt steht fest in Rust (`PROJEKT_KI_FREIGEGEBEN = false`); die
// Attrappe stellt sie über `projekt_ki_status` nach (`__kiFrei`), die App selbst kennt keinen Schalter.

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

/** Wie der Kern: die Gesprächsablage und der Sendeweg `voice_call_chat` (bleibt offen, bis der Test ihn beendet). */
const installGespraech = async (page: Page, frei: boolean) => {
  await page.addInitScript((kiFrei) => {
    const w = window as any;
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__kiFrei = kiFrei;
    w.__chat = null;
    const convs: any[] = [];
    const entries: Record<string, any[]> = {};
    let seq = 0;
    const find = (id: string) => {
      const c = convs.find((x) => x.id === id);
      if (!c) throw "Das Gespräch gibt es nicht (mehr).";
      return c;
    };
    // Erst eine Wahl im Wechsel treffen: die Gesprächsmeldungen der App brauchen `souveraenitaet: frei`,
    // denn die Attrappe hat ein Cloud-Modell.
    localStorage.setItem(
      "voice-call.config",
      JSON.stringify({ orchestration: { souveraenitaet: "frei" } }),
    );
    const merke = (cmd: string, args: Record<string, any>) =>
      w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      switch (cmd) {
        case "projekt_ki_status":
          return { freigegeben: w.__kiFrei };
        case "voice_call_start":
          merke(cmd, args);
          return null;
        case "voice_call_cancel":
          merke(cmd, args);
          return false;
        case "voice_tools_list":
          return [];
        case "voice_call_chat":
          merke(cmd, args);
          return new Promise((resolve, reject) => {
            w.__chat = { args, resolve, reject };
          });
        case "conversations_list":
          return convs.map((c) => ({
            id: c.id,
            title: c.title,
            updated_at: c.updated_at,
            pinned: false,
            archived: false,
            entry_count: (entries[c.id] ?? []).length,
            last_text: "",
          }));
        case "conversation_create": {
          merke(cmd, args);
          const now = new Date().toISOString();
          const c = {
            id: `conv_${String(++seq).padStart(26, "0")}`,
            title: args.title || "Neues Gespräch",
            created_at: now,
            updated_at: now,
            kind: "voice",
            pinned: false,
            archived: false,
            scope: args.projectId ? { project_id: args.projectId } : null,
          };
          convs.push(c);
          entries[c.id] = [];
          return c;
        }
        case "conversation_get":
          return {
            conversation: find(args.id),
            entries: entries[args.id] ?? [],
          };
        case "conversation_update": {
          const c = find(args.id);
          if (args.title != null) c.title = args.title;
          return c;
        }
        case "conversation_append_entry": {
          const c = find(args.conversationId);
          const stored = {
            ...args.entry,
            id: `entry_${String(++seq).padStart(26, "0")}`,
            conversation_id: c.id,
          };
          entries[c.id].push(stored);
          return stored;
        }
        case "conversation_update_entry": {
          const found = Object.values(entries)
            .flat()
            .find((x) => x.id === args.id);
          if (!found) throw "Den Beitrag gibt es nicht (mehr).";
          found.text = args.text;
          found.status = args.status;
          return found;
        }
      }
      return inner(cmd, args);
    };
  }, frei);
};

const setup = async (
  page: Page,
  opts: {
    frei?: boolean;
    width?: number;
    height?: number;
    dark?: boolean;
  } = {},
) => {
  const { frei = true, width = 1440, height = 900, dark = false } = opts;
  page.on("pageerror", (error) => {
    throw error;
  });
  await installLiveMock(page);
  await installProjekteKontextMock(page);
  await installGespraech(page, frei);
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
  test("Gesperrt: der Knopf ist aus und es entsteht kein Gespräch", async ({
    page,
  }) => {
    await setup(page, { frei: false });
    await geheZu(page, "Projekte");
    await waehleProjekt(page, "Finanzwissen");
    const knopf = await knopfSichtbar(page);
    await expect(knopf).toBeDisabled();
    await expect(
      page.getByText("Gespräch im Projekt: gesperrt", { exact: false }).first(),
    ).toBeVisible();
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

  test("Bild des gesperrten Knopfs (LVA_SCREENSHOTS=1)", async ({ page }) => {
    test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur auf Wunsch");
    await setup(page, { frei: false });
    await geheZu(page, "Projekte");
    await waehleProjekt(page, "Finanzwissen");
    await expect(await knopfSichtbar(page)).toBeDisabled();
    await aufnehmen(page, "02-projekt-knopf-gesperrt-1440-hell.png");
  });
});
