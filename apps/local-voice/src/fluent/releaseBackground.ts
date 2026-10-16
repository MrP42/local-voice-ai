/**
 * Ohne offenes Modal ist auf oberster Ebene nichts vor Hilfstechnik
 * verborgen. tabster räumt aria-hidden nicht immer auf, wenn ein Modal
 * (Dialog, Overlay-Drawer) ausgehängt wird (Muster aus @wai/ui Dialog).
 */
export function releaseBackground() {
  if (document.querySelector('[aria-modal="true"]')) return;
  for (const el of Array.from(document.body.children)) {
    if (
      el.getAttribute("aria-hidden") === "true" &&
      !el.hasAttribute("data-tabster-dummy")
    ) {
      el.removeAttribute("aria-hidden");
    }
  }
}

/** Sofort und nach tabsters verzögerter Aktualisierung (250 ms) erneut. */
export function releaseBackgroundSoon() {
  releaseBackground();
  setTimeout(releaseBackground, 300);
}
