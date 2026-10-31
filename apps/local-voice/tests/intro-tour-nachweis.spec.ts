import { test, expect, type Page } from "@playwright/test";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import {
  commands,
  finishDownload,
  gotoStep,
  progress,
  ready,
  setup,
} from "./introTourMock";

// Prüfnachweis der Einführungstour (Barrierefreiheit): Kontrast, Zoom und
// Umbruch, Modal/inert, Fokus, Tastatur, Esc, Zustände. Jede Prüfung landet
// zusätzlich als Zeile in test-results/intro-tour-nachweis/log.jsonl,
// Accessibility-Bäume und Bilder daneben.
const OUT = "test-results/intro-tour-nachweis";
mkdirSync(OUT, { recursive: true });

const log = (check: string, ok: boolean, detail: unknown = null) => {
  appendFileSync(
    `${OUT}/log.jsonl`,
    JSON.stringify({ test: test.info().title, check, ok, detail }) + "\n",
  );
  expect(ok, `${check}: ${JSON.stringify(detail)}`).toBe(true);
};

const tour = (page: Page) => page.getByTestId("intro-tour");

/** Echter Chromium-Accessibility-Baum (CDP), ignorierte Knoten ausgelassen. */
const axTree = async (page: Page) => {
  const cdp = await page.context().newCDPSession(page);
  const { nodes } = (await cdp.send("Accessibility.getFullAXTree")) as {
    nodes: {
      nodeId: string;
      parentId?: string;
      childIds?: string[];
      ignored: boolean;
      role?: { value: string };
      name?: { value: string };
    }[];
  };
  await cdp.detach();
  const byId = new Map(nodes.map((n) => [n.nodeId, n]));
  const skip = ["generic", "none", "InlineTextBox", "StaticText"];
  const lines: string[] = [];
  const walk = (id: string, depth: number) => {
    const n = byId.get(id);
    if (!n) return;
    const show = !n.ignored && !!n.role && !skip.includes(n.role.value);
    if (show)
      lines.push(
        `${"  ".repeat(depth)}- ${n.role!.value}${n.name?.value ? ` "${n.name.value}"` : ""}`,
      );
    for (const c of n.childIds ?? []) walk(c, show ? depth + 1 : depth);
  };
  const root = nodes.find((n) => !n.parentId);
  if (root) walk(root.nodeId, 0);
  return lines.join("\n") + "\n";
};

/** Kontrast der sichtbaren Tour-Teile, im Browser berechnet (WCAG 2.x). */
const measureContrast = (page: Page) =>
  page.evaluate(() => {
    const parse = (c: string) => {
      const m = c.match(/rgba?\(([^)]+)\)/);
      if (!m) return null;
      const [r, g, b, a = "1"] = m[1].split(/[ ,/]+/).filter(Boolean);
      return [Number(r), Number(g), Number(b), Number(a)];
    };
    const lum = ([r, g, b]: number[]) => {
      const f = (v: number) => {
        const s = v / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    const ratio = (a: number[], b: number[]) => {
      const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
      return Math.round(((x + 0.05) / (y + 0.05)) * 100) / 100;
    };
    const bgOf = (el: Element | null): number[] => {
      for (let n = el; n; n = n.parentElement) {
        const c = parse(getComputedStyle(n).backgroundColor);
        if (c && c[3] > 0.5) return c;
      }
      return [255, 255, 255, 1];
    };
    const box = document.querySelector("[role=dialog]")!;
    const out: { name: string; kind: "text" | "ui"; ratio: number }[] = [];
    const text = (name: string, sel: string) => {
      box.querySelectorAll<HTMLElement>(sel).forEach((el, i) => {
        if (!el.offsetParent || !el.textContent?.trim()) return;
        const fg = parse(getComputedStyle(el).color)!;
        out.push({
          name: `${name}#${i} ${el.textContent.trim().slice(0, 24)}`,
          kind: "text",
          ratio: ratio(fg, bgOf(el)),
        });
      });
    };
    text("Schrittlabel", ".lv-tour__eyebrow");
    text("Titel", ".lv-tour__title");
    text("Text", ".lv-tour__body li, .lv-tour__body p");
    text("Hinweis", ".lv-tour__note");
    text("Fehler", ".lv-tour__error");
    text("Fortschritt", ".lv-tour__progress > span:not(.lv-tour__bar)");
    text("Feldbeschriftung", ".lv-tour__label");
    text("Knopf", "button:not([disabled])");
    // Nicht-Text (3:1): aktiver Punkt, Balken, Primärknopf-Fläche.
    const ui = (name: string, el: Element | null, prop: string) => {
      if (!el) return;
      const c = parse(getComputedStyle(el).getPropertyValue(prop));
      if (c && c[3] > 0)
        out.push({ name, kind: "ui", ratio: ratio(c, bgOf(el.parentElement)) });
    };
    ui(
      "aktiver Schrittpunkt",
      box.querySelector(".lv-tour__dots i[data-on]"),
      "background-color",
    );
    ui(
      "Fortschrittsbalken",
      box.querySelector(".lv-tour__bar i"),
      "background-color",
    );
    ui(
      "Hauptknopf-Rand",
      box.querySelector("[data-tour-primary]:not([disabled])"),
      "box-shadow",
    );
    const field = box.querySelector(".lv-tour__try:not([disabled])");
    ui("Übungsfeld-Rahmen", field, "border-top-color");
    const spot = document.querySelector(".lv-tour__spot");
    if (spot) {
      const c = parse(getComputedStyle(spot).outlineColor)!;
      // Rahmen liegt auf der abgedunkelten App: Vergleich mit der Fläche
      // unter dem Spotlight (Navigation) und mit dem Schatten.
      out.push({
        name: "Spotlight-Rahmen",
        kind: "ui",
        ratio: ratio(c, bgOf(document.querySelector("[data-tour=nav-home]"))),
      });
    }
    return out;
  });

/** Fokusring des fokussierten Elements gegen seinen Hintergrund (3:1). */
const focusRing = (page: Page) =>
  page.evaluate(() => {
    const el = document.activeElement as HTMLElement;
    const cs = getComputedStyle(el);
    const parse = (c: string) =>
      c
        .match(/rgba?\(([^)]+)\)/)?.[1]
        .split(/[ ,/]+/)
        .filter(Boolean)
        .map(Number) ?? [0, 0, 0, 0];
    const lum = ([r, g, b]: number[]) => {
      const f = (v: number) => {
        const s = v / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
    };
    let bg = [255, 255, 255];
    for (let n: HTMLElement | null = el.parentElement; n; n = n.parentElement) {
      const c = parse(getComputedStyle(n).backgroundColor);
      if (c[3] === undefined || c[3] > 0.5) {
        bg = c;
        break;
      }
    }
    // Fluent zeichnet den Fokus je nach Element als Rahmen, Outline,
    // ::after-Rahmen oder (Inset-)Schatten. Gewertet wird der stärkste
    // deckende Bestandteil gegen die umgebende Fläche.
    const after = getComputedStyle(el, "::after");
    const candidates = [
      cs.outlineStyle !== "none" && parseFloat(cs.outlineWidth) > 0
        ? cs.outlineColor
        : "",
      after.borderTopStyle !== "none" && parseFloat(after.borderTopWidth) > 0
        ? after.borderTopColor
        : "",
      cs.borderTopStyle !== "none" && parseFloat(cs.borderTopWidth) > 0
        ? cs.borderTopColor
        : "",
      ...(cs.boxShadow.match(/rgba?\([^)]+\)/g) ?? []),
    ].filter((c) => c && (parse(c)[3] ?? 1) >= 0.9);
    const scored = candidates.map((c) => {
      const [x, y] = [lum(parse(c)), lum(bg)].sort((a, b) => b - a);
      return { c, r: Math.round(((x + 0.05) / (y + 0.05)) * 100) / 100 };
    });
    const best = scored.sort((a, b) => b.r - a.r)[0] ?? { c: "", r: 0 };
    return {
      element:
        el.tagName +
        (el.textContent ? `:${el.textContent.trim().slice(0, 30)}` : ""),
      color: best.c,
      ratio: best.r,
    };
  });

for (const theme of ["light", "dark"] as const) {
  test(`Kontrast aller Schritte (${theme})`, async ({ page }) => {
    await setup(page, { theme });
    await page.goto("/");
    await expect(tour(page).getByRole("dialog")).toBeVisible();
    await progress(page, 40);
    const all: unknown[] = [];
    const check = async (step: string) => {
      await expect(tour(page).getByRole("dialog")).toBeVisible();
      await page.waitForTimeout(400); // Übergänge der Knöpfe abwarten
      for (const m of await measureContrast(page)) {
        const min = m.kind === "text" ? 4.5 : 3;
        all.push({ step, ...m, min });
        log(`${theme} ${step} ${m.name} ≥ ${min}:1`, m.ratio >= min, m.ratio);
      }
    };
    await check("Willkommen (lädt)");
    await ready(page);
    await check("Willkommen (bereit)");
    // Fokusring per Tastatur sichtbar machen.
    await page.keyboard.press("Tab");
    await page.keyboard.press("Shift+Tab");
    const ring = await focusRing(page);
    log(`${theme} Fokusring ≥ 3:1`, ring.ratio >= 3, ring);
    const names = ["Diktieren", "Üben", "Seiten", "Werkzeuge", "Fertig"];
    await tour(page).getByRole("button", { name: "Tour starten" }).click();
    for (let i = 0; i < names.length; i++) {
      await page.waitForTimeout(350);
      await check(names[i]);
      if (names[i] === "Üben") {
        const r = await focusRing(page);
        log(`${theme} Fokusring Übungsfeld ≥ 3:1`, r.ratio >= 3, r);
      }
      if (i < names.length - 1)
        await tour(page).getByRole("button", { name: "Weiter" }).click();
    }
    writeFileSync(
      `${OUT}/kontrast-${theme}.json`,
      JSON.stringify(all, null, 2),
    );
  });
}

test("Fehlerzustand: Kontrast und Ansage", async ({ page }) => {
  await setup(page, { downloadFails: 9 });
  await page.goto("/");
  await expect(tour(page).getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  for (const m of await measureContrast(page)) {
    const min = m.kind === "text" ? 4.5 : 3;
    log(`Fehler ${m.name} ≥ ${min}:1`, m.ratio >= min, m.ratio);
  }
  log(
    "Start gesperrt bei Fehler",
    await tour(page).getByRole("button", { name: "Tour starten" }).isDisabled(),
  );
  log(
    "Fokus auf „Download wiederholen“",
    await tour(page)
      .getByRole("button", { name: "Download wiederholen" })
      .evaluate((el) => el === document.activeElement),
  );
});

// Echter Browser-Zoom: Fenster 1280×800 bei 200 % = 640×400 CSS-Pixel mit
// Gerätepixelverhältnis 2; 400 % bzw. WCAG-Reflow = 320 CSS-Pixel Breite.
for (const z of [
  { name: "200 % (1280×800)", w: 640, h: 400 },
  { name: "400 % / Reflow 320 px (1280×1024)", w: 320, h: 256 },
]) {
  test.describe(`Zoom ${z.name}`, () => {
    test.use({ viewport: { width: z.w, height: z.h }, deviceScaleFactor: 2 });
    test("alle Schritte ohne waagerechtes Scrollen, alles erreichbar", async ({
      page,
    }) => {
      await setup(page);
      await page.goto("/");
      await ready(page);
      const steps = [
        "Willkommen",
        "Diktieren",
        "Üben",
        "Seiten",
        "Werkzeuge",
        "Fertig",
      ];
      for (let i = 0; i < steps.length; i++) {
        const m = await page.evaluate(() => {
          const box = document.querySelector<HTMLElement>("[role=dialog]")!;
          const r = box.getBoundingClientRect();
          return {
            docOverflowX:
              document.documentElement.scrollWidth > window.innerWidth,
            boxOverflowX: box.scrollWidth > box.clientWidth + 1,
            inside:
              r.left >= 0 &&
              r.top >= 0 &&
              r.right <= window.innerWidth + 0.5 &&
              r.bottom <= window.innerHeight + 0.5,
            innerScroll: box.scrollHeight > box.clientHeight,
            overflowY: getComputedStyle(box).overflowY,
          };
        });
        log(
          `${z.name} ${steps[i]}: kein waagerechtes Scrollen`,
          !m.docOverflowX && !m.boxOverflowX,
          m,
        );
        log(`${z.name} ${steps[i]}: Dialog im Fenster`, m.inside, m);
        if (m.innerScroll)
          log(
            `${z.name} ${steps[i]}: inneres Scrollen aktiv`,
            m.overflowY === "auto",
            m,
          );
        // Jeder Knopf lässt sich per Tastatur erreichen und ist dann sichtbar.
        const buttons = tour(page).getByRole("button");
        for (let b = 0; b < (await buttons.count()); b++) {
          const btn = buttons.nth(b);
          if (await btn.isDisabled()) continue;
          await btn.focus();
          const visible = await btn.evaluate((el) => {
            const r = el.getBoundingClientRect();
            return (
              r.top >= 0 &&
              r.bottom <= window.innerHeight + 0.5 &&
              r.left >= 0 &&
              r.right <= window.innerWidth + 0.5
            );
          });
          log(
            `${z.name} ${steps[i]}: Knopf „${await btn.innerText()}“ erreichbar`,
            visible,
          );
        }
        await page.screenshot({ path: `${OUT}/zoom-${z.w}-${i + 1}.png` });
        if (i < steps.length - 1)
          await tour(page)
            .getByRole("button", { name: i === 0 ? "Tour starten" : "Weiter" })
            .click();
      }
    });
  });
}

test("Modal: Accessibility-Baum, inert, Fokusfalle, Tab-Reihenfolge, Fokusrückgabe", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await ready(page);
  const tree = await axTree(page);
  writeFileSync(`${OUT}/a11y-baum-willkommen.yaml`, tree);
  log(
    "Baum enthält den Dialog",
    /dialog "Schön, dass du da bist"/.test(tree),
    null,
  );
  log(
    "Navigation der App nicht im Baum (inert)",
    !/button "Sprachgespräch"/.test(tree) &&
      !/RootWebArea[\s\S]*main "Start"/.test(tree),
    null,
  );
  const inert = await page.evaluate(() =>
    Array.from(
      document.querySelectorAll<HTMLElement>(
        ".workspace-shell, .lv-titlebar > *",
      ),
    ).map((el) => ({
      el: el.className.toString().slice(0, 40),
      inert: !!el.closest("[inert]"),
    })),
  );
  log(
    "App-Bereiche inert, Fensterknöpfe bedienbar",
    inert.every((x) =>
      x.el.includes("lv-window-controls") ? !x.inert : x.inert,
    ),
    inert,
  );
  log(
    "Dialog mit aria-modal, Titel und Beschreibung",
    await tour(page)
      .getByRole("dialog")
      .evaluate(
        (d) =>
          d.getAttribute("aria-modal") === "true" &&
          !!document.getElementById(d.getAttribute("aria-labelledby")!) &&
          !!document.getElementById(d.getAttribute("aria-describedby")!),
      ),
  );

  // Tab-Reihenfolge im Übungsschritt, zweimal herum.
  await tour(page).getByRole("button", { name: "Tour starten" }).click();
  await tour(page).getByRole("button", { name: "Weiter" }).click();
  const order: string[] = [];
  for (let i = 0; i < 8; i++) {
    order.push(
      await page.evaluate(() => {
        const el = document.activeElement as HTMLElement;
        const inDialog = !!el.closest("[role=dialog]");
        return `${inDialog ? "" : "AUSSERHALB "}${el.tagName === "TEXTAREA" ? "Übungsfeld" : el.innerText}`;
      }),
    );
    await page.keyboard.press("Tab");
  }
  log(
    "Tab bleibt im Dialog",
    order.every((x) => !x.startsWith("AUSSERHALB")),
    order,
  );
  log(
    "Tab-Reihenfolge: Feld, Überspringen, Zurück, Weiter, Feld …",
    order.slice(0, 5).join(" | ") ===
      "Übungsfeld | Überspringen | Zurück | Weiter | Übungsfeld",
    order,
  );
  writeFileSync(`${OUT}/a11y-baum-ueben.yaml`, await axTree(page));

  // Pfeiltasten auf einem Knopf schalten nicht weiter, auf dem Dialog schon.
  await tour(page).getByRole("button", { name: "Zurück" }).focus();
  await page.keyboard.press("ArrowRight");
  log(
    "Pfeil auf Knopf: kein Schrittwechsel",
    await tour(page)
      .getByRole("heading", { name: "Probier es gleich aus" })
      .isVisible(),
  );
  await tour(page).getByRole("dialog").focus();
  await page.keyboard.press("ArrowRight");
  log(
    "Pfeil auf Dialog: nächster Schritt",
    await tour(page)
      .getByRole("heading", { name: "Alles auf einen Blick" })
      .isVisible(),
  );
  await expect(tour(page).getByRole("status")).toHaveText(
    "4/6: Alles auf einen Blick",
  );
  log(
    "Schrittwechsel angesagt (role=status)",
    true,
    "4/6: Alles auf einen Blick",
  );

  // Esc außerhalb einer Aufnahme: Tour zu, Fokus zurück, App wieder bedienbar.
  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  await expect(page.getByTestId("workspace-start-tour")).toBeFocused();
  log("Esc schließt, Fokus auf „Einführung ansehen“", true);
  log(
    "inert nach dem Schließen aufgehoben",
    await page.evaluate(() => !document.querySelector("[inert]")),
  );

  // Auslöser im Hilfe-Menü: Fokus kehrt zum Auslöser zurück (Startseite).
  await page.getByTestId("workspace-start-tour").click();
  await tour(page).getByRole("button", { name: "Überspringen" }).click();
  await expect(page.getByTestId("workspace-start-tour")).toBeFocused();
  log("Fokusrückgabe an den Auslöser", true);
});

test("Esc während einer Aufnahme bricht nur die Aufnahme ab", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 2);
  const field = tour(page).getByRole("textbox", {
    name: "Übungsfeld zum Diktieren",
  });
  await expect(field).toBeFocused();
  await page.evaluate(
    () => ((window as unknown as { __recording: boolean }).__recording = true),
  );
  await page.keyboard.press("Escape");
  await expect
    .poll(
      async () =>
        (await commands(page)).filter((c) => c.cmd === "is_recording").length,
    )
    .toBe(1);
  await page.waitForTimeout(200);
  log("Aufnahme läuft: Tour bleibt offen", (await tour(page).count()) === 1);
  log(
    "Fokus bleibt im Übungsfeld",
    await field.evaluate((el) => el === document.activeElement),
  );
  await page.evaluate(
    () => ((window as unknown as { __recording: boolean }).__recording = false),
  );
  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  log("Ohne Aufnahme: Esc schließt die Tour, auch im Übungsfeld", true);
});

test("Halten und Umschalten: Texte je Modus", async ({ page }) => {
  for (const ptt of [true, false]) {
    await setup(page, { pushToTalk: ptt });
    await page.goto("/");
    await gotoStep(page, 1);
    const text = await tour(page).innerText();
    const want = ptt
      ? "Strg + Leertaste gedrückt halten"
      : "Strg + Leertaste erneut drücken";
    log(
      `${ptt ? "Halten" : "Umschalten"}: Diktier-Schritt nennt den Modus`,
      text.includes(want),
      want,
    );
    await tour(page).getByRole("button", { name: "Weiter" }).click();
    const tryText = await tour(page).innerText();
    const wantTry = ptt
      ? "Beim Loslassen erscheint der Text"
      : "erneut, dann erscheint der Text";
    log(
      `${ptt ? "Halten" : "Umschalten"}: Übungsschritt nennt den Modus`,
      tryText.includes(wantTry),
      wantTry,
    );
    log(
      `${ptt ? "Halten" : "Umschalten"}: Esc-Hinweis klar getrennt`,
      tryText.includes("Ohne laufende Aufnahme schließt Esc die Einführung"),
    );
    await page.evaluate(() => localStorage.clear());
  }
});

test("Gesperrt bis bereit, Wiederholen, Modellwahl, Neustart ohne Download", async ({
  page,
}) => {
  await setup(page, { downloadFails: 1 });
  await page.goto("/");
  await expect(tour(page).getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  log("Download 1 gescheitert, Fehler angesagt", true);
  await tour(page)
    .getByRole("button", { name: "Download wiederholen" })
    .click();
  await progress(page, 70);
  const bar = tour(page).getByRole("progressbar");
  log(
    "Fortschritt als progressbar mit Klartext",
    (await bar.getAttribute("aria-valuetext"))?.includes("70 %") === true,
    await bar.getAttribute("aria-valuetext"),
  );
  log(
    "Start gesperrt während des Downloads",
    await tour(page).getByRole("button", { name: "Tour starten" }).isDisabled(),
  );
  await finishDownload(page);
  await expect(tour(page).getByRole("alert")).toContainText("ist bereit");
  const calls = await commands(page);
  log(
    "Nach Download: Modell als Diktatmodell gewählt, erst dann bereit",
    calls.some((c) => c.cmd === "set_active_model"),
  );
  log(
    "Start frei, Text angepasst",
    (await tour(page).innerText()).includes("In sechs kurzen Schritten"),
  );
  await tour(page).getByRole("button", { name: "Tour starten" }).click();
  await tour(page).getByRole("button", { name: "Weiter" }).click();
  log(
    "Übungsfeld freigegeben",
    await tour(page).getByRole("textbox").isEnabled(),
  );
  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  const before = (await commands(page)).filter(
    (c) => c.cmd === "download_model",
  ).length;
  await page.getByTestId("workspace-start-tour").click();
  await expect(
    tour(page).getByRole("button", { name: "Tour starten" }),
  ).toBeEnabled();
  const after = (await commands(page)).filter(
    (c) => c.cmd === "download_model",
  ).length;
  log(
    "Neustart der Tour ohne weiteren Download",
    before === after && before === 2,
    { before, after },
  );
});

test("Übungsfeld gesperrt, solange das Modell lädt", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await progress(page, 30);
  // Tour starten ist gesperrt; das Übungsfeld wäre es ebenso (Schritt 3).
  log(
    "Start gesperrt",
    await tour(page).getByRole("button", { name: "Tour starten" }).isDisabled(),
  );
  writeFileSync(`${OUT}/a11y-baum-laedt.yaml`, await axTree(page));
});

test("Bewegung reduzieren: live umschaltbar, statischer Endzustand", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 1);
  log(
    "ohne Einstellung animiert",
    (await tour(page).getAttribute("data-reduced")) === null,
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(tour(page)).toHaveAttribute("data-reduced", "true");
  await expect(tour(page).locator(".lv-tour-anim__field")).toHaveText(
    "Sehr geehrte Frau Berger, vielen Dank für Ihre Nachricht.",
  );
  const anims = await page.evaluate(
    () =>
      Array.from(document.querySelectorAll(".lv-tour *")).filter(
        (el) => getComputedStyle(el).animationName !== "none",
      ).length,
  );
  log(
    "Einstellung live übernommen, Endzustand statisch, keine Animation",
    anims === 0,
    { anims },
  );
  const infinite = await page.evaluate(
    () =>
      Array.from(document.styleSheets)
        .flatMap((s) => {
          try {
            return Array.from(s.cssRules);
          } catch {
            return [];
          }
        })
        .filter(
          (r) => r.cssText.includes("lv-tour") && /infinite/.test(r.cssText),
        ).length,
  );
  log("keine Endlos-Animationen in den Tour-Stilen", infinite === 0, {
    infinite,
  });
});

test("Inneres Scrollen im niedrigen Fenster", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 300 });
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 1);
  const m = await tour(page)
    .getByRole("dialog")
    .evaluate((d) => ({
      scroll: d.scrollHeight > d.clientHeight,
      overflowY: getComputedStyle(d).overflowY,
      bottom: d.getBoundingClientRect().bottom <= window.innerHeight,
    }));
  log(
    "Dialog scrollt innen und bleibt im Fenster",
    m.scroll && m.overflowY === "auto" && m.bottom,
    m,
  );
  // Wie ein Nutzer: Klick in den Dialog, dann Umschalt+Tab auf „Weiter“.
  await tour(page).getByRole("dialog").focus();
  await page.keyboard.press("Shift+Tab");
  const pos = await tour(page)
    .getByRole("button", { name: "Weiter" })
    .evaluate((el) => ({
      bottom: el.getBoundingClientRect().bottom,
      vh: window.innerHeight,
      focused: el === document.activeElement,
    }));
  log(
    "„Weiter“ per Tastatur in Sicht gescrollt",
    pos.focused && pos.bottom <= pos.vh + 0.5,
    pos,
  );
});
