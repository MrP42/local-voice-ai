import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type PendingApproval } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { capabilityKey, errorText } from "./model";

interface ApprovalDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pending: PendingApproval[];
  /** Nach jeder Entscheidung: Liste neu laden. */
  onDecided: () => Promise<void> | void;
}

const stamp = (ms: number, language: string): string =>
  new Date(ms).toLocaleString(language, {
    dateStyle: "short",
    timeStyle: "short",
  });

/**
 * Freigaben („fragen“): was ein Workflow oder Agent tun will, vollstaendig und
 * mit dem Ziel. Die Entscheidung gilt nur fuer diese eine Aktion; sie laeuft
 * nur hier in der Oberflaeche, nie ueber einen Agentenzugang.
 */
export const ApprovalDialog: React.FC<ApprovalDialogProps> = ({
  open,
  onOpenChange,
  pending,
  onDecided,
}) => {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const decide = async (id: string, approve: boolean) => {
    setBusy(id);
    setError(null);
    try {
      const result = await commands.approvalDecide(id, approve);
      if (result.status === "error") {
        const code = errorText(result.error);
        setError(
          t(`integrations.approvals.errors.${code}`, {
            defaultValue: code || t("integrations.errors.generic"),
          }),
        );
      }
    } catch (e) {
      setError(errorText(e) || t("integrations.errors.generic"));
    }
    await onDecided();
    setBusy(null);
  };

  // Nichts mehr offen: der Dialog hat nichts mehr zu zeigen.
  React.useEffect(() => {
    if (open && pending.length === 0 && busy === null && !error) {
      onOpenChange(false);
    }
  }, [open, pending.length, busy, error, onOpenChange]);

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("integrations.approvals.title")}
      description={t("integrations.approvals.description")}
      closeLabel={t("integrations.close")}
      className="max-w-2xl"
    >
      <div className="space-y-3" data-testid="approval-dialog">
        {error && (
          <p
            className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red"
            role="alert"
            data-testid="approval-error"
          >
            {error}
          </p>
        )}
        {pending.length === 0 && (
          <p className="text-sm text-text-muted">
            {t("integrations.approvals.none")}
          </p>
        )}
        <ul className="space-y-3">
          {pending.map(({ approval, integration_label }) => (
            <li
              key={approval.id}
              className="space-y-2 rounded-lg border border-mid-gray/30 p-3"
              data-testid="approval-item"
              data-approval-id={approval.id}
            >
              <p className="text-sm">
                {t("integrations.approvals.request", {
                  caller: t(`integrations.callers.${approval.caller}`, {
                    defaultValue: approval.caller,
                  }),
                  action: t(
                    `${capabilityKey(approval.tool_or_capability)}.title`,
                    { defaultValue: approval.tool_or_capability },
                  ),
                  target:
                    integration_label ?? t("integrations.approvals.unknown"),
                })}
              </p>
              {approval.args_preview && (
                <pre
                  className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md border border-mid-gray/20 bg-mid-gray/10 p-2 text-xs"
                  data-testid="approval-preview"
                >
                  {approval.args_preview}
                </pre>
              )}
              <p className="text-xs text-text-muted">
                {t("integrations.approvals.asked", {
                  time: stamp(approval.created_at, i18n.language),
                })}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  disabled={busy !== null}
                  onClick={() => void decide(approval.id, true)}
                  data-testid="approval-allow"
                >
                  {t("integrations.approvals.allow")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy !== null}
                  onClick={() => void decide(approval.id, false)}
                  data-testid="approval-deny"
                >
                  {t("integrations.approvals.deny")}
                </Button>
              </div>
            </li>
          ))}
        </ul>
        <p className="text-xs text-text-muted">
          {t("integrations.approvals.once")}
        </p>
      </div>
    </Dialog>
  );
};
