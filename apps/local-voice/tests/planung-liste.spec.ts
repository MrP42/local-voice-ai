import { test, expect, type Page } from "@playwright/test";
import type { Plan, PlanLink } from "../src/bindings";
import {
  listenMarke,
  listenStand,
  trifft,
  wochenAnfang,
  zeitGruppe,
} from "../src/components/planung/liste";
import { installMock, link, plan, T0 } from "./planungMock";

// lva#844 U2: Liste der Planungen mit Suche, Zeitgruppen, Zustandschip, Zählern, Schublade unter
// 1000 px sowie Leer- und Ladezustand. Nur synthetische Daten (example.org). Die App-Sprache setzt
// die Attrappe auf Deutsch; die Uhr steht auf T0 (Montag, 26.10.2026, 08:33 in Berlin).
test.use({ locale: "en-US", timezoneId: "Europe/Berlin" });

const TAG = 86_400_000;

const zeile = (page: Page, titel: string) =>
  page.getByTestId("planung-zeile").filter({ hasText: titel });
const suche = (page: Page) =>
  page.getByRole("searchbox", { name: "Planungen durchsuchen" });

/** Zusätzliche Planungen der Attrappe (nach `installMock`, vor dem Laden der Seite). */
async function zusaetzlich(
  page: Page,
  plaene: unknown[],
  links: Record<string, unknown>,
  nurAktiv = false,
) {
  await page.addInitScript(
    ({ plaene, links, nurAktiv }) => {
      if (window.top !== window) return;
      const d = (window as unknown as { __daten: Record<string, any> }).__daten;
      if (nurAktiv)
        d.plaene = d.plaene.filter(
          (p: { status: string }) =>
            ![
              "abgesagt",
              "durchgefuehrt",
              "protokolliert",
              "versendet",
            ].includes(p.status),
        );
      d.plaene.push(...plaene);
      Object.assign(d.links, links);
    },
    { plaene, links, nurAktiv },
  );
}

/** Weitere Planungen: Umlaute im Titel und in den Rollen, nächste Woche, später, früher. */
async function mitWeiteren(page: Page, nurAktiv = false) {
  await zusaetzlich(
    page,
    [
      plan("p_baer", "Rückblick Bärenbrunnen", { status: "verknuepft" }),
      plan("p_strasse", "Begehung Hauptstraße 1", {
        status: "verknuepft",
        roles: {
          moderation: "Lea Schäfer",
          ergebnissicherung: null,
          entscheidung: null,
        },
      }),
      plan("p_spaet", "Jahresplanung Gremien", { status: "verknuepft" }),
      plan("p_frueh", "Nachlese Messe", { status: "verknuepft" }),
    ],
    {
      p_baer: link("p_baer", {
        starts_at: T0 + 8 * TAG,
        ends_at: T0 + 8 * TAG + 1_500_000,
        join_url: "https://meet.example.org/j/1",
      }),
      p_strasse: link("p_strasse", {
        starts_at: T0 + 9 * TAG,
        ends_at: T0 + 9 * TAG + 1_500_000,
      }),
      p_spaet: link("p_spaet", {
        starts_at: T0 + 30 * TAG,
        ends_at: T0 + 30 * TAG + 1_500_000,
      }),
      p_frueh: link("p_frueh", {
        starts_at: T0 - 3 * TAG,
        ends_at: T0 - 3 * TAG + 1_500_000,
      }),
    },
    nurAktiv,
  );
}

async function oeffne(page: Page, breite = 1440, hoehe = 900) {
  await page.clock.setFixedTime(new Date(T0));
  await page.setViewportSize({ width: breite, height: hoehe });
  await page.goto("/");
  await expect(page.getByTestId("planung-seite")).toBeVisible();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;transition-duration:0s!important}",
  });
}

async function keinSeitwaertsRollen(page: Page) {
  const breit = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    fenster: window.innerWidth,
    inhalt: [...document.querySelectorAll<HTMLElement>(".plg-spalte")].map(
      (el) => el.scrollWidth - el.clientWidth,
    ),
  }));
  expect(breit.scroll).toBeLessThanOrEqual(breit.fenster);
  for (const ueber of breit.inhalt) expect(ueber).toBeLessThanOrEqual(0);
}

// ---------------------------------------------------------------- reine Logik

test("Logik: Zeitgruppen nach der Kalenderwoche (Montag bis Sonntag)", () => {
  const montag = wochenAnfang(T0);
  expect(new Date(montag).getDay()).toBe(1);
  expect(new Date(montag).getHours()).toBe(0);
  expect(zeitGruppe(null, T0)).toBe("ohneTermin");
  expect(zeitGruppe(montag - 1, T0)).toBe("frueher");
  expect(zeitGruppe(montag, T0)).toBe("dieseWoche");
  expect(zeitGruppe(T0 + 6 * TAG, T0)).toBe("dieseWoche");
  expect(zeitGruppe(montag + 7 * TAG, T0)).toBe("naechsteWoche");
  expect(zeitGruppe(T0 + 13 * TAG, T0)).toBe("naechsteWoche");
  expect(zeitGruppe(T0 + 15 * TAG, T0)).toBe("spaeter");
});

test("Logik: ein Chip je Zeile, was Handlung verlangt vor dem Status", () => {
  const l = (extra: Record<string, unknown>) =>
    link("p", extra) as unknown as PlanLink;
  expect(listenMarke({ status: "verknuepft" }, null).schluessel).toBe(
    "verknuepft",
  );
  expect(
    listenMarke({ status: "verknuepft" }, l({ block_konflikt: true }))
      .schluessel,
  ).toBe("konflikt");
  expect(
    listenMarke({ status: "verknuepft" }, l({ verschiebung_offen: true }))
      .schluessel,
  ).toBe("verschoben");
  // Konflikt (Gefahr) geht der Verschiebung (Warnung) vor.
  expect(
    listenMarke(
      { status: "verschoben" },
      l({ verschiebung_offen: true, block_konflikt: true }),
    ).schluessel,
  ).toBe("konflikt");
  // Ein fremder Termin (Info) verdrängt den Status nicht.
  expect(
    listenMarke({ status: "uebergeben" }, l({ fremd_beobachtet: true }))
      .schluessel,
  ).toBe("uebergeben");
  // Nach der Durchführung zählt nur der Status.
  expect(
    listenMarke({ status: "durchgefuehrt" }, l({ block_konflikt: true }))
      .schluessel,
  ).toBe("durchgefuehrt");
});

test("Logik: Suchvertrag aus #839 (Umlaute, Treffer nur an Zeichengrenzen)", () => {
  const p = (titel: string, extra: Record<string, unknown> = {}) =>
    plan("p", titel, {
      goal_text: "",
      expected_result: null,
      classification: "",
      agenda: [],
      roles: { moderation: null, ergebnissicherung: null, entscheidung: null },
      ...extra,
    }) as unknown as Plan;
  const schaefer = p("Jour fixe", {
    roles: {
      moderation: "Lea Schäfer",
      ergebnissicherung: null,
      entscheidung: null,
    },
  });
  expect(trifft(schaefer, null, "schae")).toBe(true);
  expect(trifft(schaefer, null, "schä")).toBe(true);
  expect(trifft(p("Hauptstraße 1"), null, "strasse")).toBe(true);
  const baer = p("Bär");
  expect(trifft(baer, null, "ae")).toBe(true);
  expect(trifft(baer, null, "er")).toBe(false);
  expect(trifft(baer, null, "e")).toBe(false);
  expect(trifft(baer, null, "ba")).toBe(false);
  expect(trifft(baer, null, "bä")).toBe(true);
  expect(trifft(baer, null, "bae")).toBe(true);
  // Agenda, Ort und Unterlagen gehören dazu; Felder werden nicht verkettet.
  const agenda = p("Jour fixe", {
    agenda: [{ minutes: 5, topic: "Haushalt", owner: "Jörg Müller" }],
    materials: [{ label: "Entwurf Satzung", url: null, due_date: null }],
  });
  expect(trifft(agenda, null, "mueller")).toBe(true);
  expect(trifft(agenda, null, "satzung")).toBe(true);
  expect(trifft(agenda, null, "fixe haushalt")).toBe(false);
  expect(trifft(p("Jour fixe"), l("p", { location: "Raum Süd" }), "sued")).toBe(
    true,
  );
  function l(id: string, extra: Record<string, unknown>) {
    return link(id, extra) as unknown as PlanLink;
  }
});

test("Logik: Zähler je Filter folgen der Suche, Gruppen bleiben zusammenhängend", () => {
  const a = plan("a", "Alpha Bär", { status: "verknuepft" }) as unknown as Plan;
  const b = plan("b", "Beta", { status: "entwurf" }) as unknown as Plan;
  const c = plan("c", "Gamma Bär", {
    status: "durchgefuehrt",
  }) as unknown as Plan;
  const links = new Map<string, PlanLink | null>([
    ["a", link("a", { starts_at: T0 + TAG }) as unknown as PlanLink],
    ["b", null],
    ["c", link("c", { starts_at: T0 - 10 * TAG }) as unknown as PlanLink],
  ]);
  const ohne = listenStand([a, b, c], links, "aktiv", "", T0);
  expect(ohne.zaehler).toEqual({ aktiv: 2, archiv: 1, alle: 3 });
  expect(ohne.aktivGesamt).toBe(2);
  expect(ohne.gruppen.map((g) => g.gruppe)).toEqual([
    "dieseWoche",
    "ohneTermin",
  ]);
  const baer = listenStand([a, b, c], links, "alle", "bär", T0);
  expect(baer.zaehler).toEqual({ aktiv: 1, archiv: 1, alle: 2 });
  expect(baer.zeilen.map((z) => z.plan.id)).toEqual(["a", "c"]);
  // Die Kopfzahl zählt ohne Suche.
  expect(baer.aktivGesamt).toBe(2);
});

// ---------------------------------------------------------------- Oberfläche

test("Suche: Titel, Person und Stichwort, Umlautvertrag, Hervorhebung und Leerzustand", async ({
  page,
}) => {
  await installMock(page);
  await mitWeiteren(page);
  await oeffne(page);
  await expect(page.getByTestId("planung-zeile")).toHaveCount(9);
  const box = suche(page);
  await expect(box).toHaveAttribute(
    "placeholder",
    "Titel, Person oder Stichwort",
  );

  // „ba“ findet „Bär“ nicht (Treffer erst mit „bä“ oder „bae“).
  await box.fill("ba");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(0);
  const leer = page.getByTestId("planung-leer");
  await expect(leer).toHaveAttribute("data-art", "suche");
  await expect(leer).toContainText("Keine Planung enthält „ba“.");
  await expect(page.getByRole("tab", { name: /Aktiv/ })).toHaveText("Aktiv 0");

  for (const eingabe of ["bä", "bae", "Bär"]) {
    await box.fill(eingabe);
    await expect(page.getByTestId("planung-zeile")).toHaveCount(1);
    await expect(zeile(page, "Rückblick Bärenbrunnen")).toBeVisible();
  }
  await expect(
    zeile(page, "Rückblick Bärenbrunnen").locator("mark"),
  ).toHaveText("Bär");

  // Person aus den Rollen, Straße als „strasse“, Stichwort aus der Agenda.
  await box.fill("schae");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(1);
  await expect(zeile(page, "Begehung Hauptstraße 1")).toBeVisible();
  await box.fill("strasse");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(1);
  await expect(
    zeile(page, "Begehung Hauptstraße 1").locator("mark"),
  ).toHaveText("straße");
  await box.fill("optionen");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(9);
  // Die Zähler folgen der Suche, auch für den anderen Filter.
  await expect(page.getByRole("tab", { name: /Archiv/ })).toHaveText(
    "Archiv 2",
  );

  await box.fill("xyz");
  await page
    .getByTestId("planung-leer")
    .getByRole("button", { name: "Suche löschen" })
    .click();
  await expect(box).toHaveValue("");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(9);
});

test("Gruppen und Zähler: Zeitgruppen in Reihenfolge, Kopf „n aktiv“, Zähler je Filter", async ({
  page,
}) => {
  await installMock(page);
  await mitWeiteren(page);
  await oeffne(page);
  const liste = page.getByTestId("planung-liste");
  await expect(liste.getByRole("heading", { level: 2 })).toHaveText(
    "Planungen 9 aktiv",
  );
  await expect(page.getByTestId("planung-gruppe")).toHaveText([
    "Früher",
    "Diese Woche",
    "Nächste Woche",
    "Später",
    "Ohne Termin",
  ]);
  // Jede Gruppe ist eine eigene Liste mit ihrer Überschrift als Name.
  const naechste = liste.getByRole("list", { name: "Nächste Woche" });
  await expect(naechste.getByTestId("planung-zeile")).toHaveCount(2);
  await expect(
    liste
      .getByRole("list", { name: "Diese Woche" })
      .getByTestId("planung-zeile"),
  ).toHaveCount(3);
  await expect(page.getByRole("tab", { name: /Aktiv/ })).toHaveText("Aktiv 9");
  await expect(page.getByRole("tab", { name: /Archiv/ })).toHaveText(
    "Archiv 2",
  );
  await expect(page.getByRole("tab", { name: /Alle/ })).toHaveText("Alle 11");
  // Online-Beitritt nur, wo der Termin einen Link hat; Datum ohne Jahr im laufenden Jahr.
  const baer = zeile(page, "Rückblick Bärenbrunnen");
  await expect(baer.getByText("Online-Beitritt")).toHaveCount(1);
  await expect(baer).toContainText("Di., 03.11., 08:33");
  await expect(
    zeile(page, "Planung Quartal 1").getByText("Online-Beitritt"),
  ).toHaveCount(0);

  await page.getByRole("tab", { name: /Archiv/ }).click();
  await expect(page.getByTestId("planung-gruppe")).toHaveText([
    "Früher",
    "Diese Woche",
  ]);
  await expect(page.getByTestId("planung-zeile")).toHaveCount(2);
});

test("Zustandschip: genau einer je Zeile, abgeleitet aus Status und Abgleich", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page);
  const zeilen = page.getByTestId("planung-zeile");
  await expect(zeilen).toHaveCount(5);
  for (let i = 0; i < 5; i += 1)
    await expect(zeilen.nth(i).locator(".fui-Badge")).toHaveCount(1);
  const erwartet: [string, string][] = [
    ["Abstimmung Pilotstart", "entwurf"],
    ["Jour fixe Betrieb", "uebergeben"],
    ["Planung Quartal 1", "verknuepft"],
    ["Review Datenschutz", "verschoben"],
    ["Workshop Rollen", "konflikt"],
  ];
  for (const [titel, marke] of erwartet)
    await expect(
      zeile(page, titel).getByTestId(`planung-status-${marke}`),
    ).toBeVisible();
  // Chip mit Symbol und Text, nie nur Farbe.
  const chip = zeile(page, "Workshop Rollen").getByTestId(
    "planung-status-konflikt",
  );
  await expect(chip).toHaveText("Konflikt");
  await expect(chip.locator("svg")).toHaveCount(1);
  // Die Auswahl ändert den Chip nicht; die gewählte Zeile trägt aria-current.
  await zeile(page, "Planung Quartal 1").click();
  await expect(zeile(page, "Planung Quartal 1")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(
    zeile(page, "Planung Quartal 1").getByTestId("planung-status-verknuepft"),
  ).toBeVisible();
});

test("Schublade bei 900 px: öffnen und schließen per Tastatur, Fokus zurück, Wahl schließt", async ({
  page,
}) => {
  await installMock(page);
  await oeffne(page, 900, 700);
  // Ohne gewählte Planung ist die Liste der Inhalt, keine Schublade.
  await expect(page.getByTestId("planung-liste")).toBeVisible();
  await expect(page.getByTestId("planung-schublade-auf")).toHaveCount(0);
  await zeile(page, "Planung Quartal 1").click();
  await expect(page.getByTestId("planung-insel")).toBeVisible();
  await expect(page.getByTestId("planung-liste")).toHaveCount(0);
  await keinSeitwaertsRollen(page);

  const knopf = page.getByRole("button", { name: "Planungen anzeigen" });
  await expect(knopf).toHaveAttribute("aria-expanded", "false");
  await knopf.focus();
  await page.keyboard.press("Enter");
  const schublade = page.getByRole("dialog", { name: "Planungen" });
  await expect(schublade).toBeVisible();
  await expect(knopf).toHaveAttribute("aria-expanded", "true");
  await expect(suche(page)).toBeFocused();
  // Suche und Filter stehen auch hier bereit.
  await expect(schublade.getByRole("tab", { name: /Aktiv/ })).toHaveText(
    "Aktiv 5",
  );
  await expect(
    schublade
      .getByTestId("planung-zeile")
      .filter({ hasText: "Planung Quartal 1" }),
  ).toHaveAttribute("aria-current", "true");

  await page.keyboard.press("Escape");
  await expect(schublade).toHaveCount(0);
  await expect(knopf).toBeFocused();
  await expect(knopf).toHaveAttribute("aria-expanded", "false");

  // Schließen-Knopf per Tastatur.
  await page.keyboard.press("Enter");
  await expect(schublade).toBeVisible();
  const zu = schublade.getByRole("button", { name: "Planungen schließen" });
  await zu.focus();
  await page.keyboard.press("Enter");
  await expect(schublade).toHaveCount(0);
  await expect(knopf).toBeFocused();

  // Eine Zeile wählen: Schublade zu, neue Planung offen, Fokus wieder am Auslöser.
  await page.keyboard.press("Enter");
  await suche(page).fill("review");
  const review = schublade
    .getByTestId("planung-zeile")
    .filter({ hasText: "Review Datenschutz" });
  await review.focus();
  await page.keyboard.press("Enter");
  await expect(schublade).toHaveCount(0);
  await expect(page.getByTestId("planung-insel")).toContainText(
    "Review Datenschutz",
  );
  await expect(knopf).toBeFocused();
  // Suche und Filter bleiben erhalten.
  await page.keyboard.press("Enter");
  await expect(suche(page)).toHaveValue("review");
  await page.keyboard.press("Escape");

  // Ab 1000 px steht die Liste wieder daneben, der Knopf entfällt.
  await page.setViewportSize({ width: 1100, height: 700 });
  await expect(page.getByTestId("planung-liste")).toBeVisible();
  await expect(page.getByTestId("planung-schublade-auf")).toHaveCount(0);
  await expect(page.getByTestId("planung-insel")).toBeVisible();
});

test("480 px: Liste ODER Planung, ohne Seitwärtsrollen", async ({ page }) => {
  await installMock(page);
  await mitWeiteren(page);
  await oeffne(page, 480, 420);
  await expect(page.getByTestId("planung-liste")).toBeVisible();
  await expect(page.getByTestId("planung-detail")).toHaveCount(0);
  await keinSeitwaertsRollen(page);
  await suche(page).fill("bä");
  await expect(page.getByTestId("planung-zeile")).toHaveCount(1);
  await keinSeitwaertsRollen(page);
  await suche(page).fill("");
  await zeile(page, "Review Datenschutz").click();
  await expect(page.getByTestId("planung-detail")).toBeVisible();
  await expect(page.getByTestId("planung-liste")).toHaveCount(0);
  await expect(page.getByTestId("planung-schublade-auf")).toHaveCount(0);
  await keinSeitwaertsRollen(page);
  await page.getByTestId("planung-zurueck").click();
  await expect(page.getByTestId("planung-liste")).toBeVisible();
});

test("Leerzustand: keine Planung, daneben „Erste Planung anlegen“", async ({
  page,
}) => {
  await installMock(page, { leer: true });
  await oeffne(page);
  const leer = page.getByTestId("planung-leer");
  await expect(leer).toHaveAttribute("data-art", "keine");
  await expect(leer).toContainText("Noch keine Planung.");
  for (const name of ["Aktiv 0", "Archiv 0", "Alle 0"])
    await expect(page.getByRole("tab", { name })).toBeVisible();
  await expect(page.getByTestId("planung-start")).toHaveCount(0);
  const erste = page.getByTestId("planung-erste");
  await expect(
    erste.getByRole("heading", { name: "Erste Planung anlegen" }),
  ).toBeVisible();
  await erste.getByRole("button", { name: "Neue Planung" }).click();
  await expect(page.getByTestId("planung-formular")).toBeVisible();
});

test("Leerzustand Archiv: Hinweis und „Aktive anzeigen“", async ({ page }) => {
  await installMock(page);
  await zusaetzlich(page, [], {}, true);
  await oeffne(page);
  await expect(page.getByRole("tab", { name: /Archiv/ })).toHaveText(
    "Archiv 0",
  );
  await page.getByRole("tab", { name: /Archiv/ }).click();
  const leer = page.getByTestId("planung-leer");
  await expect(leer).toHaveAttribute("data-art", "archiv");
  await expect(leer).toContainText("Das Archiv ist leer.");
  await leer.getByRole("button", { name: "Aktive anzeigen" }).click();
  await expect(page.getByRole("tab", { name: /Aktiv/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByTestId("planung-zeile")).toHaveCount(5);
});

test("Ladezustand: Platzhalterzeilen, aria-busy und Statusmeldung, danach die Liste", async ({
  page,
}) => {
  await installMock(page);
  await page.addInitScript(() => {
    if (window.top !== window) return;
    (
      window as unknown as { __steuerung: Record<string, unknown> }
    ).__steuerung.besprechungsplanung_liste = { pause: true };
  });
  await oeffne(page);
  const liste = page.getByTestId("planung-liste");
  await expect(page.getByTestId("planung-liste-laden")).toBeVisible();
  await expect(liste).toHaveAttribute("aria-busy", "true");
  await expect(liste.getByRole("status")).toHaveText(
    "Planungen werden geladen …",
  );
  await expect(page.getByTestId("planung-leer")).toHaveCount(0);
  // Während des Ladens keine Zähler (keine falsche „0“).
  await expect(page.getByRole("tab", { name: /Aktiv/ })).toHaveText("Aktiv");
  await page.evaluate(() => {
    const w = window as unknown as {
      __steuerung: Record<string, unknown>;
      __freigaben: Record<string, (() => void)[]>;
    };
    w.__steuerung.besprechungsplanung_liste = {};
    for (const f of w.__freigaben.besprechungsplanung_liste ?? []) f();
  });
  await expect(page.getByTestId("planung-zeile")).toHaveCount(5);
  await expect(page.getByTestId("planung-liste-laden")).toHaveCount(0);
  await expect(liste).not.toHaveAttribute("aria-busy", "true");
});

test("Ladefehler: Meldung mit „Erneut versuchen“", async ({ page }) => {
  await installMock(page);
  await page.addInitScript(() => {
    if (window.top !== window) return;
    (
      window as unknown as { __steuerung: Record<string, unknown> }
    ).__steuerung.besprechungsplanung_liste = { fehler: true };
  });
  await oeffne(page);
  const liste = page.getByTestId("planung-liste");
  await expect(liste.getByRole("alert")).toHaveText(
    "Die Planungen konnten nicht geladen werden.",
  );
  await expect(page.getByTestId("planung-leer")).toHaveCount(0);
  await page.evaluate(() => {
    (
      window as unknown as { __steuerung: Record<string, unknown> }
    ).__steuerung.besprechungsplanung_liste = {};
  });
  await page.getByTestId("planung-liste-erneut").click();
  await expect(page.getByTestId("planung-zeile")).toHaveCount(5);
  await expect(liste.getByRole("alert")).toHaveCount(0);
});
