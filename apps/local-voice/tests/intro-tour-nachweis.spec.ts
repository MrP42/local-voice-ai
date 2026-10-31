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

// Prüfnachweis der Einführungstour (Barrierefreiheit). Jede Prüfung landet
// als Zeile in test-results/intro-tour-nachweis/log.jsonl (mit Messwert),
// Accessibility-Bäume und Kontrasttabellen daneben. Grenzen der Simulation
// stehen in der Nachweisseite; der Windows-GUI-Test deckt Betriebssystem-
// Funktionen (echtes Einfügen, echter Aufnahmeabbruch) ab.
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
const STEPS = [
  "Willkommen",
  "Diktieren",
  "Üben",
  "Seiten",
  "Werkzeuge",
  "Fertig",
];

/** Farbrechnung im Browser (per `new Function` geladen): Alpha wird über
    alle Ebenen von der Wurzel abwärts verrechnet. */
const COLOR_LIB = String.raw`
const parse = (c) => {
  const m = c && c.match(/rgba?\(([^)]+)\)/);
  if (!m) return null;
  const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
  return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
};
const over = (top, bottom) => {
  const a = top.a + bottom.a * (1 - top.a);
  if (a === 0) return { r: 0, g: 0, b: 0, a: 0 };
  const mix = (t, b) => (t * top.a + b * bottom.a * (1 - top.a)) / a;
  return { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a };
};
const lum = (c) => {
  const f = (v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return Math.round(((x + 0.05) / (y + 0.05)) * 100) / 100;
};
const surface = (el) => {
  const chain = [];
  for (let n = el; n; n = n.parentElement) chain.unshift(n);
  let c = { r: 255, g: 255, b: 255, a: 1 };
  for (const n of chain) {
    const bg = parse(getComputedStyle(n).backgroundColor);
    if (bg && bg.a > 0) c = over(bg, c);
  }
  return c;
};
const hex = (c) => '#' + [c.r, c.g, c.b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
return { parse, over, ratio, surface, hex };
`;

type Measure = {
  name: string;
  kind: "text" | "ui";
  ratio: number;
  fg: string;
  bg: string;
};

/** Kontrastpunkte des aktuellen Schritts; erwartete, aber fehlende
    Messpunkte werden gemeldet (kein stilles Überspringen). */
const measureContrast = (page: Page, expected: string[]) =>
  page.evaluate(
    ({ lib, expected }) => {
      const L = new Function(lib)();
      type C = { r: number; g: number; b: number; a: number };
      const out: {
        name: string;
        kind: "text" | "ui";
        ratio: number;
        fg: string;
        bg: string;
      }[] = [];
      const box = document.querySelector("[role=dialog]");
      if (!box) return { out, missing: ["Dialog"] };
      const push = (
        name: string,
        kind: "text" | "ui",
        fgRaw: string,
        under: Element | C,
      ) => {
        const bg: C = under instanceof Element ? L.surface(under) : under;
        const fg0 = L.parse(fgRaw);
        if (!fg0) {
          out.push({
            name,
            kind,
            ratio: 0,
            fg: `unlesbar: ${fgRaw}`,
            bg: L.hex(bg),
          });
          return;
        }
        const fg = L.over(fg0, bg);
        out.push({
          name,
          kind,
          ratio: L.ratio(fg, bg),
          fg: L.hex(fg),
          bg: L.hex(bg),
        });
      };
      const texts: [string, string][] = [
        ["Schrittlabel", ".lv-tour__eyebrow"],
        ["Titel", ".lv-tour__title"],
        ["Text", ".lv-tour__body li, .lv-tour__body > p:not(.lv-tour__note)"],
        ["Hinweis", ".lv-tour__note"],
        ["Fehler", ".lv-tour__error"],
        [
          "Fortschritt",
          ".lv-tour__progress > span:not(.lv-tour__bar):not(.lv-tour__actions):not(.lv-tour__error)",
        ],
        ["Feldbeschriftung", ".lv-tour__label"],
        ["Knopf", "button:not([disabled])"],
      ];
      for (const [name, sel] of texts)
        box.querySelectorAll<HTMLElement>(sel).forEach((el, i) => {
          if (!el.offsetParent || !el.textContent?.trim()) return;
          push(
            `${name}#${i} „${el.textContent.trim().slice(0, 22)}“`,
            "text",
            getComputedStyle(el).color,
            el,
          );
        });
      const dot = box.querySelector(".lv-tour__dots i[data-on]");
      if (dot)
        push(
          "aktiver Schrittpunkt / Dialog",
          "ui",
          getComputedStyle(dot).backgroundColor,
          dot.parentElement!,
        );
      const fill = box.querySelector(".lv-tour__bar i");
      const rail = box.querySelector(".lv-tour__bar");
      if (fill && rail)
        push(
          "Balken / Schiene",
          "ui",
          getComputedStyle(fill).backgroundColor,
          rail,
        );
      const primary = box.querySelector<HTMLElement>(
        "[data-tour-primary]:not([disabled])",
      );
      if (primary) {
        const shadow =
          getComputedStyle(primary)
            .boxShadow.match(/rgba?\([^)]+\)/g)
            ?.at(-1) ?? "";
        push("Hauptknopf-Rand / Dialog", "ui", shadow, primary.parentElement!);
      }
      const field = box.querySelector<HTMLElement>(
        ".lv-tour__try:not([disabled])",
      );
      if (field)
        push(
          "Übungsfeld-Rahmen / Dialog",
          "ui",
          getComputedStyle(field).borderTopColor,
          field.parentElement!,
        );
      const spot = document.querySelector<HTMLElement>(".lv-tour__spot");
      if (spot) {
        const outline = getComputedStyle(spot).outlineColor;
        const r = spot.getBoundingClientRect();
        // Innen: hervorgehobene (nicht abgedunkelte) App-Fläche.
        const inner = document
          .elementsFromPoint(r.left + r.width / 2, r.top + 10)
          .find((el) => !el.closest(".lv-tour"));
        push(
          "Spotlight-Rahmen / hervorgehobene Fläche",
          "ui",
          outline,
          inner ?? document.body,
        );
        // Außen: App-Fläche neben dem Rahmen mit dem Schatten darüber verrechnet.
        const shade = L.parse(
          getComputedStyle(spot).boxShadow.match(/rgba?\([^)]+\)/)?.[0] ?? "",
        );
        const outer = document
          .elementsFromPoint(
            Math.min(window.innerWidth - 4, r.right + 24),
            r.top + r.height / 2,
          )
          .find((el) => !el.closest(".lv-tour"));
        const below = L.surface(outer ?? document.body);
        if (!shade)
          out.push({
            name: "Spotlight-Rahmen / abgedunkelte App",
            kind: "ui",
            ratio: 0,
            fg: "Schatten unlesbar",
            bg: L.hex(below),
          });
        else
          push(
            "Spotlight-Rahmen / abgedunkelte App",
            "ui",
            outline,
            L.over(shade, below),
          );
      }
      const names = out.map((m) => m.name);
      const missing = expected.filter(
        (e) => !names.some((n) => n.startsWith(e)),
      );
      return { out, missing };
    },
    { lib: COLOR_LIB, expected },
  );

const BASE = [
  "Schrittlabel",
  "Titel",
  "Text",
  "Knopf",
  "aktiver Schrittpunkt",
  "Hauptknopf-Rand",
];
const SPOT = [
  "Spotlight-Rahmen / hervorgehobene",
  "Spotlight-Rahmen / abgedunkelte",
];
const EXPECTED: Record<string, string[]> = {
  Willkommen: [...BASE, "Fortschritt"],
  Diktieren: [...BASE, "Hinweis"],
  Üben: [...BASE, "Hinweis", "Feldbeschriftung", "Übungsfeld-Rahmen"],
  Seiten: [...BASE, ...SPOT],
  Werkzeuge: [...BASE, ...SPOT],
  Fertig: BASE,
};

const checkContrast = async (
  page: Page,
  label: string,
  expected: string[],
  all: unknown[],
) => {
  await expect(tour(page).getByRole("dialog")).toBeVisible();
  // Erst den Schrittwechsel samt Fokus-Setzen abwarten, dann Ruhezustand
  // messen: Fokus aus dem Dialog nehmen (der Fokusring wird separat über die
  // Vorher/Nachher-Differenz geprüft).
  await page.waitForTimeout(300);
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
  await page.waitForTimeout(400); // Übergänge abwarten
  const { out, missing } = await measureContrast(page, expected);
  log(
    `${label}: alle ${expected.length} erwarteten Messpunktarten vorhanden`,
    missing.length === 0,
    { missing, messpunkte: out.length },
  );
  for (const m of out as Measure[]) {
    const min = m.kind === "text" ? 4.5 : 3;
    all.push({ step: label, ...m, min });
    log(`${label} ${m.name} ≥ ${min}:1`, m.ratio >= min, m);
  }
};

/** Fokus-relevante Eigenschaften des Elements (auch ::after). */
const FOCUS_STYLE = `(el) => {
  const cs = getComputedStyle(el);
  const af = getComputedStyle(el, "::after");
  return {
    outline: cs.outlineStyle + " " + cs.outlineWidth + " " + cs.outlineColor,
    border: cs.borderTopStyle + " " + cs.borderTopWidth + " " + cs.borderTopColor,
    boxShadow: cs.boxShadow,
    after: af.borderTopStyle + " " + af.borderTopWidth + " " + af.borderTopColor,
  };
}`;

/** Ruhezustand aller bedienbaren Elemente des Dialogs (nichts fokussiert). */
const restStyles = (page: Page) =>
  page.evaluate((fs) => {
    const style = new Function(`return ${fs}`)();
    (document.activeElement as HTMLElement | null)?.blur();
    const box = document.querySelector("[role=dialog]")!;
    return Array.from(
      box.querySelectorAll<HTMLElement>(
        "button:not([disabled]), textarea:not([disabled])",
      ),
    ).map((el) => style(el));
  }, FOCUS_STYLE);

/** Fokus-Indikator des aktiven Elements = Eigenschaften, die sich gegenüber
    dem Ruhezustand geändert haben; Kontrast gegen die angrenzende Fläche
    außen (Dialog) und innen (Element). */
const focusDelta = (page: Page, rest: Record<string, string>) =>
  page.evaluate(
    ({ lib, fs, rest }) => {
      const L = new Function(lib)();
      const style = new Function(`return ${fs}`)();
      const el = document.activeElement as HTMLElement;
      const now: Record<string, string> = style(el);
      const outside = L.surface(el.parentElement);
      const inside = L.surface(el);
      const changed = Object.keys(now).filter((k) => now[k] !== rest[k]);
      const parts = changed.flatMap((k) => {
        const v = now[k];
        if (/^none\b/.test(v) || /\b0px\b(?! )/.test(v.split(" ")[1] ?? ""))
          return [];
        return (v.match(/rgba?\([^)]+\)/g) ?? []).map((c) => {
          const col = L.parse(c);
          const o = L.over(col, outside);
          const i = L.over(col, inside);
          return {
            eigenschaft: k,
            farbe: L.hex(o),
            alpha: col.a,
            gegenAussen: L.ratio(o, outside),
            gegenInnen: L.ratio(i, inside),
          };
        });
      });
      const bester = parts
        .filter((p) => p.alpha >= 0.9)
        .reduce((m, p) => Math.max(m, p.gegenAussen, p.gegenInnen), 0);
      return {
        element: el.tagName === "TEXTAREA" ? "Übungsfeld" : el.innerText.trim(),
        geaendert: changed,
        teile: parts,
        bester,
      };
    },
    { lib: COLOR_LIB, fs: FOCUS_STYLE, rest },
  );

/** Echte Tab-Folge ab dem Dialog: jedes bedienbare Element genau einmal. */
const tabCycle = async (page: Page) => {
  await tour(page).getByRole("dialog").focus();
  const count = await page.evaluate(
    () =>
      document
        .querySelector("[role=dialog]")!
        .querySelectorAll("button:not([disabled]), textarea:not([disabled])")
        .length,
  );
  const seen: { index: number; name: string; visible: boolean }[] = [];
  for (let i = 0; i < count; i++) {
    await page.keyboard.press("Tab");
    seen.push(
      await page.evaluate(() => {
        const box = document.querySelector("[role=dialog]")!;
        const items = Array.from(
          box.querySelectorAll(
            "button:not([disabled]), textarea:not([disabled])",
          ),
        );
        const el = document.activeElement as HTMLElement;
        const r = el.getBoundingClientRect();
        return {
          index: items.indexOf(el),
          name: el.tagName === "TEXTAREA" ? "Übungsfeld" : el.innerText.trim(),
          visible:
            r.top >= 0 &&
            r.left >= 0 &&
            r.bottom <= window.innerHeight + 0.5 &&
            r.right <= window.innerWidth + 0.5,
        };
      }),
    );
  }
  return { count, seen };
};

/** Tab-Folge plus Fokus-Differenz je Element. */
const checkFocus = async (page: Page, label: string, sink: unknown[]) => {
  const rest = await restStyles(page);
  const { count, seen } = await tabCycle(page);
  log(
    `${label}: Tab erreicht jedes der ${count} Elemente genau einmal`,
    count > 0 &&
      new Set(seen.map((x) => x.index)).size === count &&
      seen.every((x) => x.index >= 0),
    seen,
  );
  await tour(page).getByRole("dialog").focus();
  for (let i = 0; i < count; i++) {
    await page.keyboard.press("Tab");
    const d = await focusDelta(page, rest[seen[i].index]);
    sink.push({ step: label, ...d });
    log(
      `${label}: Fokus „${d.element}“ verändert die Darstellung, ≥ 3:1`,
      d.geaendert.length > 0 && d.bester >= 3,
      d,
    );
  }
};

const next = (page: Page, s: number) =>
  tour(page)
    .getByRole("button", { name: s === 0 ? "Tour starten" : "Weiter" })
    .click();

for (const theme of ["light", "dark"] as const) {
  test(`Kontrast und Fokus in allen Schritten (${theme})`, async ({ page }) => {
    await setup(page, { theme });
    await page.goto("/");
    await expect(tour(page).getByRole("dialog")).toBeVisible();
    await progress(page, 40);
    const all: unknown[] = [];
    const focus: unknown[] = [];
    await checkContrast(
      page,
      `${theme} Willkommen (lädt)`,
      [
        "Schrittlabel",
        "Titel",
        "Text",
        "Fortschritt",
        "Balken / Schiene",
        "Knopf",
        "aktiver Schrittpunkt",
      ],
      all,
    );
    await checkFocus(page, `${theme} Willkommen (lädt)`, focus);
    await ready(page);
    for (let s = 0; s < STEPS.length; s++) {
      const label = `${theme} ${STEPS[s]}`;
      await checkContrast(page, label, EXPECTED[STEPS[s]], all);
      await checkFocus(page, label, focus);
      if (s < STEPS.length - 1) await next(page, s);
    }
    writeFileSync(
      `${OUT}/kontrast-${theme}.json`,
      JSON.stringify(all, null, 2),
    );
    writeFileSync(`${OUT}/fokus-${theme}.json`, JSON.stringify(focus, null, 2));
  });

  test(`Fehlerzustand: Kontrast, Fokus, Knopftexte (${theme})`, async ({
    page,
  }) => {
    await setup(page, { theme, downloadFails: 9 });
    await page.goto("/");
    await expect(tour(page).getByRole("alert")).toContainText(
      "konnte nicht geladen werden",
    );
    const all: unknown[] = [];
    const focus: unknown[] = [];
    log(
      "Fokus beim Fehler auf „Download wiederholen“",
      await tour(page)
        .getByRole("button", { name: "Download wiederholen" })
        .evaluate((el) => el === document.activeElement),
    );
    await checkContrast(
      page,
      `${theme} Fehler`,
      [
        "Schrittlabel",
        "Titel",
        "Text",
        "Fehler",
        "Knopf",
        "aktiver Schrittpunkt",
      ],
      all,
    );
    const buttons = await tour(page).getByRole("button").allInnerTexts();
    log(
      "Fehlerknöpfe: „Download wiederholen“ und „Anderes Modell auswählen“",
      buttons.includes("Download wiederholen") &&
        buttons.includes("Anderes Modell auswählen"),
      buttons,
    );
    log(
      "Start gesperrt bei Fehler",
      await tour(page)
        .getByRole("button", { name: "Tour starten" })
        .isDisabled(),
    );
    await checkFocus(page, `${theme} Fehler`, focus);
    writeFileSync(
      `${OUT}/kontrast-fehler-${theme}.json`,
      JSON.stringify(all, null, 2),
    );
    writeFileSync(
      `${OUT}/fokus-fehler-${theme}.json`,
      JSON.stringify(focus, null, 2),
    );
  });
}

// Zoom-Simulation: CSS-Ansichtsgröße wie bei Browserzoom (1280×800 bei
// 200 % = 640×400 CSS px; 1280×1024 bei 400 % = 320×256 CSS px), Geräte-
// pixelverhältnis 2. Das ist KEIN echter Browserzoom (Strg+Plus).
for (const z of [
  { name: "200 % simuliert (640×400 CSS px)", w: 640, h: 400 },
  { name: "Reflow 320 CSS px (400 % simuliert)", w: 320, h: 256 },
]) {
  test.describe(z.name, () => {
    test.use({ viewport: { width: z.w, height: z.h }, deviceScaleFactor: 2 });
    test("alle Schritte: kein waagerechtes Scrollen, Tab-Folge erreicht alles sichtbar", async ({
      page,
    }) => {
      await setup(page);
      await page.goto("/");
      await ready(page);
      for (let s = 0; s < STEPS.length; s++) {
        const m = await page.evaluate(() => {
          const box = document.querySelector<HTMLElement>("[role=dialog]")!;
          const r = box.getBoundingClientRect();
          return {
            seiteWaagerecht:
              document.documentElement.scrollWidth > window.innerWidth,
            dialogWaagerecht: box.scrollWidth > box.clientWidth + 1,
            imFenster:
              r.left >= 0 &&
              r.top >= 0 &&
              r.right <= window.innerWidth + 0.5 &&
              r.bottom <= window.innerHeight + 0.5,
            innenScroll: box.scrollHeight > box.clientHeight,
            overflowY: getComputedStyle(box).overflowY,
          };
        });
        log(
          `${z.name} ${STEPS[s]}: kein waagerechtes Scrollen`,
          !m.seiteWaagerecht && !m.dialogWaagerecht,
          m,
        );
        log(`${z.name} ${STEPS[s]}: Dialog im Fenster`, m.imFenster, m);
        if (m.innenScroll)
          log(
            `${z.name} ${STEPS[s]}: inneres Scrollen aktiv`,
            m.overflowY === "auto",
            m,
          );
        const { count, seen } = await tabCycle(page);
        log(
          `${z.name} ${STEPS[s]}: Tab-Folge erreicht alle ${count} Elemente`,
          new Set(seen.map((x) => x.index)).size === count,
          seen,
        );
        log(
          `${z.name} ${STEPS[s]}: jedes per Tab fokussierte Element sichtbar`,
          seen.every((x) => x.visible),
          seen,
        );
        if (s < STEPS.length - 1) await next(page, s);
      }
    });
  });
}

/** Chromium-Accessibility-Baum (CDP). Ausgelassen: ignorierte Knoten sowie
    generic/none/StaticText/InlineTextBox (Text steckt im Namen der Eltern). */
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
  const top: string[] = [];
  const walk = (id: string, depth: number) => {
    const n = byId.get(id);
    if (!n) return;
    const show = !n.ignored && !!n.role && !skip.includes(n.role.value);
    if (show) {
      const line = `${n.role!.value}${n.name?.value ? ` "${n.name.value}"` : ""}`;
      lines.push(`${"  ".repeat(depth)}- ${line}`);
      if (depth === 1) top.push(line);
    }
    for (const c of n.childIds ?? []) walk(c, show ? depth + 1 : depth);
  };
  const root = nodes.find((n) => !n.parentId);
  if (root) walk(root.nodeId, 0);
  return { text: lines.join("\n") + "\n", top };
};

/** Erlaubt auf oberster Ebene: Dialog, Live-Regionen, Hinweis-Region. */
const allowedTop = (line: string) =>
  /^(dialog|status|alert)\b/.test(line) || /^region "Notifications/.test(line);

test("Modal: Accessibility-Baum, Tab-Reihenfolge, Pfeile, Ansagen, Fokusrückgabe", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await ready(page);
  const ax = await axTree(page);
  writeFileSync(`${OUT}/a11y-baum-willkommen.yaml`, ax.text);
  log(
    "Oberste Baumebene: nur Dialog, Live-Regionen, Hinweis-Region (Positivliste)",
    ax.top.every(allowedTop) && ax.top.some((l) => l.startsWith("dialog")),
    ax.top,
  );
  log(
    "Dialog: aria-modal, Titel und Beschreibung verknüpft",
    await tour(page)
      .getByRole("dialog")
      .evaluate(
        (d) =>
          d.getAttribute("aria-modal") === "true" &&
          !!document.getElementById(d.getAttribute("aria-labelledby")!) &&
          !!document.getElementById(d.getAttribute("aria-describedby")!),
      ),
  );
  log(
    "Live-Regionen: status=polite, alert=assertive",
    await tour(page).evaluate(
      (t) =>
        t.querySelector("[role=status]")?.getAttribute("aria-live") ===
          "polite" &&
        t.querySelector("[role=alert]")?.getAttribute("aria-live") ===
          "assertive",
    ),
  );

  await tour(page).getByRole("button", { name: "Tour starten" }).click();
  await tour(page).getByRole("button", { name: "Weiter" }).click();
  const name = () =>
    page.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      return `${el.closest("[role=dialog]") ? "" : "AUSSERHALB "}${el.tagName === "TEXTAREA" ? "Übungsfeld" : el.innerText}`;
    });
  const order: string[] = [];
  await tour(page).getByRole("dialog").focus();
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press("Tab");
    order.push(await name());
  }
  log(
    "Tab bleibt im Dialog und läuft im Kreis",
    order.join(" | ") ===
      "Übungsfeld | Überspringen | Zurück | Weiter | Übungsfeld | Überspringen | Zurück | Weiter",
    order,
  );
  const back: string[] = [];
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("Shift+Tab");
    back.push(await name());
  }
  log(
    "Umschalt+Tab läuft rückwärts im Kreis",
    back.join(" | ") === "Zurück | Überspringen | Übungsfeld | Weiter",
    back,
  );
  writeFileSync(`${OUT}/a11y-baum-ueben.yaml`, (await axTree(page)).text);

  await tour(page).getByRole("button", { name: "Zurück" }).focus();
  await page.keyboard.press("ArrowRight");
  log(
    "Pfeil auf Knopf: kein Schrittwechsel",
    await tour(page)
      .getByRole("heading", { name: "Probier es gleich aus" })
      .isVisible(),
  );
  await tour(page).getByRole("textbox").focus();
  await page.keyboard.press("ArrowRight");
  log(
    "Pfeil im Übungsfeld: kein Schrittwechsel",
    await tour(page)
      .getByRole("heading", { name: "Probier es gleich aus" })
      .isVisible(),
  );
  await tour(page).getByRole("dialog").focus();
  await page.keyboard.press("ArrowRight");
  await expect(
    tour(page).getByRole("heading", { name: "Alles auf einen Blick" }),
  ).toBeVisible();
  log("Pfeil auf dem Dialog: nächster Schritt", true);
  await expect(tour(page).getByRole("status")).toHaveText(
    "4/6: Alles auf einen Blick",
  );
  log(
    "Schrittwechsel steht in role=status",
    true,
    "4/6: Alles auf einen Blick",
  );

  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  await expect(page.getByTestId("workspace-start-tour")).toBeFocused();
  log("Esc schließt, Fokus auf „Einführung ansehen“", true);
  log(
    "inert nach dem Schließen aufgehoben",
    await page.evaluate(() => !document.querySelector("[inert]")),
  );
  const after = await axTree(page);
  log(
    "App nach dem Schließen wieder im Accessibility-Baum",
    after.top.some((l) => l.startsWith("navigation")) &&
      after.top.some((l) => l.startsWith("main")),
    after.top,
  );

  await page.getByTestId("workspace-start-tour").click();
  await tour(page).getByRole("button", { name: "Überspringen" }).click();
  await expect(page.getByTestId("workspace-start-tour")).toBeFocused();
  log("Überspringen: Fokus zurück an den Auslöser", true);
});

test("Fensterknöpfe bleiben während der Tour bedienbar (Desktop)", async ({
  page,
}) => {
  await setup(page, { desktop: true });
  await page.goto("/");
  await ready(page);
  const controls = page.locator(".lv-window-controls");
  await expect(controls).toBeVisible();
  log(
    "Fensterknöpfe nicht inert",
    await controls.evaluate((el) => !el.closest("[inert]")),
  );
  log(
    "Rest der Titelleiste inert",
    await page
      .locator(".lv-titlebar__app")
      .evaluate((el) => !!el.closest("[inert]")),
  );
  // Echter Mausklick: Playwright klickt nur, wenn nichts darüber liegt.
  await controls.getByRole("button").first().click({ timeout: 5000 });
  await expect
    .poll(async () =>
      (await commands(page)).some((c) => c.cmd === "plugin:window|minimize"),
    )
    .toBe(true);
  log(
    "Klick auf „Minimieren“ erreicht das Fenster (plugin:window|minimize)",
    true,
  );
  log("Tour bleibt offen", (await tour(page).count()) === 1);
});

test("Esc im Übungsfeld: Tour bleibt während einer Aufnahme offen (Abbruch selbst: Windows-GUI-Test)", async ({
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
  log(
    "Aufnahmezustand abgefragt, Tour bleibt offen",
    (await tour(page).count()) === 1,
  );
  log(
    "Fokus bleibt im Übungsfeld",
    await field.evaluate((el) => el === document.activeElement),
  );
  await page.evaluate(
    () => ((window as unknown as { __recording: boolean }).__recording = false),
  );
  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  log("Ohne Aufnahme: Esc aus dem Übungsfeld schließt die Tour", true);
  await expect(page.getByTestId("workspace-start-tour")).toBeFocused();
  log("Fokusrückgabe nach Esc aus dem Übungsfeld", true);
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
      `${ptt ? "Halten" : "Umschalten"}: Esc-Hinweis trennt Aufnahme und Tour`,
      tryText.includes("Ohne laufende Aufnahme schließt Esc die Einführung"),
    );
    await page.evaluate(() => localStorage.clear());
  }
});

test("Download scheitert, Wiederholen, Sperre bis Modellwahl abgeschlossen, Neustart ohne Download", async ({
  page,
}) => {
  await setup(page, { downloadFails: 1, holdSelect: true });
  await page.goto("/");
  await expect(tour(page).getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  log("Download gescheitert, Fehler in role=alert", true);
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
  const start = tour(page).getByRole("button", { name: "Tour starten" });
  log("Start gesperrt während des Downloads", await start.isDisabled());
  await finishDownload(page);
  await expect
    .poll(async () =>
      (await commands(page)).some((c) => c.cmd === "set_active_model"),
    )
    .toBe(true);
  await page.waitForTimeout(300);
  log(
    "Download fertig, Modellwahl läuft: Start weiter gesperrt",
    await start.isDisabled(),
  );
  log(
    "… und noch keine Bereit-Ansage",
    !(await tour(page).getByRole("alert").innerText()).includes("ist bereit"),
  );
  await page.evaluate(() =>
    (window as unknown as { __releaseSelect: () => void }).__releaseSelect(),
  );
  await expect(tour(page).getByRole("alert")).toContainText("ist bereit");
  log("Modellwahl abgeschlossen: Bereit-Ansage", true);
  log("Start erst jetzt frei", await start.isEnabled());
  log(
    "Willkommenstext passt zum Bereit-Zustand",
    (await tour(page).innerText()).includes("In sechs kurzen Schritten"),
  );
  await start.click();
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

test("Modellwahl scheitert, Wiederholen wählt erneut", async ({ page }) => {
  await setup(page, { selectFails: 1 });
  await page.goto("/");
  await expect(tour(page).getByRole("dialog")).toBeVisible();
  await finishDownload(page);
  await expect(tour(page).getByRole("alert")).toContainText(
    "konnte nicht geladen werden",
  );
  log("Gescheiterte Modellwahl: Fehler angesagt", true);
  log(
    "Start gesperrt",
    await tour(page).getByRole("button", { name: "Tour starten" }).isDisabled(),
  );
  const selectsBefore = (await commands(page)).filter(
    (c) => c.cmd === "set_active_model",
  ).length;
  await tour(page)
    .getByRole("button", { name: "Download wiederholen" })
    .click();
  await expect(tour(page).getByRole("alert")).toContainText("ist bereit");
  const selects = (await commands(page)).filter(
    (c) => c.cmd === "set_active_model",
  ).length;
  log(
    "Wiederholen löst eine neue Modellwahl aus (die Fußleiste wählt beim Download-Ende zusätzlich selbst)",
    selects > selectsBefore,
    { selectsBefore, selects },
  );
  log(
    "Start danach frei",
    await tour(page).getByRole("button", { name: "Tour starten" }).isEnabled(),
  );
});

test("Anderes Modell auswählen öffnet die Modellauswahl", async ({ page }) => {
  await setup(page, { downloadFails: 9 });
  await page.goto("/");
  await tour(page)
    .getByRole("button", { name: "Anderes Modell auswählen" })
    .click();
  await expect(tour(page)).toHaveCount(0);
  await expect(page.getByText("Nemotron 3.5 ASR Streaming")).toBeVisible();
  log("Tour geschlossen, Modellauswahl sichtbar", true);
});

test("Lädt noch: Start gesperrt, Übungsfeld nicht erreichbar", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await expect(tour(page).getByRole("dialog")).toBeVisible();
  await progress(page, 30);
  log(
    "Start gesperrt",
    await tour(page).getByRole("button", { name: "Tour starten" }).isDisabled(),
  );
  await tour(page).getByRole("dialog").focus();
  await page.keyboard.press("ArrowRight");
  log(
    "Pfeil schaltet nicht weiter, solange das Modell lädt",
    await tour(page)
      .getByRole("heading", { name: "Schön, dass du da bist" })
      .isVisible(),
  );
  log(
    "Kein Übungsfeld erreichbar",
    (await tour(page).getByRole("textbox").count()) === 0,
  );
  writeFileSync(`${OUT}/a11y-baum-laedt.yaml`, (await axTree(page)).text);
});

const runningAnimations = (page: Page) =>
  page.evaluate(
    () =>
      document.getAnimations().filter((a) => {
        const target = (a.effect as KeyframeEffect | null)?.target;
        return (
          target instanceof Element &&
          !!target.closest(".lv-tour") &&
          a.playState === "running"
        );
      }).length,
  );

test("Bewegung reduzieren: live umschaltbar, Animationen gestoppt, Endzustand statisch", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 1);
  await page.waitForTimeout(700);
  const before = await runningAnimations(page);
  log(
    "ohne Einstellung laufen Animationen (document.getAnimations)",
    before > 0,
    { laufend: before },
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(tour(page)).toHaveAttribute("data-reduced", "true");
  await expect(tour(page).locator(".lv-tour-anim__field")).toHaveText(
    "Sehr geehrte Frau Berger, vielen Dank für Ihre Nachricht.",
  );
  log("Diktier-Animation zeigt sofort den Endzustand", true);
  await page.waitForTimeout(300);
  const afterCount = await runningAnimations(page);
  log("mit Einstellung keine laufende Animation", afterCount === 0, {
    laufend: afterCount,
  });
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

test("Inneres Scrollen im niedrigen Fenster per Tastatur", async ({ page }) => {
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
  await tour(page).getByRole("dialog").focus();
  await page.keyboard.press("Shift+Tab");
  const pos = await page.evaluate(() => {
    const el = document.activeElement as HTMLElement;
    return {
      element: el.innerText,
      bottom: el.getBoundingClientRect().bottom,
      vh: window.innerHeight,
    };
  });
  log(
    "Umschalt+Tab erreicht „Weiter“ und scrollt es in Sicht",
    pos.element === "Weiter" && pos.bottom <= pos.vh + 0.5,
    pos,
  );
});

test("Mobil 390 px: Knopfreihenfolge wie am Desktop, eine Zeile", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 2);
  const row = await tour(page)
    .locator(".lv-tour__nav button")
    .evaluateAll((els) =>
      els.map((el) => ({
        text: (el as HTMLElement).innerText,
        top: Math.round(el.getBoundingClientRect().top),
        left: Math.round(el.getBoundingClientRect().left),
      })),
    );
  log(
    "Reihenfolge Überspringen, Zurück, Weiter",
    row.map((r) => r.text).join(",") === "Überspringen,Zurück,Weiter",
    row,
  );
  log(
    "alle in einer Zeile, von links nach rechts",
    new Set(row.map((r) => r.top)).size === 1 &&
      row.every((r, i) => i === 0 || r.left > row[i - 1].left),
    row,
  );
});

test("Abschluss: nur „Zurück“ und „Loslegen“", async ({ page }) => {
  await setup(page);
  await page.goto("/");
  await gotoStep(page, 5);
  const buttons = await tour(page).getByRole("button").allInnerTexts();
  log(
    "letzter Schritt zeigt genau Zurück und Loslegen",
    buttons.join(",") === "Zurück,Loslegen",
    buttons,
  );
});
