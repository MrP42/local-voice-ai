import { test, expect, type Page } from "@playwright/test";
import { join } from "node:path";
import { calls, installTauriMock } from "./calendarMock";
import { geheZu } from "./profilmenue";
import { openRecordings, pickMeeting } from "./recLayoutMock";
import { installLiveMock } from "./recLiveMock";

// G9a-2 (lva#578, ADR-0031 E2.6–E2.8): Freigabe für KI-Clients. Eingaben unter dem
// MCP-Schalter (Eigene Domänen, Cloud bei Externen) und die Freigabe EINER Besprechung im
// Details-Dialog, gegen die Tauri-Attrappe. Die Regeln selbst prüfen die Rust-Tests
// (`besprechungsplanung::assistent`, `ki_freigabe`, `mcp::ki_sperre_tests`). Aufnahmen nur mit
// G9A2_SHOTS=1 (docs/vorschau/ki-freigabe-g9a2). Alle Adressen sind erfunden (`example.*`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOTS = process.env.G9A2_SHOTS === "1";
const SHOT_DIR = join("docs", "vorschau", "ki-freigabe-g9a2");
const shoot = async (page: Page, name: string) => {
  if (!SHOTS) return;
  await page.waitForTimeout(200);
  await page.screenshot({
    path: join(SHOT_DIR, `${name}.png`),
    animations: "disabled",
  });
};

const FP = "f".repeat(64);

/** Rechtsgrundlage im Freigabe-Feld wählen (Liste hängt an `document.body`). */
const grundlage = async (page: Page, name: string) => {
  await page
    .getByTestId("details-ai-release")
    .getByRole("combobox", { name: /Rechtsgrundlage/ })
    .click();
  await page.getByRole("option", { name: new RegExp(name) }).click();
};

/** Attrappe der G9a-2-Commands; `stand` ist der Zustand je Besprechung. */
const installReleaseCommands = async (
  page: Page,
  init: {
    einstufung: string;
    externe?: string[];
    ohne?: number;
    statusFails?: boolean;
    /** Diese Besprechungen sind beim Start schon freigegeben. */
    vorab?: string[];
    /** Statusabruf dieser Besprechung wartet, bis `__weiter()` aufgerufen wird. */
    verzoegert?: string;
  },
) => {
  await page.addInitScript((cfg) => {
    const w = window as any;
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__settings.meeting_mcp_enabled = true;
    w.__settings.meeting_own_domains = w.__settings.meeting_own_domains ?? [];
    w.__settings.meeting_cloud_with_externals = false;
    // Bestätigung JE Besprechung, damit ein Fehler über Besprechungen hinweg auffällt.
    w.__bestaetigungen = {};
    for (const id of cfg.vorab ?? []) {
      w.__bestaetigungen[id] = {
        einstufung: "intern",
        fingerabdruck: "f".repeat(64),
      };
    }
    let weiter: () => void = () => {};
    const sperre = new Promise<void>((r) => (weiter = r));
    w.__weiter = () => weiter();
    const stand = (id: string) => {
      const extern = cfg.einstufung === "extern";
      const b = w.__bestaetigungen[id] ?? null;
      let entscheidung: any;
      if (!b) {
        entscheidung = {
          ergebnis: "gesperrt",
          gruende: ["einstufung_unbestaetigt"].concat(
            extern ? ["teilnehmende_extern"] : [],
          ),
        };
      } else if (extern && !w.__settings.meeting_cloud_with_externals) {
        entscheidung = {
          ergebnis: "gesperrt",
          gruende: ["teilnehmende_extern"],
        };
      } else {
        entscheidung = { ergebnis: "erlaubt", einstufung: b.einstufung };
      }
      return {
        ergebnis: {
          einstufung: cfg.einstufung,
          externe: cfg.externe ?? [],
          ohne_adresse: cfg.ohne ?? 0,
          aus_termin: true,
          fingerabdruck: "f".repeat(64),
        },
        bestaetigung: b,
        entscheidung,
      };
    };
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, unknown> = {},
    ) => {
      switch (cmd) {
        case "meeting_mcp_info":
          return {
            exe_path: "C:\\Programme\\Local Voice AI\\local-voice-ai.exe",
          };
        case "meeting_ai_release_status":
          w.__calls.push({ cmd, args });
          if (cfg.statusFails) throw "meetings_unavailable";
          if (cfg.verzoegert === args.meetingId) {
            await sperre;
            // Verspätete Antwort: behauptet „freigegeben“, darf aber nirgends ankommen.
            w.__verspaetetGeliefert = true;
            return {
              ...stand(args.meetingId as string),
              entscheidung: { ergebnis: "erlaubt", einstufung: "intern" },
            };
          }
          return stand(args.meetingId as string);
        case "meeting_ai_release_confirm":
          w.__calls.push({ cmd, args });
          {
            const n = args.nachweis as any;
            if (n?.informiert !== true) throw "ki_freigabe_ohne_information";
            if (!n.rechtsgrundlage) throw "ki_freigabe_ohne_rechtsgrundlage";
            if (
              n.rechtsgrundlage === "einwilligung" &&
              !n.einwilligungen_liegen_vor
            )
              throw "ki_freigabe_ohne_einwilligung";
          }
          w.__bestaetigungen[args.meetingId as string] = {
            einstufung: args.einstufung,
            fingerabdruck: args.fingerabdruck,
          };
          return stand(args.meetingId as string);
        case "meeting_ai_release_revoke":
          w.__calls.push({ cmd, args });
          delete w.__bestaetigungen[args.meetingId as string];
          return stand(args.meetingId as string);
        case "change_meeting_own_domains_setting": {
          w.__calls.push({ cmd, args });
          const all = args.domains as string[];
          const abgelehnt = all.filter(
            (d) => d === "gmx.de" || d === "*.co.at",
          );
          const gueltig = all.filter((d) => !abgelehnt.includes(d));
          w.__settings.meeting_own_domains = gueltig;
          return { gueltig, abgelehnt };
        }
        case "meeting_own_domains_suggestion":
          w.__calls.push({ cmd, args });
          return ["example.org"];
        case "change_meeting_cloud_with_externals_setting":
          w.__calls.push({ cmd, args });
          w.__settings.meeting_cloud_with_externals = args.enabled;
          return null;
      }
      return original(cmd, args);
    };
  }, init);
};

test.describe("Eingaben unter dem MCP-Schalter", () => {
  const setup = async (page: Page) => {
    await installTauriMock(page, "main");
    await installReleaseCommands(page, { einstufung: "intern" });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await geheZu(page, "Integrationen");
    await expect(page.getByTestId("ai-release-settings")).toBeVisible();
  };

  test("Vorschlag landet nur im Feld; Speichern meldet Abgelehntes", async ({
    page,
  }) => {
    await setup(page);
    await page.getByTestId("own-domains-suggest").click();
    await expect(page.getByTestId("own-domains-input")).toHaveValue(
      "example.org",
    );
    expect(
      await calls(page, "change_meeting_own_domains_setting"),
    ).toHaveLength(0);
    await page
      .getByTestId("own-domains-input")
      .fill("example.org; *.example.org, gmx.de *.co.at");
    await page.getByTestId("own-domains-save").click();
    await expect(page.getByTestId("own-domains-rejected")).toContainText(
      "gmx.de, *.co.at",
    );
    expect(
      (await calls(page, "change_meeting_own_domains_setting"))[0].args,
    ).toEqual({
      domains: ["example.org", "*.example.org", "gmx.de", "*.co.at"],
    });
    await expect(page.getByTestId("own-domains-input")).toHaveValue(
      "example.org, *.example.org",
    );
    await shoot(page, "01-einstellungen");
  });

  test("Cloud bei Externen ist aus und schaltbar", async ({ page }) => {
    await setup(page);
    // Zugänglicher Name und echte Tastatur (kein DOM-Klick).
    const toggle = page.getByRole("switch", {
      name: "Cloud-Modelle in Besprechungen mit Externen",
      exact: true,
    });
    await expect(toggle).not.toBeChecked();
    await toggle.focus();
    await page.keyboard.press("Space");
    await expect(toggle).toBeChecked();
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_cloud_with_externals_setting"))
            .length,
      )
      .toBe(1);
  });
});

test.describe("Freigabe einer Besprechung in den Details", () => {
  const dialog = (page: Page) => page.getByRole("dialog", { name: "Details" });

  const open = async (page: Page) => {
    await openRecordings(page, 1366, 900);
    await pickMeeting(page, "m2");
    await page.getByTestId("meeting-details-open").click();
    await expect(dialog(page)).toBeVisible();
  };

  test("intern: gesperrt, dann freigeben und zurücknehmen", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, { einstufung: "intern" });
    await open(page);
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await expect(row.getByTestId("ai-release-reasons")).toContainText(
      "noch nicht bestätigt",
    );
    await expect(row.getByTestId("ai-release-list")).toHaveText(
      "Laut Termin nur interne Teilnehmende.",
    );
    await expect(row.getByTestId("ai-release-confirm-extern")).toHaveCount(0);
    // Ohne Information und Rechtsgrundlage kein Bestätigen.
    const confirmIntern = row.getByRole("button", {
      name: "Als intern freigeben",
    });
    await expect(confirmIntern).toBeDisabled();
    await shoot(page, "02-details-gesperrt");
    // Tastatur: Häkchen mit Leertaste, Knopf mit Eingabetaste.
    const informed = row.getByRole("checkbox", { name: /informiert/ });
    await informed.focus();
    await page.keyboard.press("Space");
    await expect(informed).toBeChecked();
    await expect(confirmIntern).toBeDisabled();
    const basis = row.getByRole("combobox", { name: /Rechtsgrundlage/ });
    await basis.focus();
    await page.keyboard.type("Einwilligung");
    await page.keyboard.press("Enter");
    // Einwilligung: erst mit „liegt vor“.
    await expect(confirmIntern).toBeDisabled();
    const consent = row.getByRole("checkbox", { name: /Einwilligungen/ });
    await consent.focus();
    await page.keyboard.press("Space");
    await expect(confirmIntern).toBeEnabled();
    await confirmIntern.focus();
    await page.keyboard.press("Enter");
    await expect(row.getByTestId("ai-release-status")).toHaveText(
      "Freigegeben (intern)",
    );
    const confirm = await calls(page, "meeting_ai_release_confirm");
    expect(confirm[0].args).toMatchObject({
      meetingId: "m2",
      einstufung: "intern",
      fingerabdruck: FP,
      nachweis: {
        informiert: true,
        rechtsgrundlage: "einwilligung",
        einwilligungen_liegen_vor: true,
      },
    });
    await shoot(page, "03-details-freigegeben");
    await row.getByTestId("ai-release-revoke").click();
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
  });

  test("extern: kein „intern“, Bestätigung bleibt ohne Einstellung gesperrt", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, {
      einstufung: "extern",
      externe: ["x@example.com"],
    });
    await open(page);
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByTestId("ai-release-list")).toContainText(
      "x@example.com",
    );
    await expect(row.getByTestId("ai-release-confirm-intern")).toHaveCount(0);
    await row.getByRole("checkbox", { name: /informiert/ }).check();
    await grundlage(page, "Öffentliche Aufgabe");
    await row.getByTestId("ai-release-confirm-extern").click();
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await expect(row.getByTestId("ai-release-reasons")).toContainText(
      "Externe dabei",
    );
    await shoot(page, "04-details-extern");
  });

  test("auch ohne MCP-Server sichtbar (Brücke, Sprachgespräch)", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, { einstufung: "intern" });
    await page.addInitScript(() => {
      (window as any).__settings.meeting_mcp_enabled = false;
    });
    await open(page);
    await expect(
      dialog(page)
        .getByTestId("details-ai-release")
        .getByTestId("ai-release-status"),
    ).toHaveText("Gesperrt");
  });

  test("Ladefehler bei bestehender Freigabe: Status unbekannt, nicht „gesperrt“", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, {
      einstufung: "intern",
      statusFails: true,
      vorab: ["m2"],
    });
    await open(page);
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByRole("alert")).toHaveText(
      "Der Freigabestand ließ sich nicht laden. Der aktuelle Status ist unbekannt.",
    );
    await expect(row).not.toContainText("gesperrt");
  });

  test("verspätete Antwort einer anderen Besprechung kommt nicht an", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, {
      einstufung: "intern",
      verzoegert: "m2",
    });
    await open(page);
    // m2 wartet noch; Dialog schließen und m1 öffnen.
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);
    await pickMeeting(page, "m1");
    await page.getByTestId("meeting-details-open").click();
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await page.evaluate(() => (window as any).__weiter());
    await expect
      .poll(async () => (await calls(page, "meeting_ai_release_status")).length)
      .toBeGreaterThanOrEqual(2);
    // Ohne feste Wartezeit: erst ist die verspätete Antwort nachweislich zurückgegeben, dann
    // laufen zwei Bildwechsel, in denen React einen daraus folgenden Zustand gezeichnet hätte.
    await page.waitForFunction(
      () => (window as any).__verspaetetGeliefert === true,
    );
    await page.evaluate(
      () =>
        new Promise<void>((r) =>
          requestAnimationFrame(() => requestAnimationFrame(() => r())),
        ),
    );
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await expect(row).not.toContainText("Freigegeben");
  });

  test("Ladefehler wird angezeigt, nichts ist freigegeben", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, {
      einstufung: "intern",
      statusFails: true,
    });
    await open(page);
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByRole("alert")).toContainText("ließ sich nicht laden");
    await expect(row.getByTestId("ai-release-confirm-intern")).toHaveCount(0);
  });

  test("Freigabe gilt nur für die eine Besprechung", async ({ page }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, { einstufung: "intern" });
    await open(page);
    const row = dialog(page).getByTestId("details-ai-release");
    await row.getByRole("checkbox", { name: /informiert/ }).check();
    await grundlage(page, "Vertrag");
    await row.getByTestId("ai-release-confirm-intern").click();
    await expect(row.getByTestId("ai-release-status")).toHaveText(
      "Freigegeben (intern)",
    );
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);
    // Andere Besprechung: eigener, noch gesperrter Stand; Häkchen nicht übernommen.
    await pickMeeting(page, "m1");
    await page.getByTestId("meeting-details-open").click();
    const other = dialog(page).getByTestId("details-ai-release");
    await expect(other.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await expect(
      other.getByRole("checkbox", { name: /informiert/ }),
    ).not.toBeChecked();
    const status = await calls(page, "meeting_ai_release_status");
    expect(
      status.map((c: { args: { meetingId: string } }) => c.args.meetingId),
    ).toContain("m1");
  });
});

// Englische Oberfläche: dieselben Bedienwege mit englischen Texten.
test.describe("Release in English", () => {
  test.use({ locale: "en-US" });
  const dialog = (page: Page) => page.getByRole("dialog");

  test("internal: needs information and legal basis", async ({ page }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, { einstufung: "intern" });
    await page.addInitScript(() => {
      (window as any).__settings.app_language = "en";
    });
    await page.setViewportSize({ width: 1366, height: 900 });
    await page.goto("/");
    await page.getByRole("button", { name: "Recordings", exact: true }).click();
    await page.getByTestId("rec-content").waitFor();
    await pickMeeting(page, "m2");
    await page.getByTestId("meeting-details-open").click();
    const row = dialog(page).getByTestId("details-ai-release");
    await expect(row.getByTestId("ai-release-status")).toHaveText("Blocked");
    const confirm = row.getByTestId("ai-release-confirm-intern");
    await expect(confirm).toBeDisabled();
    await row.getByRole("checkbox", { name: /informed/ }).check();
    await row.getByRole("combobox", { name: /Legal basis/ }).click();
    await page.getByRole("option", { name: /Legitimate interest/ }).click();
    await confirm.click();
    await expect(row.getByTestId("ai-release-status")).toHaveText(
      "Released (internal)",
    );
  });
});
