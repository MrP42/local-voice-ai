import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { calls, installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";
import {
  installAgentClientsMock,
  PIPE,
  type AgentClientsMockOptions,
} from "./agentClientsMock";

// A7 (Goal Integrationen, #66): die Zugangsliste der Agentenbrücke im Detail der
// Agent-Integration: Zugang anlegen (Schlüssel genau einmal, mit Kopieren), Liste mit
// Zustand und letzter Nutzung, zurückziehen und entfernen, Werkzeugrechte je Zugang (Aufnahme
// nie „Erlaubt“) mit dem Grund, wenn die Obergrenze der Integration „Aus“ steht, und der
// Zustand der Brücke. Gegen die Tauri-Attrappe; Token, Pipe und die Rechenregel prüfen die
// Rust-Tests (`cargo test --lib agent_bridge::`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const AGENT = [{ id: "agent-1", kind: "agent", label: "Externe Agenten" }];

const setup = async (page: Page, agents: AgentClientsMockOptions = {}) => {
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, { integrations: AGENT });
  await installAgentClientsMock(page, agents);
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.goto("/");
};

const openAgentDetail = async (page: Page) => {
  await page
    .getByRole("navigation")
    .getByRole("button", { name: "Integrationen", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Integrationen", level: 1 }),
  ).toBeVisible();
  await page
    .locator('[data-testid="integration-card"][data-integration-id="agent-1"]')
    .getByTestId("integration-open")
    .click();
  await expect(page.getByTestId("agent-clients")).toBeVisible();
};

const clientCard = (page: Page, id: string) =>
  page.locator(`[data-testid="agent-client"][data-client-id="${id}"]`);

const tool = (page: Page, id: string, name: string, mode: string) =>
  page.getByTestId(`agent-tool-${id}-${name}-${mode}`);

const FRESH = {
  id: "C-1",
  label: "Claude Code",
};

test.describe("Zugänge der Agentenbrücke", () => {
  test("der Platzhalter ist weg: Brückenstatus, leere Liste und Knopf zum Anlegen", async ({
    page,
  }) => {
    await setup(page);
    await openAgentDetail(page);
    await expect(page.getByTestId("agent-clients-placeholder")).toHaveCount(0);
    const status = page.getByTestId("agent-bridge-status");
    await expect(status).toHaveAttribute("data-running", "true");
    await expect(status).toContainText("Agentenbrücke läuft");
    await expect(status).toContainText(PIPE);
    await expect(status).toContainText("nicht über das Netzwerk");
    await expect(page.getByTestId("agent-clients-empty")).toContainText(
      "Noch kein Zugang",
    );
    await expect(page.getByTestId("agent-client-create-open")).toBeVisible();
  });

  test("läuft die Brücke nicht, steht der Grund da", async ({ page }) => {
    await setup(page, {
      bridge: {
        running: false,
        pipe_name: PIPE,
        error: "Zugriff verweigert (os error 5)",
      },
    });
    await openAgentDetail(page);
    const status = page.getByTestId("agent-bridge-status");
    await expect(status).toHaveAttribute("data-running", "false");
    await expect(status).toContainText("Agentenbrücke läuft nicht");
    await expect(page.getByTestId("agent-bridge-error")).toContainText(
      "Zugriff verweigert",
    );
  });

  test("Zugang anlegen: der Schlüssel erscheint genau einmal und lässt sich kopieren", async ({
    page,
    context,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await setup(page);
    await openAgentDetail(page);

    await page.getByTestId("agent-client-create-open").click();
    const submit = page.getByTestId("agent-client-create-submit");
    await expect(submit).toBeDisabled();
    await page.getByTestId("agent-client-name").fill("Claude Code");
    await submit.click();

    const card = page.getByTestId("agent-token-card");
    await expect(card).toBeVisible();
    await expect(card).toContainText("Schlüssel für „Claude Code“");
    await expect(page.getByTestId("agent-token-warning")).toContainText(
      "nur jetzt angezeigt",
    );
    const token = (
      await page.getByTestId("agent-token-value").innerText()
    ).trim();
    expect(token).toMatch(/^lvat_[A-Za-z0-9_-]{43}$/);

    await page.getByTestId("agent-token-copy").click();
    await expect(page.getByTestId("agent-token-copy")).toContainText("Kopiert");
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
      token,
    );

    // Der Zugang steht in der Liste, ohne den Schlüssel.
    const row = page.getByTestId("agent-client");
    await expect(row).toHaveCount(1);
    await expect(row).toContainText("Claude Code");
    await expect(page.getByTestId("agent-client-status")).toHaveText("Aktiv");
    await expect(page.getByTestId("agent-client-last-used")).toHaveText(
      "Noch nie benutzt",
    );
    await expect(row).not.toContainText(token);

    const created = await calls(page, "agent_client_create");
    expect(created).toHaveLength(1);
    expect(created[0].args).toMatchObject({
      label: "Claude Code",
      integrationId: "agent-1",
    });

    // „Fertig“ schließt die Karte; der Schlüssel ist weg und nach dem Neuladen nirgends mehr.
    await page.getByTestId("agent-token-done").click();
    await expect(page.getByTestId("agent-token-card")).toHaveCount(0);
    await page.reload();
    await openAgentDetail(page);
    await expect(page.getByTestId("agent-client")).toHaveCount(1);
    await expect(page.getByTestId("agent-token-value")).toHaveCount(0);
    await expect(page.locator("body")).not.toContainText(token);
  });

  test("scheitert das Kopieren, steht ein Hinweis zum Kopieren von Hand da", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", {
        value: {
          writeText: () => Promise.reject(new Error("verweigert")),
        },
        configurable: true,
      });
    });
    await setup(page);
    await openAgentDetail(page);
    await page.getByTestId("agent-client-create-open").click();
    await page.getByTestId("agent-client-name").fill("Codex");
    await page.getByTestId("agent-client-create-submit").click();
    await page.getByTestId("agent-token-copy").click();
    await expect(page.getByTestId("agent-token-copy-failed")).toContainText(
      "von Hand",
    );
  });

  test("ein Fehler des Backends steht im Klartext da", async ({ page }) => {
    await setup(page, {
      clients: Array.from({ length: 20 }, (_, i) => ({
        id: `C-${i}`,
        label: `Zugang ${i}`,
      })),
    });
    await openAgentDetail(page);
    await page.getByTestId("agent-client-create-open").click();
    await page.getByTestId("agent-client-name").fill("Einer zu viel");
    await page.getByTestId("agent-client-create-submit").click();
    await expect(page.getByTestId("agent-clients-error")).toContainText(
      "Es gibt schon 20 aktive Zugänge",
    );
    await expect(page.getByTestId("agent-token-card")).toHaveCount(0);
  });

  test("Anlegen lässt sich abbrechen", async ({ page }) => {
    await setup(page);
    await openAgentDetail(page);
    await page.getByTestId("agent-client-create-open").click();
    await page.getByTestId("agent-client-name").fill("Nichts");
    await page.getByTestId("agent-client-create-cancel").click();
    await expect(page.getByTestId("agent-client-form")).toHaveCount(0);
    expect(await calls(page, "agent_client_create")).toHaveLength(0);
  });

  test("die Liste zeigt Zustand und letzte Nutzung", async ({ page }) => {
    await setup(page, {
      clients: [
        { ...FRESH, last_used_at: NOW - 3_600_000 },
        { id: "C-2", label: "Skript", created_at: NOW - 172_800_000 },
        {
          id: "C-3",
          label: "Alter Zugang",
          revoked_at: NOW - 60_000,
          last_used_at: NOW - 7_200_000,
        },
      ],
    });
    await openAgentDetail(page);
    await expect(page.getByTestId("agent-client")).toHaveCount(3);
    await expect(
      clientCard(page, "C-1").getByTestId("agent-client-last-used"),
    ).toContainText("Zuletzt benutzt");
    await expect(
      clientCard(page, "C-2").getByTestId("agent-client-last-used"),
    ).toHaveText("Noch nie benutzt");
    await expect(
      clientCard(page, "C-3").getByTestId("agent-client-status"),
    ).toHaveText("Zurückgezogen");
    await expect(clientCard(page, "C-3")).toContainText("Zurückgezogen");
    await expect(
      clientCard(page, "C-3").getByTestId("agent-client-revoke"),
    ).toHaveCount(0);
  });

  test("Werkzeugrechte: alles aus; Fragen wirkt erst, wenn die Obergrenze es zulässt", async ({
    page,
  }) => {
    await setup(page, { clients: [FRESH] });
    await openAgentDetail(page);
    const c = clientCard(page, "C-1");
    // Ein neuer Zugang: jedes Werkzeug auf „Aus“.
    for (const name of [
      "add_youtube_source",
      "start_recording",
      "transcribe_file",
      "tts_render_audio",
    ]) {
      await expect(tool(page, "C-1", name, "off")).toHaveAttribute(
        "aria-checked",
        "true",
      );
    }
    // Die Texte der Werkzeuge kommen aus der Übersetzung, nicht aus dem Backend.
    await expect(c).toContainText("Datei transkribieren");
    await expect(c).toContainText("Legt eine leere Besprechung an.");

    // Auf „Fragen“ gestellt, aber die Obergrenze (Rechte-Matrix) steht auf „Aus“: der Grund steht da.
    await tool(page, "C-1", "transcribe_file", "ask").click();
    await expect(tool(page, "C-1", "transcribe_file", "ask")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(
      (await calls(page, "agent_client_set_tool_mode")).map((x) => x.args),
    ).toEqual([{ clientId: "C-1", tool: "transcribe_file", mode: "ask" }]);
    const row = page.getByTestId("agent-tool-C-1-transcribe_file");
    await expect(row.getByTestId("agent-tool-off-reason")).toHaveAttribute(
      "data-reason",
      "grant_off",
    );
    await expect(row.getByTestId("agent-tool-off-reason")).toContainText(
      "Obergrenze der Integration steht auf „Aus“",
    );

    // Obergrenze in der Rechte-Matrix auf „Fragen“: der Grund verschwindet.
    await page.getByTestId("grant-transcribe.file-agent_external-ask").click();
    await page.reload();
    await openAgentDetail(page);
    await expect(
      page
        .getByTestId("agent-tool-C-1-transcribe_file")
        .getByTestId("agent-tool-off-reason"),
    ).toHaveCount(0);

    // Zugang „Erlaubt“, Obergrenze nur „Fragen“: wirksam „Fragen“.
    await tool(page, "C-1", "transcribe_file", "allow").click();
    await expect(
      page
        .getByTestId("agent-tool-C-1-transcribe_file")
        .getByTestId("agent-tool-effective"),
    ).toContainText("Wirkt jetzt: Fragen");

    // Nach dem Neuladen zählt der Aufruf-Verlauf neu: nur das „Erlaubt“.
    expect(
      (await calls(page, "agent_client_set_tool_mode")).map((x) => x.args),
    ).toEqual([{ clientId: "C-1", tool: "transcribe_file", mode: "allow" }]);
  });

  test("Aufnahme lässt sich nie auf „Erlaubt“ stellen", async ({ page }) => {
    await setup(page, { clients: [FRESH] });
    await openAgentDetail(page);
    for (const name of ["start_recording", "stop_recording"]) {
      await expect(tool(page, "C-1", name, "allow")).toBeDisabled();
      await expect(tool(page, "C-1", name, "ask")).toBeEnabled();
    }
    await expect(
      page.getByTestId("agent-tool-C-1-start_recording"),
    ).toContainText("Einwilligungsdialog");
    await tool(page, "C-1", "start_recording", "ask").click();
    await expect(tool(page, "C-1", "start_recording", "ask")).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  test("ein Werkzeug, das es noch nicht gibt, trägt den Hinweis", async ({
    page,
  }) => {
    await setup(page, { clients: [FRESH], available: ["create_meeting"] });
    await openAgentDetail(page);
    await expect(
      page
        .getByTestId("agent-tool-C-1-transcribe_file")
        .getByTestId("agent-tool-unavailable"),
    ).toHaveText("Noch nicht verfügbar");
    await expect(
      page
        .getByTestId("agent-tool-C-1-create_meeting")
        .getByTestId("agent-tool-unavailable"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("agent-tools-unavailable-note"),
    ).toContainText("gibt es in dieser Version noch nicht");
  });

  test("Zurückziehen fragt nach und friert die Rechte ein", async ({
    page,
  }) => {
    await setup(page, {
      clients: [{ ...FRESH, tools: { create_meeting: "ask" } }],
    });
    await openAgentDetail(page);
    const c = clientCard(page, "C-1");
    await c.getByTestId("agent-client-revoke").click();
    await expect(c.getByTestId("agent-client-confirm")).toContainText(
      "gilt sofort nicht mehr",
    );
    // Nein: nichts geschieht.
    await c.getByTestId("agent-client-confirm-no").click();
    await expect(c.getByTestId("agent-client-status")).toHaveText("Aktiv");
    expect(await calls(page, "agent_client_revoke")).toHaveLength(0);
    // Ja.
    await c.getByTestId("agent-client-revoke").click();
    await c.getByTestId("agent-client-confirm-yes").click();
    await expect(c.getByTestId("agent-client-status")).toHaveText(
      "Zurückgezogen",
    );
    await expect(c.getByTestId("agent-client-revoke")).toHaveCount(0);
    await expect(tool(page, "C-1", "create_meeting", "allow")).toBeDisabled();
    await expect(tool(page, "C-1", "create_meeting", "off")).toBeDisabled();
    await expect(c).toContainText("Rechte sind eingefroren");
    expect(
      (await calls(page, "agent_client_revoke")).map((x) => x.args),
    ).toEqual([{ id: "C-1" }]);
  });

  test("Entfernen fragt nach und nimmt den Zugang aus der Liste", async ({
    page,
  }) => {
    await setup(page, {
      clients: [FRESH, { id: "C-2", label: "Codex" }],
    });
    await openAgentDetail(page);
    await clientCard(page, "C-1").getByTestId("agent-client-delete").click();
    await expect(
      clientCard(page, "C-1").getByTestId("agent-client-confirm"),
    ).toContainText("samt Werkzeugrechten entfernen");
    await clientCard(page, "C-1")
      .getByTestId("agent-client-confirm-yes")
      .click();
    await expect(page.getByTestId("agent-client")).toHaveCount(1);
    await expect(clientCard(page, "C-2")).toBeVisible();
    await page.reload();
    await openAgentDetail(page);
    await expect(page.getByTestId("agent-client")).toHaveCount(1);
  });

  test("andere Integrationen zeigen keine Zugangsliste", async ({ page }) => {
    await page.clock.setFixedTime(NOW);
    await installTauriMock(page, "main");
    await installIntegrationsMock(page, {
      integrations: [
        ...AGENT,
        { id: "ordner-1", kind: "folder", label: "Ablage", path: "C:\\Ablage" },
      ],
    });
    await installAgentClientsMock(page, { clients: [FRESH] });
    await page.setViewportSize({ width: 1280, height: 1100 });
    await page.goto("/");
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Integrationen", exact: true })
      .click();
    await page
      .locator(
        '[data-testid="integration-card"][data-integration-id="ordner-1"]',
      )
      .getByTestId("integration-open")
      .click();
    await expect(page.getByTestId("integration-detail")).toBeVisible();
    await expect(page.getByTestId("agent-clients")).toHaveCount(0);
  });

  test("schmal ohne Überlauf und ohne schwere Zugänglichkeitsfehler", async ({
    page,
  }) => {
    await setup(page, {
      clients: [{ ...FRESH, last_used_at: NOW - 60_000 }],
    });
    await page.setViewportSize({ width: 390, height: 900 });
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Mehr" })
      .click();
    await page
      .getByRole("navigation")
      .getByRole("button", { name: "Integrationen", exact: true })
      .click();
    await page
      .locator(
        '[data-testid="integration-card"][data-integration-id="agent-1"]',
      )
      .getByTestId("integration-open")
      .click();
    await expect(page.getByTestId("agent-clients")).toBeVisible();
    // Kein waagerechtes Scrollen der Seite.
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
    const result = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa"])
      .include('[data-testid="agent-clients"]')
      .analyze();
    const bad = result.violations
      .filter((v) => v.impact === "critical" || v.impact === "serious")
      .filter((v) => v.id !== "color-contrast")
      .map(
        (v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`,
      );
    expect(bad).toEqual([]);
  });
});
