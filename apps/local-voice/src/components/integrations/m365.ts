import type { Capability } from "@/bindings";

/** Fähigkeiten, die sich am Microsoft-365-Konto einschalten lassen (Backend: `ENABLEABLE`). */
export const M365_CAPABILITIES: Capability[] = [
  "mail.send",
  "files.write",
  "calendar.write",
];

/** Fehler der Kommandos: `code` oder `code|detail` (Backend: `M365Error::wire`). */
export const splitWire = (raw: string): { code: string; detail: string } => {
  const at = raw.indexOf("|");
  return at < 0
    ? { code: raw, detail: "" }
    : { code: raw.slice(0, at), detail: raw.slice(at + 1) };
};

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** Klartext zu einem Fehlercode des Kontos (Code und Zusatz werden übersetzt). */
export const m365ErrorText = (t: Translate, raw: string): string => {
  const { code, detail } = splitWire(raw);
  if (!code) return t("integrations.errors.generic");
  if (code === "m365_gate") {
    const why = t(`integrations.m365.gate.${detail}`, { defaultValue: "" });
    return t("integrations.m365.errors.m365_gate", {
      detail: why || detail,
    });
  }
  const text = t(`integrations.m365.errors.${code}`, {
    detail,
    defaultValue: "",
  });
  return text || raw || t("integrations.errors.generic");
};

/** Eingeschaltete Fähigkeiten aus der Konfiguration (leer, wenn unlesbar). */
export const m365Capabilities = (configJson: string): Capability[] => {
  try {
    const parsed = JSON.parse(configJson) as {
      enabled_capabilities?: unknown;
    };
    const list = Array.isArray(parsed.enabled_capabilities)
      ? parsed.enabled_capabilities
      : [];
    return M365_CAPABILITIES.filter((c) => list.includes(c));
  } catch {
    return [];
  }
};
