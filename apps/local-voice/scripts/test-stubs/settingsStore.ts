export const useSettingsStore = {
  subscribe: () => () => undefined,
  // `globalThis.__settings`: der Test setzt Einstellungen (sonst keine).
  getState: () => ({
    settings:
      (globalThis as unknown as { __settings?: unknown }).__settings ?? null,
  }),
};
