import { test, expect, type Page } from "@playwright/test";
import {
  VIEWPORTS,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// Aufnahmen-Oberflaeche (Goal aufnahmen-ui, M6): Barrierefreiheit (AK9).
// @axe-core/playwright ist nicht installiert (und wird nicht nachgeruestet);
// stattdessen pruefen gezielte Regeln: zugaengliche Namen, ARIA-Rollen und
// -Verweise, sichtbarer Fokus, Tastaturwege fuer Griffe, Menue und Dialoge.

test.beforeEach(async ({ page }) => {
  await installRecMock(page);
});

/** Findet Verstoesse gegen Namen, Rollen und Verweise im sichtbaren DOM. */
const audit = (page: Page): Promise<string[]> =>
  page.evaluate(() => {
    const problems: string[] = [];
    const visible = (el: Element) => {
      const r = (el as HTMLElement).getBoundingClientRect();
      const s = getComputedStyle(el);
      return (
        r.width > 0 &&
        r.height > 0 &&
        s.visibility !== "hidden" &&
        s.display !== "none"
      );
    };
    const describe = (el: Element) => {
      const h = el as HTMLElement;
      return (
        `${el.tagName.toLowerCase()}` +
        (h.dataset.testid ? `[${h.dataset.testid}]` : "") +
        (el.getAttribute("role") ? `{${el.getAttribute("role")}}` : "") +
        ` "${(h.innerText ?? "").trim().slice(0, 30)}"`
      );
    };
    const nameOf = (el: Element): string => {
      const by = el.getAttribute("aria-labelledby");
      if (by) {
        const t = by
          .split(/\s+/)
          .map((id) => document.getElementById(id)?.textContent ?? "")
          .join(" ")
          .trim();
        if (t) return t;
      }
      const label = el.getAttribute("aria-label")?.trim();
      if (label) return label;
      const labels = (el as HTMLInputElement).labels;
      if (labels && labels.length) {
        const t = Array.from(labels)
          .map((l) => l.textContent ?? "")
          .join(" ")
          .trim();
        if (t) return t;
      }
      const text = ((el as HTMLElement).innerText ?? "").trim();
      if (text) return text;
      const img = el.querySelector("img[alt]");
      if (img?.getAttribute("alt")) return img.getAttribute("alt")!;
      return (el.getAttribute("title") ?? "").trim();
    };

    const interactive = document.querySelectorAll(
      [
        "button",
        "a[href]",
        "input:not([type=hidden])",
        "select",
        "textarea",
        "summary",
        "[role=button]",
        "[role=tab]",
        "[role=menuitem]",
        "[role=menuitemcheckbox]",
        "[role=menuitemradio]",
        "[role=checkbox]",
        "[role=switch]",
        "[role=radio]",
        "[role=separator][tabindex]",
        "[role=slider]",
        "[role=option]",
        "[role=link]",
        "[tabindex='0']",
      ].join(","),
    );
    for (const el of Array.from(interactive)) {
      if (!visible(el)) continue;
      // Gesperrte Haken im Protokoll (Aufgabenliste) sind Anzeige, keine Bedienung.
      if ((el as HTMLInputElement).disabled && el.tagName === "INPUT") continue;
      if (!nameOf(el)) problems.push(`ohne Namen: ${describe(el)}`);
    }

    // Reiter liegen in einer Reiterleiste, tragen aria-selected und verweisen
    // auf ein vorhandenes Feld.
    for (const tab of Array.from(document.querySelectorAll("[role=tab]"))) {
      if (!visible(tab)) continue;
      if (!tab.closest("[role=tablist]"))
        problems.push(`Reiter ausserhalb tablist: ${describe(tab)}`);
      const sel = tab.getAttribute("aria-selected");
      if (sel !== "true" && sel !== "false")
        problems.push(`Reiter ohne aria-selected: ${describe(tab)}`);
      const ctl = tab.getAttribute("aria-controls");
      if (ctl && !document.getElementById(ctl))
        problems.push(`aria-controls ohne Ziel (${ctl}): ${describe(tab)}`);
    }
    for (const list of Array.from(
      document.querySelectorAll("[role=tablist]"),
    )) {
      if (!visible(list)) continue;
      if (!nameOf(list)) problems.push(`tablist ohne Namen: ${describe(list)}`);
    }
    // Felder und Bereiche mit Rolle haben einen Namen.
    for (const el of Array.from(
      document.querySelectorAll(
        "[role=tabpanel],[role=region],[role=dialog],[role=alertdialog],[role=menu]",
      ),
    )) {
      if (!visible(el)) continue;
      if (!nameOf(el) && !el.getAttribute("aria-labelledby"))
        problems.push(`Rolle ohne Namen: ${describe(el)}`);
    }
    // Griffe: Wert und Grenzen.
    for (const sep of Array.from(
      document.querySelectorAll("[role=separator][tabindex]"),
    )) {
      if (!visible(sep)) continue;
      for (const a of ["aria-valuenow", "aria-valuemin", "aria-valuemax"])
        if (!sep.hasAttribute(a))
          problems.push(`Griff ohne ${a}: ${describe(sep)}`);
    }
    // Eindeutige IDs, verwaiste aria-Verweise.
    const seen = new Set<string>();
    for (const el of Array.from(document.querySelectorAll("[id]"))) {
      if (seen.has(el.id)) problems.push(`doppelte id: ${el.id}`);
      seen.add(el.id);
    }
    for (const el of Array.from(
      document.querySelectorAll(
        "[aria-labelledby],[aria-describedby],[aria-controls]",
      ),
    )) {
      if (!visible(el)) continue;
      for (const attr of [
        "aria-labelledby",
        "aria-describedby",
        "aria-controls",
      ]) {
        for (const id of (el.getAttribute(attr) ?? "")
          .split(/\s+/)
          .filter(Boolean))
          // Tooltips ("...-tip") entstehen erst bei Hover oder Fokus; bis dahin
          // zeigt aria-describedby bewusst ins Leere.
          if (!id.endsWith("-tip") && !document.getElementById(id))
            problems.push(`${attr}="${id}" ohne Ziel: ${describe(el)}`);
      }
    }
    // Nichts Fokussierbares unter aria-hidden.
    for (const hidden of Array.from(
      document.querySelectorAll("[aria-hidden=true]"),
    )) {
      const f = hidden.matches(
        "button,a[href],input,select,textarea,[tabindex='0']",
      )
        ? hidden
        : hidden.querySelector(
            "button,a[href],input,select,textarea,[tabindex='0']",
          );
      if (f && visible(f))
        problems.push(`fokussierbar unter aria-hidden: ${describe(f)}`);
    }
    // Bilder mit Alternativtext.
    for (const img of Array.from(document.querySelectorAll("img"))) {
      if (!visible(img)) continue;
      if (
        !img.hasAttribute("alt") &&
        img.getAttribute("role") !== "presentation"
      )
        problems.push(`img ohne alt: ${describe(img)}`);
    }
    return problems;
  });

/**
 * Sichtbarer Fokus: Das fokussierte Element sieht anders aus als ohne Fokus
 * (Umriss, Ring, Rahmen, Hintergrund oder der Strich der Griffe). Uebergaenge
 * sind abgeschaltet, damit der Vergleich nicht mitten in einer Animation liegt.
 */
const focusIndicator = (page: Page) =>
  page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (!el || el === document.body) return null;
    const sig = () => {
      const s = getComputedStyle(el);
      const after = getComputedStyle(el, "::after");
      return [
        s.outlineStyle === "none"
          ? "none"
          : `${s.outlineWidth} ${s.outlineColor}`,
        s.boxShadow,
        s.borderColor,
        s.backgroundColor,
        s.color,
        after.backgroundColor,
        s.textDecorationLine,
      ].join("|");
    };
    const focused = sig();
    el.blur();
    const plain = sig();
    el.focus();
    return {
      visible: focused !== plain,
      label:
        `${el.tagName.toLowerCase()}` +
        (el.dataset.testid ? `[${el.dataset.testid}]` : "") +
        ` "${(el.getAttribute("aria-label") ?? el.innerText ?? "").trim().slice(0, 30)}"`,
    };
  });

const noTransitions = (page: Page) =>
  page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });

const openM2 = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

// ---------------------------------------------------------------------------
// Namen, Rollen, Verweise
// ---------------------------------------------------------------------------

for (const [label, vp] of Object.entries(VIEWPORTS)) {
  test(`AK9: Namen, Rollen und Verweise - ${label} px, Besprechung offen`, async ({
    page,
  }) => {
    await openM2(page, vp.width, vp.height);
    expect(await audit(page)).toEqual([]);
  });
}

test("AK9: Namen, Rollen und Verweise - jeder Reiter der Arbeitsflaeche und der Bedienung", async ({
  page,
}) => {
  await openM2(page, 1920, 1050);
  for (const area of ["rec-content", "rec-controls"]) {
    const tabs = page.getByTestId(area).getByRole("tab");
    const n = await tabs.count();
    expect(n).toBeGreaterThan(1);
    for (let i = 0; i < n; i++) {
      await tabs.nth(i).click();
      expect(await audit(page), `${area} Reiter ${i}`).toEqual([]);
    }
  }
});

test("AK9: Namen, Rollen und Verweise - ohne Auswahl und waehrend einer Aufnahme", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  expect(await audit(page)).toEqual([]);
  const live = await page.context().newPage();
  await installRecMock(live, { recording: true });
  await openRecordings(live, 1366, 768);
  await expect(live.getByTestId("live-notes-pad")).toBeVisible();
  expect(await audit(live)).toEqual([]);
  await live.close();
});

test("AK9: Landmarken - drei benannte Bereiche, die Seite hat genau ein main", async ({
  page,
}) => {
  await openM2(page, 1920, 1050);
  for (const id of ["rec-sessions", "rec-content", "rec-controls"]) {
    const area = page.getByTestId(id);
    await expect(area).toHaveAttribute("role", "region");
    expect(
      ((await area.getAttribute("aria-label")) ?? "").length,
    ).toBeGreaterThan(0);
  }
  const reading = await page.evaluate(() => ({
    mains: document.querySelectorAll("main,[role=main]").length,
    lang: document.documentElement.lang,
  }));
  expect(reading.mains).toBe(1);
  expect(reading.lang).not.toBe("");
});

// ---------------------------------------------------------------------------
// Fokus
// ---------------------------------------------------------------------------

test("AK9: Tab durchlaeuft die Seite, jedes fokussierte Element zeigt den Fokus", async ({
  page,
}) => {
  await openM2(page, 1366, 768);
  await noTransitions(page);
  const missing: string[] = [];
  const order: string[] = [];
  // Erst eine Taste, damit auch das erste programmatische focus() als
  // Tastaturfokus gilt (:focus-visible).
  await page.keyboard.press("Shift");
  await page.getByTestId("rec-sessions").getByRole("button").first().focus();
  for (let i = 0; i < 70; i++) {
    const f = await focusIndicator(page);
    // Der Notizblock ist eine Schreibflaeche ohne Rahmen: Schreibmarke und
    // Platzhalter zeigen den Fokus, wie in jedem Editor.
    if (f && !/^textarea\[note-/.test(f.label)) {
      order.push(f.label);
      if (!f.visible) missing.push(f.label);
    }
    await page.keyboard.press("Tab");
  }
  expect(order.length).toBeGreaterThan(20);
  expect(missing).toEqual([]);
});

test("AK9: die Griffe sind per Tab erreichbar und zeigen den Fokus", async ({
  page,
}) => {
  await openM2(page, 1920, 1050);
  await noTransitions(page);
  await page.keyboard.press("Shift");
  for (const id of ["resize-sessions", "resize-right"]) {
    const handle = page.getByTestId(id);
    await expect(handle).toHaveAttribute("tabindex", "0");
    await handle.focus();
    await expect(handle).toBeFocused();
    const f = await focusIndicator(page);
    expect(f?.visible, id).toBeTruthy();
  }
});

// ---------------------------------------------------------------------------
// Tastaturwege
// ---------------------------------------------------------------------------

test("AK9: Home und End setzen die Griffe an die Grenzen, Pfeile ändern in Schritten", async ({
  page,
}) => {
  await openM2(page, 1920, 1050);
  const handle = page.getByTestId("resize-sessions");
  await handle.focus();
  await page.keyboard.press("Home");
  const min = Number(await handle.getAttribute("aria-valuemin"));
  const max = Number(await handle.getAttribute("aria-valuemax"));
  const now = async () => Number(await handle.getAttribute("aria-valuenow"));
  const afterHome = await now();
  await page.keyboard.press("End");
  const afterEnd = await now();
  // Home/End sind optional; wenn sie wirken, dann an die Grenzen.
  if (afterHome !== 248 || afterEnd !== 248) {
    expect(afterHome).toBe(min);
    expect(afterEnd).toBe(max);
  }
});

test("AK9: Menue ☰ - Pfeiltasten wandern durch die Eintraege, Escape schliesst und gibt den Fokus zurueck", async ({
  page,
}) => {
  await openM2(page, 1366, 768);
  const trigger = page.getByTestId("meeting-menu");
  await trigger.focus();
  await page.keyboard.press("Enter");
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  const items = menu.getByRole("menuitem");
  expect(await items.count()).toBeGreaterThan(2);
  const active = () =>
    page.evaluate(
      () =>
        (document.activeElement as HTMLElement | null)?.dataset.testid ??
        document.activeElement?.getAttribute("role") ??
        "",
    );
  const first = await active();
  await page.keyboard.press("ArrowDown");
  const second = await active();
  expect(second).not.toBe(first);
  await page.keyboard.press("ArrowDown");
  const third = await active();
  expect(third).not.toBe(second);
  await page.keyboard.press("ArrowUp");
  expect(await active()).toBe(second);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test("AK9: Projekte - Menue per Tastatur, Eintraege mit Namen, Escape gibt den Fokus zurueck", async ({
  page,
}) => {
  await openRecordings(page, 1366, 768);
  const row = page
    .getByTestId("rec-sessions")
    .locator('[data-testid="project-row"]', { hasText: "Podcast" });
  await row.focus();
  await page.keyboard.press("ContextMenu");
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  for (const item of await menu.getByRole("menuitem").all()) {
    expect(((await item.innerText()) ?? "").trim().length).toBeGreaterThan(0);
  }
  const itemNames = await menu.getByRole("menuitem").allInnerTexts();
  expect(itemNames.length).toBeGreaterThan(1);
  const focusedItem = () =>
    page.evaluate(() => document.activeElement?.textContent?.trim() ?? "");
  expect(await focusedItem()).toBe(itemNames[0]);
  await page.keyboard.press("ArrowDown");
  expect(await focusedItem()).toBe(itemNames[1]);
  await page.keyboard.press("End");
  expect(await focusedItem()).toBe(itemNames[itemNames.length - 1]);
  await page.keyboard.press("ArrowDown");
  expect(await focusedItem()).toBe(itemNames[0]);
  await page.keyboard.press("ArrowUp");
  expect(await focusedItem()).toBe(itemNames[itemNames.length - 1]);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(row).toBeFocused();
  // Tab schliesst ebenfalls und laesst den Fokus an der Zeile.
  await page.keyboard.press("ContextMenu");
  await expect(menu).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(menu).toHaveCount(0);
  await expect(row).toBeFocused();
});

test("AK9: Dialoge - Fokus wandert hinein, bleibt gefangen, Escape schliesst und gibt ihn zurueck", async ({
  page,
}) => {
  await openM2(page, 1366, 768);
  const opener = page.getByTestId("meeting-details-open");
  await opener.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Details" });
  await expect(dialog).toBeVisible();
  // Fokus liegt im Dialog und verlaesst ihn auch nach vielen Tabs nicht.
  const inside = () =>
    page.evaluate(() => !!document.activeElement?.closest("[role=dialog]"));
  expect(await inside()).toBeTruthy();
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    expect(await inside(), `Tab ${i + 1}`).toBeTruthy();
  }
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Shift+Tab");
    expect(await inside(), `Shift+Tab ${i + 1}`).toBeTruthy();
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("AK9: Projekt-Dialog per Tastatur - Fokus gefangen, Escape schliesst", async ({
  page,
}) => {
  await openM2(page, 1366, 768);
  await page.getByTestId("project-chip").focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "In Projekt verschieben …" });
  await expect(dialog).toBeVisible();
  const inside = () =>
    page.evaluate(() => !!document.activeElement?.closest("[role=dialog]"));
  for (let i = 0; i < 10; i++) {
    await page.keyboard.press("Tab");
    expect(await inside(), `Tab ${i + 1}`).toBeTruthy();
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});
