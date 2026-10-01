import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type Capability,
  type FilesMode,
  type IntegrationView,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";
import { capabilityKey, errorText } from "./model";
import { M365_CAPABILITIES, m365ErrorText } from "./m365";

interface M365DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (view: IntegrationView) => void;
}

const DEFAULT_CAPABILITIES: Capability[] = ["mail.send", "files.write"];
const DEFAULT_FOLDER = "Local Voice AI";

/**
 * Microsoft-365-Konto anlegen (A5): Name, Client-ID der eigenen App-Registrierung,
 * Verzeichnis, Fähigkeiten und OneDrive-Ablage. Angemeldet wird erst im Detail
 * („Mit Microsoft anmelden“); hier entsteht nur der Eintrag, ohne Geheimnis. Die
 * Anmeldung fragt später nur die Berechtigungen der hier eingeschalteten
 * Fähigkeiten an.
 */
export const M365Dialog: React.FC<M365DialogProps> = ({
  open: isOpen,
  onOpenChange,
  onCreated,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [clientId, setClientId] = useState("");
  const [tenant, setTenant] = useState("");
  const [caps, setCaps] = useState<Capability[]>(DEFAULT_CAPABILITIES);
  const [filesMode, setFilesMode] = useState<FilesMode>("full");
  const [folder, setFolder] = useState(DEFAULT_FOLDER);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    setName(t("integrations.m365.dialog.defaultName"));
    setClientId("");
    setTenant("");
    setCaps(DEFAULT_CAPABILITIES);
    setFilesMode("full");
    setFolder(DEFAULT_FOLDER);
    setBusy(false);
    setError(null);
  }, [isOpen, t]);

  const toggle = (cap: Capability) =>
    setCaps((list) =>
      list.includes(cap) ? list.filter((c) => c !== cap) : [...list, cap],
    );

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await commands.m365Create(
        name.trim(),
        clientId.trim() || null,
        tenant.trim() || null,
        caps,
        filesMode,
        folder.trim(),
      );
      if (result.status === "ok") {
        onCreated(result.data);
        onOpenChange(false);
        return;
      }
      setError(m365ErrorText(t, errorText(result.error)));
    } catch (e) {
      setError(m365ErrorText(t, errorText(e)));
    }
    setBusy(false);
  };

  const filesOn = caps.includes("files.write");

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("integrations.m365.dialog.title")}
      description={t("integrations.m365.dialog.description")}
      closeLabel={t("integrations.close")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid="m365-cancel-create"
          >
            {t("integrations.cancel")}
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={name.trim() === "" || busy}
            data-testid="m365-submit"
          >
            {t("integrations.m365.dialog.create")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="m365-dialog">
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="m365-name">
            {t("integrations.m365.dialog.name")}
          </label>
          <Input
            id="m365-name"
            className="w-full"
            value={name}
            maxLength={120}
            onChange={(e) => setName(e.target.value)}
            data-testid="m365-name"
          />
        </div>
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="m365-client">
            {t("integrations.m365.dialog.clientId")}
          </label>
          <Input
            id="m365-client"
            className="w-full"
            value={clientId}
            placeholder="00000000-0000-0000-0000-000000000000"
            spellCheck={false}
            onChange={(e) => {
              setClientId(e.target.value);
              setError(null);
            }}
            data-testid="m365-client-id"
          />
          <p className="text-xs text-text-muted">
            {t("integrations.m365.dialog.clientIdHint")}
          </p>
        </div>
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="m365-tenant">
            {t("integrations.m365.dialog.tenant")}
          </label>
          <Input
            id="m365-tenant"
            className="w-full"
            value={tenant}
            placeholder="common"
            spellCheck={false}
            onChange={(e) => setTenant(e.target.value)}
            data-testid="m365-tenant"
          />
          <p className="text-xs text-text-muted">
            {t("integrations.m365.dialog.tenantHint")}
          </p>
        </div>
        <fieldset className="space-y-1.5">
          <legend className="text-sm font-medium">
            {t("integrations.m365.dialog.capabilities")}
          </legend>
          <p className="text-xs text-text-muted">
            {t("integrations.m365.dialog.capabilitiesHint")}
          </p>
          {M365_CAPABILITIES.map((cap) => (
            <label
              key={cap}
              className="flex cursor-pointer items-start gap-2 text-sm"
            >
              <input
                type="checkbox"
                className="mt-1"
                checked={caps.includes(cap)}
                onChange={() => toggle(cap)}
                data-testid={`m365-cap-${cap}`}
              />
              <span>
                <span className="font-medium">
                  {t(`${capabilityKey(cap)}.title`)}
                </span>
                <span className="block text-xs text-text-muted">
                  {t(`integrations.m365.caps.${cap.replace(".", "_")}`)}
                </span>
              </span>
            </label>
          ))}
        </fieldset>
        {filesOn && (
          <fieldset className="space-y-1.5">
            <legend className="text-sm font-medium">
              {t("integrations.m365.dialog.filesMode")}
            </legend>
            {(["full", "app_folder"] as FilesMode[]).map((mode) => (
              <label
                key={mode}
                className="flex cursor-pointer items-start gap-2 text-sm"
              >
                <input
                  type="radio"
                  name="m365-files-mode"
                  className="mt-1"
                  checked={filesMode === mode}
                  onChange={() => setFilesMode(mode)}
                  data-testid={`m365-files-${mode}`}
                />
                <span>
                  {t(
                    mode === "full"
                      ? "integrations.m365.dialog.filesFull"
                      : "integrations.m365.dialog.filesApp",
                  )}
                </span>
              </label>
            ))}
            <label className="block text-sm font-medium" htmlFor="m365-folder">
              {t("integrations.m365.dialog.filesFolder")}
            </label>
            <Input
              id="m365-folder"
              className="w-full"
              value={folder}
              onChange={(e) => setFolder(e.target.value)}
              data-testid="m365-folder"
            />
          </fieldset>
        )}
        {error && (
          <p
            className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red"
            role="alert"
            data-testid="m365-create-error"
          >
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
