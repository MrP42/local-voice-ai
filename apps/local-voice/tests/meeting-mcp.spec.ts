import { test, expect, type Page } from "@playwright/test";
import { calls, installTauriMock } from "./calendarMock";
import {
  MCP_CLIENTS,
  MCP_EXE_PLACEHOLDER,
  mcpSnippet,
} from "../src/lib/mcpSnippets";

// M6-P6e: Zeile "Lokaler MCP-Server (nur lesend)" (seit A4 auf der Seite
// Integrationen, vorher in der Gruppe Besprechungen; Schalter, Warnung, Transkript-Freigabe, Kopier-Schnipsel)
// gegen die Tauri-Attrappe, dazu die reine Schnipsel-Logik.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const EXE = "C:\\Program Files\\Local Voice AI\\local-voice-ai.exe";

/** Attrappe der drei neuen Commands (Aufruf im Aufrufprotokoll, Wirkung auf die Einstellungen). */
const installMcpCommands = async (page: Page) => {
  await page.addInitScript((exe) => {
    const w = window as any;
    const original = w.__TAURI_INTERNALS__.invoke;
    w.__clipboard = null;
    w.__clipboardFails = false;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          if (w.__clipboardFails) throw new Error("denied");
          w.__clipboard = text;
        },
      },
    });
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, unknown> = {},
    ) => {
      switch (cmd) {
        case "meeting_mcp_info":
          w.__calls.push({ cmd, args });
          return { exe_path: exe };
        case "change_meeting_mcp_enabled_setting":
          w.__calls.push({ cmd, args });
          w.__settings.meeting_mcp_enabled = args.enabled;
          return null;
        case "change_meeting_mcp_include_transcript_setting":
          w.__calls.push({ cmd, args });
          w.__settings.meeting_mcp_include_transcript = args.enabled;
          return null;
      }
      return original(cmd, args);
    };
  }, EXE);
};

const setup = async (page: Page, init?: () => void) => {
  await installTauriMock(page, "main");
  await installMcpCommands(page);
  if (init) await page.addInitScript(init);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/");
  // A4: der Schalter wohnt auf der Seite "Integrationen" (unter Einstellungen
  // > Besprechungen steht nur ein Verweis, siehe integrations.spec.ts).
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await expect(
    page.getByText("Lokaler MCP-Server (nur lesend)", { exact: true }),
  ).toBeVisible();
};

const switchOf = (page: Page, title: string) =>
  page
    .getByText(title, { exact: true })
    .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
    .locator("input[type=checkbox]");

const mainSwitch = (page: Page) =>
  switchOf(page, "Lokaler MCP-Server (nur lesend)");

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("MCP-Schnipsel Logik", () => {
  test("Claude Code: eine Zeile mit dem Pfad in Anführungszeichen", () => {
    expect(mcpSnippet("claude-code", EXE)).toBe(
      `claude mcp add local-voice -- "${EXE}" --mcp`,
    );
  });

  test("Claude Desktop: gültiges JSON mit Befehl und Argument", () => {
    const parsed = JSON.parse(mcpSnippet("claude-desktop", EXE));
    expect(parsed).toEqual({
      mcpServers: { "local-voice": { command: EXE, args: ["--mcp"] } },
    });
  });

  test("Codex: TOML mit wörtlicher Zeichenkette, sonst maskiert", () => {
    expect(mcpSnippet("codex", EXE)).toBe(
      [
        "[mcp_servers.local-voice]",
        `command = '${EXE}'`,
        'args = ["--mcp"]',
      ].join("\n"),
    );
    const odd = "C:\\Users\\O'Neil\\local-voice-ai.exe";
    expect(mcpSnippet("codex", odd)).toContain(
      'command = "C:\\\\Users\\\\O\'Neil\\\\local-voice-ai.exe"',
    );
  });

  test("ohne bekannten Pfad steht ein Platzhalter da", () => {
    for (const client of MCP_CLIENTS) {
      expect(mcpSnippet(client, null)).toContain(
        client === "claude-desktop"
          ? MCP_EXE_PLACEHOLDER.replace(/\\/g, "\\\\")
          : MCP_EXE_PLACEHOLDER,
      );
      expect(mcpSnippet(client, "  ")).toContain(MCP_EXE_PLACEHOLDER);
    }
  });
});

// ---------------------------------------------------------------------------
// Oberfläche
// ---------------------------------------------------------------------------

test.describe("Lokaler MCP-Server auf der Seite Integrationen", () => {
  test("Standard: aus, ohne Warnung und ohne Schnipsel", async ({ page }) => {
    await setup(page);
    await expect(mainSwitch(page)).not.toBeChecked();
    await expect(page.getByTestId("mcp-settings")).toHaveCount(0);
    await expect(page.getByText("Transkript freigeben")).toHaveCount(0);
    // Beim Öffnen wird nichts geschaltet.
    expect(
      await calls(page, "change_meeting_mcp_enabled_setting"),
    ).toHaveLength(0);
  });

  test("Einschalten ruft den Command, zeigt Warnung, Transkript-Schalter und Schnipsel", async ({
    page,
  }) => {
    await setup(page);
    await mainSwitch(page).evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_mcp_enabled_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_mcp_enabled_setting"))[0].args,
    ).toEqual({
      enabled: true,
    });
    await expect(page.getByTestId("mcp-warning")).toContainText(
      "sendet gelesene Besprechungsinhalte an seinen Anbieter",
    );
    await expect(page.getByTestId("mcp-warning")).toContainText(
      "Aussagen Dritter",
    );
    await expect(page.getByTestId("mcp-snippet")).toHaveText(
      `claude mcp add local-voice -- "${EXE}" --mcp`,
    );
    // Transkript-Freigabe ist vorangehakt und schaltbar.
    const transcript = switchOf(page, "Transkript freigeben");
    await expect(transcript).toBeChecked();
    await transcript.evaluate((el) => (el as HTMLInputElement).click());
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_mcp_include_transcript_setting"))
            .length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_mcp_include_transcript_setting"))[0]
        .args,
    ).toEqual({ enabled: false });
  });

  test("Ausschalten blendet Warnung und Schnipsel wieder aus", async ({
    page,
  }) => {
    await setup(page, () => {
      (window as any).__settings.meeting_mcp_enabled = true;
    });
    await expect(mainSwitch(page)).toBeChecked();
    await expect(page.getByTestId("mcp-settings")).toBeVisible();
    await mainSwitch(page).evaluate((el) => (el as HTMLInputElement).click());
    await expect(page.getByTestId("mcp-settings")).toHaveCount(0);
    expect(
      (await calls(page, "change_meeting_mcp_enabled_setting"))[0].args,
    ).toEqual({
      enabled: false,
    });
  });

  test("Client wählen und Schnipsel kopieren", async ({ page }) => {
    await setup(page, () => {
      (window as any).__settings.meeting_mcp_enabled = true;
    });
    const snippet = page.getByTestId("mcp-snippet");
    await expect(snippet).toContainText("claude mcp add local-voice");

    await page.getByTestId("mcp-client-claude-desktop").click();
    await expect(snippet).toContainText('"mcpServers"');
    await expect(snippet).toContainText('"--mcp"');
    await page.getByTestId("mcp-client-codex").click();
    await expect(snippet).toContainText("[mcp_servers.local-voice]");

    await page.getByTestId("mcp-copy").click();
    await expect(page.getByTestId("mcp-copy-status")).toHaveText("Kopiert.");
    expect(await page.evaluate(() => (window as any).__clipboard)).toBe(
      mcpSnippet("codex", EXE),
    );
  });

  test("Kopieren schlägt fehl: Hinweis statt Stille", async ({ page }) => {
    await setup(page, () => {
      (window as any).__settings.meeting_mcp_enabled = true;
      (window as any).__clipboardFails = true;
    });
    await page.getByTestId("mcp-copy").click();
    await expect(page.getByTestId("mcp-copy-status")).toContainText(
      "Kopieren nicht möglich",
    );
    expect(await page.evaluate(() => (window as any).__clipboard)).toBeNull();
  });
});
