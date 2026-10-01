import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { installTauriMock } from "./calendarMock";
import { installIntegrationsMock, NOW } from "./integrationsMock";
import {
  installAgentClientsMock,
  type AgentClientsMockOptions,
} from "./agentClientsMock";

// A7 (#66, AK12 Bündel 2): Bilder der Zugangsliste der Agentenbrücke für die Abnahme:
// Liste, Schlüssel (einmal), Werkzeugrechte mit dem Grund der Obergrenze, zurückgezogener
// Zugang, Brücke aus, schmal. Nur mit LVA_SCREENS=1, sonst übersprungen; die Bilder landen in
// koordination/integrationen/screens-a7/.

test.skip(!process.env.LVA_SCREENS, "Bilder nur mit LVA_SCREENS=1");
test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const OUT = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../../koordination/integrationen/screens-a7",
);

const shot = async (page: Page, name: string) => {
  mkdirSync(OUT, { recursive: true });
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await page.waitForTimeout(250);
  await page.screenshot({
    path: resolve(OUT, `${name}.png`),
    animations: "disabled",
  });
};

const setup = async (
  page: Page,
  agents: AgentClientsMockOptions,
  width = 1280,
  height = 1500,
) => {
  const narrow = width < 768;
  await page.clock.setFixedTime(NOW);
  await installTauriMock(page, "main");
  await installIntegrationsMock(page, {
    integrations: [
      {
        id: "agent-1",
        kind: "agent",
        label: "Externe Agenten",
        grants: { "transcribe.file|agent_external": "ask" },
      },
    ],
  });
  await installAgentClientsMock(page, agents);
  await page.setViewportSize({ width, height });
  await page.goto("/");
  const nav = page.getByRole("navigation");
  if (narrow) await nav.getByRole("button", { name: "Mehr" }).click();
  await nav.getByRole("button", { name: "Integrationen", exact: true }).click();
  await page
    .locator('[data-testid="integration-card"][data-integration-id="agent-1"]')
    .getByTestId("integration-open")
    .click();
  await expect(page.getByTestId("agent-clients")).toBeVisible();
  await page.getByTestId("agent-clients").scrollIntoViewIfNeeded();
};

const CLIENTS = [
  {
    id: "C-1",
    label: "Claude Code",
    last_used_at: NOW - 1_800_000,
    tools: { transcribe_file: "allow", create_meeting: "allow" },
  },
  {
    id: "C-2",
    label: "Codex",
    tools: { transcribe_file: "ask", start_recording: "ask" },
  },
  {
    id: "C-3",
    label: "Altes Skript",
    revoked_at: NOW - 3_600_000,
    last_used_at: NOW - 86_400_000,
    tools: { create_meeting: "ask" },
  },
];

test("Bilder Zugänge der Agentenbrücke", async ({ page }) => {
  await setup(page, { clients: CLIENTS });
  await shot(page, "1-zugangsliste");

  await page.getByTestId("agent-client-create-open").click();
  await page.getByTestId("agent-client-name").fill("Mein Skript");
  await page.getByTestId("agent-client-create-submit").click();
  await expect(page.getByTestId("agent-token-card")).toBeVisible();
  await page.getByTestId("agent-token-card").scrollIntoViewIfNeeded();
  await shot(page, "2-schluessel-einmal");
});

test("Bilder Werkzeugrechte und Brücke aus", async ({ page }) => {
  await setup(
    page,
    {
      clients: [
        {
          id: "C-2",
          label: "Codex",
          tools: { transcribe_file: "ask", create_meeting: "allow" },
        },
      ],
      bridge: {
        running: false,
        pipe_name: "\\\\.\\pipe\\local-voice-ai-agent-0123456789abcdef",
        error: "Zugriff verweigert (os error 5)",
      },
    },
    1280,
    1900,
  );
  await page.getByTestId("agent-clients").scrollIntoViewIfNeeded();
  await shot(page, "3-werkzeugrechte-und-bruecke-aus");
});

test("Bilder Zugänge schmal", async ({ page }) => {
  await setup(page, { clients: CLIENTS }, 390, 1900);
  await shot(page, "4-schmal");
});
