import { test, expect } from "@playwright/test";
import { installMock } from "./planungMock";

// lva#838: Der Größengreifer eines mehrzeiligen Felds sitzt unten rechts in der Ecke des
// sichtbaren Rahmens. Das innere <textarea> muss den Fluent-Rahmen deshalb ganz ausfüllen,
// vor und nach einer Größenänderung.
test.use({ locale: "de-DE", timezoneId: "Europe/Berlin" });

const MESSFELDER = ["planung-ziel", "planung-ergebnis"];

async function abstaende(
  page: import("@playwright/test").Page,
  testId: string,
) {
  return page.getByTestId(testId).evaluate((ta) => {
    const rahmen = ta.parentElement!.getBoundingClientRect();
    const feld = ta.getBoundingClientRect();
    return {
      unten: rahmen.bottom - feld.bottom,
      rechts: rahmen.right - feld.right,
    };
  });
}

for (const thema of ["light", "dark"] as const) {
  test(`Greifer sitzt unten rechts im Rahmen (${thema})`, async ({ page }) => {
    await installMock(page, { theme: thema });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
    await expect(page.getByTestId("planung-seite")).toBeVisible();
    await page.getByTestId("planung-neu").click();
    for (const id of MESSFELDER) {
      const vorher = await abstaende(page, id);
      expect(vorher.unten, `${id}: Abstand unten`).toBeLessThanOrEqual(4);
      expect(vorher.rechts, `${id}: Abstand rechts`).toBeLessThanOrEqual(4);
      // Größe ändern wie mit dem Greifer.
      await page.getByTestId(id).evaluate((ta) => {
        (ta as HTMLTextAreaElement).style.height = "220px";
      });
      const nachher = await abstaende(page, id);
      expect(
        nachher.unten,
        `${id}: Abstand unten nach dem Ziehen`,
      ).toBeLessThanOrEqual(4);
    }
  });
}
