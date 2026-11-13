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

/** Attrappe der G9a-2-Commands; `stand` ist der Zustand je Besprechung. */
const installReleaseCommands = async (
  page: Page,
  init: { einstufung: string; externe?: string[]; ohne?: number },
) => {
  await page.addInitScript((cfg) => {
    const w = window as any;
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__settings.meeting_mcp_enabled = true;
    w.__settings.meeting_own_domains = w.__settings.meeting_own_domains ?? [];
    w.__settings.meeting_cloud_with_externals = false;
    w.__bestaetigung = null;
    const stand = () => {
      const extern = cfg.einstufung === "extern";
      const b = w.__bestaetigung;
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
          return stand();
        case "meeting_ai_release_confirm":
          w.__calls.push({ cmd, args });
          w.__bestaetigung = {
            einstufung: args.einstufung,
            fingerabdruck: args.fingerabdruck,
          };
          return stand();
        case "meeting_ai_release_revoke":
          w.__calls.push({ cmd, args });
          w.__bestaetigung = null;
          return stand();
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
    const toggle = page
      .getByText("Cloud-Modelle in Besprechungen mit Externen", {
        exact: true,
      })
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");
    await expect(toggle).not.toBeChecked();
    await toggle.evaluate((el) => (el as HTMLInputElement).click());
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
    await shoot(page, "02-details-gesperrt");
    await row.getByTestId("ai-release-confirm-intern").click();
    await expect(row.getByTestId("ai-release-status")).toHaveText(
      "Freigegeben (intern)",
    );
    const confirm = await calls(page, "meeting_ai_release_confirm");
    expect(confirm[0].args).toMatchObject({
      einstufung: "intern",
      fingerabdruck: FP,
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
    await row.getByTestId("ai-release-confirm-extern").click();
    await expect(row.getByTestId("ai-release-status")).toHaveText("Gesperrt");
    await expect(row.getByTestId("ai-release-reasons")).toContainText(
      "Externe dabei",
    );
    await shoot(page, "04-details-extern");
  });

  test("ohne MCP-Server keine Zeile", async ({ page }) => {
    await installLiveMock(page);
    await installReleaseCommands(page, { einstufung: "intern" });
    await page.addInitScript(() => {
      (window as any).__settings.meeting_mcp_enabled = false;
    });
    await open(page);
    await expect(dialog(page).getByTestId("details-ai-release")).toHaveCount(0);
  });
});
