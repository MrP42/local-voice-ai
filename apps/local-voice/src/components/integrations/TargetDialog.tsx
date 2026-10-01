import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type IntegrationView,
  type Security,
  type TargetSettings,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { Select } from "../ui/Select";
import {
  CONTEXT_AREAS,
  TIERS,
  configText,
  errorText,
  type TargetKind,
} from "./model";

interface TargetDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Welche Art eingerichtet oder bearbeitet wird. */
  kind: TargetKind;
  /** Gesetzt: vorhandene Integration bearbeiten (Einstellungen), sonst neu anlegen. */
  editing?: IntegrationView;
  onSaved: (view: IntegrationView) => void;
}

const EMPTY_SETTINGS = {
  name: "",
  path: "",
  subfolder: "",
  host: "",
  port: "587",
  security: "starttls" as Security,
  username: "",
  fromAddress: "",
  fromName: "",
  contextArea: "beruf",
  tier: "propose",
  endpoint: "",
  tool: "wissen_suchen",
  area: "",
  secret: "",
};

type Form = typeof EMPTY_SETTINGS;

const formFrom = (view: IntegrationView | undefined): Form => {
  if (!view) return { ...EMPTY_SETTINGS };
  const c = (key: string) => configText(view.integration.config_json, key);
  return {
    ...EMPTY_SETTINGS,
    name: view.integration.label,
    path: c("path"),
    subfolder: c("subfolder"),
    host: c("host"),
    port: c("port") || EMPTY_SETTINGS.port,
    security: c("security") === "tls" ? "tls" : "starttls",
    username: c("username"),
    fromAddress: c("from_address"),
    fromName: c("from_name"),
    contextArea: c("context_area") || EMPTY_SETTINGS.contextArea,
    tier: c("tier") || EMPTY_SETTINGS.tier,
    endpoint: c("endpoint"),
    tool: c("search_tool") || EMPTY_SETTINGS.tool,
    area: c("area"),
  };
};

/** Letzter Ordnername eines Pfads, als Vorschlag fuer den Namen. */
const nameFromPath = (path: string): string => {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : "";
};

const DEFAULT_PORT: Record<Security, string> = {
  starttls: "587",
  tls: "465",
  plain: "25",
};

/**
 * Formular je Art (A6): SMTP-Postfach, Obsidian-Vault, WAI-Wissensbasis und die
 * Ordner-Einstellungen. Das Passwort bzw. der Schluessel geht einmal ans Backend und
 * wird dort verschluesselt abgelegt; die Oberflaeche bekommt es nie zurueck (beim
 * Bearbeiten bleibt das Feld leer = unveraendert).
 */
export const TargetDialog: React.FC<TargetDialogProps> = ({
  open: isOpen,
  onOpenChange,
  kind,
  editing,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [form, setForm] = useState<Form>(formFrom(editing));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nameTouched, setNameTouched] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    setForm(formFrom(editing));
    setBusy(false);
    setError(null);
    setNameTouched(false);
  }, [isOpen, editing, kind]);

  const set = <K extends keyof Form>(key: K, value: Form[K]) => {
    setForm((f) => ({ ...f, [key]: value }));
    setError(null);
  };

  const explain = (raw: string) => {
    const text = t(`integrations.errors.${raw}`, { defaultValue: "" });
    return text || raw || t("integrations.errors.generic");
  };

  const pick = async () => {
    try {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) {
        setForm((f) => ({
          ...f,
          path: picked,
          name: !nameTouched && !editing ? nameFromPath(picked) : f.name,
        }));
        setError(null);
      }
    } catch {
      /* Abbruch des Dialogs */
    }
  };

  const settings = (): TargetSettings => {
    const base: TargetSettings = {
      path: null,
      subfolder: null,
      host: null,
      port: null,
      security: null,
      username: null,
      from_address: null,
      from_name: null,
      context_area: null,
      tier: null,
      endpoint: null,
      search_tool: null,
      area: null,
      secret: null,
    };
    switch (kind) {
      case "folder":
        return { ...base, path: form.path.trim(), subfolder: form.subfolder };
      case "smtp":
        return {
          ...base,
          host: form.host.trim(),
          port: Number.parseInt(form.port, 10) || 0,
          security: form.security,
          username: form.username.trim(),
          from_address: form.fromAddress.trim(),
          from_name: form.fromName.trim(),
          secret: form.secret || null,
        };
      case "obsidian":
        return {
          ...base,
          path: form.path.trim(),
          subfolder: form.subfolder.trim(),
          context_area: form.contextArea,
          tier: form.tier,
        };
      case "wissen":
        return {
          ...base,
          endpoint: form.endpoint.trim(),
          search_tool: form.tool.trim(),
          area: form.area.trim(),
          secret: form.secret || null,
        };
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = editing
        ? await commands.integrationUpdateSettings(
            editing.integration.id,
            settings(),
          )
        : await commands.integrationCreateWithSettings(
            kind,
            form.name.trim(),
            null,
            settings(),
          );
      if (result.status === "ok") {
        onSaved(result.data);
        onOpenChange(false);
        return;
      }
      setError(explain(errorText(result.error)));
    } catch (e) {
      setError(explain(errorText(e)));
    }
    setBusy(false);
  };

  const filled = (s: string) => s.trim() !== "";
  const needsPassword = kind === "smtp" && filled(form.username) && !editing;
  const ready = (() => {
    if (!editing && !filled(form.name)) return false;
    switch (kind) {
      case "folder":
        return filled(form.path);
      case "smtp":
        return (
          filled(form.host) &&
          filled(form.fromAddress) &&
          (!needsPassword || filled(form.secret))
        );
      case "obsidian":
        return filled(form.path);
      case "wissen":
        return filled(form.endpoint) && (!!editing || filled(form.secret));
    }
  })();

  const title = editing
    ? t(`integrations.target.${kind}.titleEdit`, {
        defaultValue: t("integrations.target.folder.title"),
      })
    : t(`integrations.target.${kind}.title`);

  const field = (
    id: string,
    label: string,
    input: React.ReactNode,
    hint?: string,
  ) => (
    <div className="space-y-1">
      <label className="text-sm font-medium" htmlFor={id}>
        {label}
      </label>
      {input}
      {hint && <p className="text-xs text-text-muted">{hint}</p>}
    </div>
  );

  const text = (
    key: keyof Form,
    id: string,
    extra: Partial<React.InputHTMLAttributes<HTMLInputElement>> = {},
  ) => (
    <Input
      id={id}
      className="w-full"
      value={form[key] as string}
      onChange={(e) => set(key, e.target.value as never)}
      data-testid={`target-${id}`}
      {...extra}
    />
  );

  const pathField = (label: string, placeholder: string) =>
    field(
      "path",
      label,
      <div className="flex flex-wrap gap-2">
        <Input
          id="path"
          className="min-w-0 flex-1 basis-48"
          value={form.path}
          placeholder={placeholder}
          onChange={(e) => set("path", e.target.value)}
          data-testid="target-path"
        />
        <Button
          variant="secondary"
          onClick={() => void pick()}
          data-testid="target-pick"
        >
          {t("integrations.target.pick")}
        </Button>
      </div>,
    );

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={title}
      description={
        editing ? undefined : t(`integrations.target.${kind}.description`)
      }
      closeLabel={t("integrations.close")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid="target-cancel"
          >
            {t("integrations.cancel")}
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={!ready || busy}
            data-testid="target-submit"
          >
            {editing
              ? t("integrations.target.save")
              : t("integrations.target.create")}
          </Button>
        </>
      }
    >
      <div
        className="space-y-3"
        data-testid="target-dialog"
        data-kind={kind}
        data-mode={editing ? "edit" : "create"}
      >
        {!editing &&
          field(
            "name",
            t("integrations.target.name"),
            <Input
              id="name"
              className="w-full"
              value={form.name}
              maxLength={120}
              onChange={(e) => {
                setNameTouched(true);
                set("name", e.target.value);
              }}
              data-testid="target-name"
            />,
          )}

        {kind === "smtp" && (
          <>
            {field(
              "host",
              t("integrations.target.smtp.host"),
              text("host", "host", {
                placeholder: t("integrations.target.smtp.hostPlaceholder"),
                autoComplete: "off",
              }),
            )}
            <div className="flex flex-wrap gap-3">
              <div className="w-28 space-y-1">
                <label className="text-sm font-medium" htmlFor="port">
                  {t("integrations.target.smtp.port")}
                </label>
                {text("port", "port", { inputMode: "numeric", maxLength: 5 })}
              </div>
              <div className="space-y-1">
                <div className="text-sm font-medium" id="smtp-security">
                  {t("integrations.target.smtp.security")}
                </div>
                <div
                  role="radiogroup"
                  aria-labelledby="smtp-security"
                  className="inline-flex max-w-full flex-wrap overflow-hidden rounded-lg border border-mid-gray/30"
                >
                  {(["starttls", "tls"] as const).map((s) => (
                    <button
                      key={s}
                      type="button"
                      role="radio"
                      aria-checked={form.security === s}
                      data-testid={`target-security-${s}`}
                      onClick={() => {
                        setForm((f) => ({
                          ...f,
                          security: s,
                          port:
                            f.port === DEFAULT_PORT[f.security]
                              ? DEFAULT_PORT[s]
                              : f.port,
                        }));
                        setError(null);
                      }}
                      className={`min-h-9 cursor-pointer px-3 py-1 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
                        form.security === s
                          ? "bg-logo-primary text-on-accent"
                          : "text-text hover:bg-mid-gray/15"
                      }`}
                    >
                      {t(`integrations.target.smtp.${s}`)}
                    </button>
                  ))}
                </div>
              </div>
            </div>
            {field(
              "username",
              t("integrations.target.smtp.username"),
              text("username", "username", { autoComplete: "off" }),
            )}
            {field(
              "from-address",
              t("integrations.target.smtp.fromAddress"),
              text("fromAddress", "from-address", {
                type: "email",
                autoComplete: "off",
              }),
            )}
            {field(
              "from-name",
              t("integrations.target.smtp.fromName"),
              text("fromName", "from-name", { maxLength: 120 }),
            )}
            {field(
              "secret",
              t("integrations.target.smtp.password"),
              text("secret", "secret", {
                type: "password",
                autoComplete: "new-password",
                placeholder: editing
                  ? t("integrations.target.secretKeep", {
                      what: t("integrations.target.smtp.passwordWhat"),
                    })
                  : undefined,
              }),
              t("integrations.target.smtp.passwordHint"),
            )}
          </>
        )}

        {kind === "obsidian" && (
          <>
            {pathField(
              t("integrations.target.obsidian.path"),
              t("integrations.target.obsidian.pathPlaceholder"),
            )}
            {field(
              "subfolder",
              t("integrations.target.obsidian.subfolder"),
              text("subfolder", "subfolder", { placeholder: "00_inbox" }),
            )}
            <div className="flex flex-wrap gap-3">
              <div className="min-w-0 flex-1 basis-48 space-y-1">
                <span className="text-sm font-medium">
                  {t("integrations.target.obsidian.contextArea")}
                </span>
                <Select
                  ariaLabel={t("integrations.target.obsidian.contextArea")}
                  value={form.contextArea}
                  isClearable={false}
                  menuPortal
                  options={CONTEXT_AREAS.map((a) => ({
                    value: a,
                    label: t(`integrations.areas.${a}`),
                  }))}
                  onChange={(v) => set("contextArea", v ?? "beruf")}
                />
              </div>
              <div className="min-w-0 flex-1 basis-48 space-y-1">
                <span className="text-sm font-medium">
                  {t("integrations.target.obsidian.tier")}
                </span>
                <Select
                  ariaLabel={t("integrations.target.obsidian.tier")}
                  value={form.tier}
                  isClearable={false}
                  menuPortal
                  options={TIERS.map((v) => ({
                    value: v,
                    label: t(`integrations.tiers.${v}`),
                  }))}
                  onChange={(v) => set("tier", v ?? "propose")}
                />
              </div>
            </div>
            <p className="text-xs text-text-muted">
              {t("integrations.target.obsidian.hint")}
            </p>
          </>
        )}

        {kind === "wissen" && (
          <>
            {field(
              "endpoint",
              t("integrations.target.wissen.endpoint"),
              text("endpoint", "endpoint", {
                placeholder: t(
                  "integrations.target.wissen.endpointPlaceholder",
                ),
                autoComplete: "off",
              }),
            )}
            {field(
              "tool",
              t("integrations.target.wissen.tool"),
              text("tool", "tool"),
            )}
            {field(
              "area",
              t("integrations.target.wissen.area"),
              text("area", "area", { placeholder: "wai" }),
            )}
            {field(
              "secret",
              t("integrations.target.wissen.token"),
              text("secret", "secret", {
                type: "password",
                autoComplete: "new-password",
                placeholder: editing
                  ? t("integrations.target.secretKeep", {
                      what: t("integrations.target.wissen.tokenWhat"),
                    })
                  : undefined,
              }),
              t("integrations.target.wissen.tokenHint"),
            )}
          </>
        )}

        {kind === "folder" && (
          <>
            {pathField(
              t("integrations.target.folder.path"),
              t("integrations.folder.pathPlaceholder"),
            )}
            {field(
              "subfolder",
              t("integrations.target.folder.subfolder"),
              text("subfolder", "subfolder", { placeholder: "Protokolle" }),
              t("integrations.target.folder.subfolderHint"),
            )}
          </>
        )}

        {error && (
          <p
            className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red"
            role="alert"
            data-testid="target-error"
          >
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
