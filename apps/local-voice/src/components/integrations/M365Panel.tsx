import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type Capability,
  type FilesMode,
  type IntegrationView,
  type M365ActionResult,
  type M365Status,
} from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { capabilityKey, errorText } from "./model";
import { M365_CAPABILITIES, m365ErrorText } from "./m365";

interface M365PanelProps {
  view: IntegrationView;
  /** Die Integrationsliste neu laden (Zustand am Eintrag hat sich geändert). */
  onChanged: () => void | Promise<void>;
}

type Busy = "signin" | "signout" | "test" | "mail" | "file" | "settings" | null;
type Message = { kind: "ok" | "error"; text: string };

/**
 * Das Microsoft-365-Konto im Detail (A5): Zustand und Konto, eingeschaltete
 * Fähigkeiten mit den Berechtigungen, die sie anfragen, Anmelden/Abmelden und die
 * Prüfungen für den Eigentümer (Verbindung, Testmail, Testdatei). Fähigkeiten und
 * OneDrive-Ablage gehen sofort ans Backend; Client-ID, Verzeichnis und Ordner
 * speichert ein Knopf. Eine zusätzliche Fähigkeit zeigt „Zustimmung erweitern“,
 * bis man sich erneut angemeldet hat. Die Anmeldung läuft im Systembrowser; hier
 * steht nur, dass sie läuft, und ein Knopf zum Abbrechen.
 */
export const M365Panel: React.FC<M365PanelProps> = ({ view, onChanged }) => {
  const { t } = useTranslation();
  const id = view.integration.id;
  const [status, setStatus] = useState<M365Status | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  const [message, setMessage] = useState<Message | null>(null);
  const [clientId, setClientId] = useState("");
  const [tenant, setTenant] = useState("");
  const [folder, setFolder] = useState("");

  const adopt = useCallback((next: M365Status) => {
    setStatus(next);
    setClientId(next.client_id);
    setTenant(next.tenant);
    setFolder(next.files_folder);
  }, []);

  useEffect(() => {
    let alive = true;
    void commands.m365Status(id).then((result) => {
      if (alive && result.status === "ok") adopt(result.data);
    });
    return () => {
      alive = false;
    };
  }, [id, adopt]);

  const fail = (raw: string) =>
    setMessage({ kind: "error", text: m365ErrorText(t, raw) });

  const run = async (
    kind: Exclude<Busy, null>,
    action: () => Promise<void>,
  ) => {
    setBusy(kind);
    setMessage(null);
    try {
      await action();
    } catch (e) {
      fail(errorText(e));
    }
    setBusy(null);
    await onChanged();
  };

  const settings = (patch: {
    clientId?: string | null;
    tenant?: string | null;
    capabilities?: Capability[] | null;
    filesMode?: FilesMode | null;
    filesFolder?: string | null;
  }) =>
    run("settings", async () => {
      const result = await commands.m365UpdateSettings(
        id,
        patch.clientId ?? null,
        patch.tenant ?? null,
        patch.capabilities ?? null,
        patch.filesMode ?? null,
        patch.filesFolder ?? null,
      );
      if (result.status === "ok") {
        adopt(result.data);
        setMessage({ kind: "ok", text: t("integrations.m365.panel.saved") });
      } else {
        fail(errorText(result.error));
      }
    });

  const signIn = () =>
    run("signin", async () => {
      const result = await commands.m365SignIn(id);
      if (result.status === "ok") {
        adopt(result.data);
        setMessage({
          kind: "ok",
          text: t("integrations.m365.panel.signedIn"),
        });
      } else {
        fail(errorText(result.error));
        const fresh = await commands.m365Status(id);
        if (fresh.status === "ok") adopt(fresh.data);
      }
    });

  const cancelSignIn = async () => {
    await commands.m365CancelSignIn();
  };

  const signOut = () =>
    run("signout", async () => {
      const result = await commands.m365SignOut(id);
      if (result.status === "ok") {
        adopt(result.data);
        setMessage({
          kind: "ok",
          text: t("integrations.m365.panel.signedOut"),
        });
      } else {
        fail(errorText(result.error));
      }
    });

  const outcome = async (
    result: Awaited<ReturnType<typeof commands.m365Test>>,
    okText: (r: M365ActionResult) => string,
  ) => {
    if (result.status === "error") {
      fail(errorText(result.error));
    } else if (result.data.ok) {
      setMessage({ kind: "ok", text: okText(result.data) });
    } else {
      fail(result.data.code);
    }
    const fresh = await commands.m365Status(id);
    if (fresh.status === "ok") adopt(fresh.data);
  };

  const test = () =>
    run("test", async () =>
      outcome(await commands.m365Test(id), (r) =>
        t("integrations.m365.panel.testOk", { account: r.detail ?? "" }),
      ),
    );
  const testMail = () =>
    run("mail", async () =>
      outcome(await commands.m365SendTestMail(id), () =>
        t("integrations.m365.panel.testMailOk"),
      ),
    );
  const testFile = () =>
    run("file", async () =>
      outcome(await commands.m365UploadTestFile(id), (r) =>
        t("integrations.m365.panel.testFileOk", {
          name: r.detail ?? "",
          folder: status?.files_folder ?? "",
        }),
      ),
    );

  if (!status) {
    return (
      <section data-testid="m365-panel" aria-busy="true">
        <p className="text-sm text-text-muted">
          {t("integrations.m365.panel.loading")}
        </p>
      </section>
    );
  }

  const state = status.state;
  const signingIn = busy === "signin" || status.signing_in;
  const disabled = busy !== null;
  const canSignIn =
    state === "needs_sign_in" || state === "needs_consent" || state === "ready";
  const signInLabel =
    state === "needs_consent"
      ? t("integrations.m365.panel.extend")
      : state === "ready"
        ? t("integrations.m365.panel.signInAgain")
        : t("integrations.m365.panel.signIn");
  const enabled = status.enabled_capabilities;
  const ready = state === "ready";
  const dirty =
    clientId.trim() !== status.client_id ||
    tenant.trim() !== status.tenant ||
    folder.trim() !== status.files_folder;

  const toggleCap = (cap: Capability) =>
    void settings({
      capabilities: enabled.includes(cap)
        ? enabled.filter((c) => c !== cap)
        : [...enabled, cap],
    });

  return (
    <section
      className="space-y-4"
      aria-labelledby="int-m365"
      data-testid="m365-panel"
    >
      <h3 id="int-m365" className="text-sm font-semibold">
        {t("integrations.m365.panel.title")}
      </h3>

      <div
        className={`rounded-lg px-3 py-2 text-sm ${
          ready ? "bg-green-500/10" : "bg-amber-500/10"
        }`}
        role="status"
        data-testid="m365-state"
        data-state={state}
      >
        <p className="font-medium">
          {t(`integrations.m365.panel.states.${state}`)}
        </p>
        {status.account && (
          <p className="text-xs text-text-muted" data-testid="m365-account">
            {t("integrations.m365.panel.account", {
              account: status.account,
              name: status.display_name ?? "",
            })}
          </p>
        )}
        {state === "needs_consent" && (
          <p className="text-xs" data-testid="m365-missing">
            {t("integrations.m365.panel.missing", {
              scopes: status.missing_scopes.join(", "),
            })}
          </p>
        )}
      </div>

      <fieldset className="space-y-1.5" disabled={disabled}>
        <legend className="text-sm font-medium">
          {t("integrations.m365.dialog.capabilities")}
        </legend>
        {M365_CAPABILITIES.map((cap) => (
          <label
            key={cap}
            className="flex cursor-pointer items-start gap-2 text-sm"
          >
            <input
              type="checkbox"
              className="mt-1"
              checked={enabled.includes(cap)}
              onChange={() => toggleCap(cap)}
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
        {enabled.includes("files.write") && (
          <div className="ps-6 space-y-1">
            {(["full", "app_folder"] as FilesMode[]).map((mode) => (
              <label
                key={mode}
                className="flex cursor-pointer items-start gap-2 text-sm"
              >
                <input
                  type="radio"
                  name={`m365-files-mode-${id}`}
                  className="mt-1"
                  checked={status.files_mode === mode}
                  onChange={() => void settings({ filesMode: mode })}
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
          </div>
        )}
      </fieldset>

      <div className="space-y-1" data-testid="m365-scopes">
        <div className="text-sm font-medium">
          {t("integrations.m365.panel.scopes")}
        </div>
        <p className="text-xs text-text-muted">
          {t("integrations.m365.panel.scopesHint")}
        </p>
        <ul className="flex flex-wrap gap-1.5">
          {status.required_scopes.map((scope) => (
            <li
              key={scope}
              className={`rounded-full px-2 py-0.5 text-xs ${
                status.missing_scopes.includes(scope)
                  ? "bg-amber-500/20 text-status-amber"
                  : "bg-mid-gray/20 text-text-muted"
              }`}
              data-scope={scope}
            >
              {scope}
            </li>
          ))}
        </ul>
      </div>

      <div className="space-y-2">
        <div className="space-y-1">
          <label className="text-sm font-medium" htmlFor="m365-client-edit">
            {t("integrations.m365.dialog.clientId")}
          </label>
          <Input
            id="m365-client-edit"
            className="w-full"
            value={clientId}
            spellCheck={false}
            disabled={disabled}
            onChange={(e) => setClientId(e.target.value)}
            data-testid="m365-client-id"
          />
        </div>
        <div className="grid gap-2 sm:grid-cols-2">
          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="m365-tenant-edit">
              {t("integrations.m365.dialog.tenant")}
            </label>
            <Input
              id="m365-tenant-edit"
              className="w-full"
              value={tenant}
              spellCheck={false}
              disabled={disabled}
              onChange={(e) => setTenant(e.target.value)}
              data-testid="m365-tenant"
            />
          </div>
          {enabled.includes("files.write") && (
            <div className="space-y-1">
              <label className="text-sm font-medium" htmlFor="m365-folder-edit">
                {t("integrations.m365.dialog.filesFolder")}
              </label>
              <Input
                id="m365-folder-edit"
                className="w-full"
                value={folder}
                disabled={disabled}
                onChange={(e) => setFolder(e.target.value)}
                data-testid="m365-folder"
              />
            </div>
          )}
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={disabled || !dirty}
          onClick={() =>
            void settings({
              clientId: clientId.trim(),
              tenant: tenant.trim(),
              filesFolder: folder.trim(),
            })
          }
          data-testid="m365-save"
        >
          {t("integrations.m365.panel.save")}
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        {canSignIn && !signingIn && (
          <Button
            size="sm"
            variant={ready ? "secondary" : "primary"}
            disabled={disabled}
            onClick={() => void signIn()}
            data-testid="m365-sign-in"
          >
            {signInLabel}
          </Button>
        )}
        {signingIn && (
          <>
            <span
              className="text-sm text-text-muted"
              role="status"
              data-testid="m365-waiting"
            >
              {t("integrations.m365.panel.waiting")}
            </span>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void cancelSignIn()}
              data-testid="m365-cancel-sign-in"
            >
              {t("integrations.cancel")}
            </Button>
          </>
        )}
        {status.secret === "present" && !signingIn && (
          <Button
            size="sm"
            variant="ghost"
            disabled={disabled}
            onClick={() => void signOut()}
            data-testid="m365-sign-out"
          >
            {t("integrations.m365.panel.signOut")}
          </Button>
        )}
      </div>

      {ready && (
        <div className="space-y-2" data-testid="m365-checks">
          <div className="text-sm font-medium">
            {t("integrations.m365.panel.checks")}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={disabled}
              onClick={() => void test()}
              data-testid="m365-test"
            >
              {busy === "test"
                ? t("integrations.detail.testing")
                : t("integrations.detail.test")}
            </Button>
            {enabled.includes("mail.send") && (
              <Button
                size="sm"
                variant="secondary"
                disabled={disabled}
                onClick={() => void testMail()}
                data-testid="m365-test-mail"
              >
                {t("integrations.m365.panel.testMail")}
              </Button>
            )}
            {enabled.includes("files.write") && (
              <Button
                size="sm"
                variant="secondary"
                disabled={disabled}
                onClick={() => void testFile()}
                data-testid="m365-test-file"
              >
                {t("integrations.m365.panel.testFile")}
              </Button>
            )}
          </div>
        </div>
      )}

      <div aria-live="polite" data-testid="m365-message">
        {message && (
          <p
            className={`rounded-lg px-3 py-2 text-sm ${
              message.kind === "ok"
                ? "bg-green-500/10 text-status-green"
                : "bg-red-500/10 text-status-red"
            }`}
            role={message.kind === "error" ? "alert" : "status"}
          >
            {message.text}
          </p>
        )}
      </div>
    </section>
  );
};
