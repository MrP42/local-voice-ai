/** Wie lange das Ermitteln einer fehlenden Dauer höchstens wartet. */
const PROBE_TIMEOUT_MS = 8000;

/** Dauer einer Hördatei in ms, ohne sie abzuspielen. */
export function probeDuration(url: string): Promise<number | null> {
  return new Promise((resolve) => {
    const el = new Audio();
    let settled = false;
    const finish = (ms: number | null) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      el.removeAttribute("src");
      el.load();
      resolve(ms);
    };
    const timer = window.setTimeout(() => finish(null), PROBE_TIMEOUT_MS);
    el.preload = "metadata";
    el.onloadedmetadata = () =>
      finish(
        Number.isFinite(el.duration) && el.duration > 0
          ? Math.round(el.duration * 1000)
          : null,
      );
    el.onerror = () => finish(null);
    el.src = url;
  });
}
