import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type LibraryBackup,
  type LibraryLayoutProblem,
  type LibraryLayoutStatus,
} from "@/bindings";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";
import { Alert } from "../ui/Alert";
import { Dialog } from "../ui/Dialog";
import { formatBytes } from "@/lib/library/remove";

/**
 * Einstellungen › Allgemein › „Bibliothek und Sicherungen“ (lva#365 P2b, Spec 4.2/4.3).
 *
 * Zeigt, ob die Bücher schon in ihren Buch-Ordnern liegen, bietet das Neu-ordnen
 * (Dialog nach Vorschau 05-migration) und die Rückkehr zur alten Ordnung an,
 * meldet einen angehaltenen Lauf (06-migfehler) und listet die Sicherungen mit
 * Zurückspielen und – nach 30 Tagen – Löschen. Lehnt das Laufwerk ein sicheres
 * Zurückspielen ab (kein Ordner-Sync), erklärt es das verständlich (lva#371).
 */

type Pending =
  | { kind: "migrate" }
  | { kind: "rollback" }
  | { kind: "restore"; backup: LibraryBackup }
  | { kind: "delete"; backup: LibraryBackup };

type Notice =
  | { kind: "success"; text: string }
  | { kind: "error"; text: string }
  | { kind: "noSync" };

/** Fehlertexte des Kerns, wenn das Laufwerk nicht dauerhaft sichern kann. */
const isNoSync = (error: string) =>
  error.includes("nicht dauerhaft sichern") ||
  error.includes("ließ sich nicht dauerhaft sichern");

/** Phasen, in denen eine Neuordnung offen ist (Spec 4.3). */
const OPEN_PHASES = new Set(["geplant", "gesichert", "rueckfall"]);

export const LibraryLayoutSettings: React.FC<{ grouped?: boolean }> = ({
  grouped = true,
}) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || "de";
  const [status, setStatus] = useState<LibraryLayoutStatus | null>(null);
  const [backups, setBackups] = useState<LibraryBackup[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [problems, setProblems] = useState<LibraryLayoutProblem[]>([]);

  const load = useCallback(async () => {
    const [s, b] = await Promise.all([
      commands.libraryLayoutStatus(),
      commands.libraryBackupList(),
    ]);
    if (s.status === "ok") {
      setStatus(s.data);
      setLoadError(null);
    } else {
      setLoadError(s.error);
    }
    if (b.status === "ok") setBackups(b.data);
  }, []);

  useEffect(() => {
    void load();
    const unlisten = listen("library-layout-changed", () => void load());
    return () => {
      void unlisten.then((off) => off());
    };
  }, [load]);

  const date = (ms: number) =>
    new Date(ms).toLocaleString(locale, {
      dateStyle: "medium",
      timeStyle: "short",
    });
  const size = (bytes: number) => formatBytes(bytes, locale);

  const fail = (error: string) =>
    setNotice(
      isNoSync(error)
        ? { kind: "noSync" }
        : {
            kind: "error",
            text: t("settings.app.libraryLayout.failed", { error }),
          },
    );

  const run = async () => {
    if (!pending) return;
    const job = pending;
    setBusy(true);
    setNotice(null);
    try {
      if (job.kind === "migrate" || job.kind === "rollback") {
        const r =
          job.kind === "migrate"
            ? await commands.libraryLayoutMigrate()
            : await commands.libraryLayoutRollback();
        if (r.status === "error") {
          fail(r.error);
        } else {
          setProblems(r.data.probleme);
          if (!r.data.angehalten) {
            setNotice({
              kind: "success",
              text: t(
                job.kind === "migrate"
                  ? "settings.app.libraryLayout.done"
                  : "settings.app.libraryLayout.rolledBack",
                { count: r.data.erledigt },
              ),
            });
          }
        }
      } else if (job.kind === "restore") {
        const r = await commands.libraryBackupRestore(job.backup.name);
        if (r.status === "error") {
          fail(r.error);
        } else {
          const restored = t("settings.app.libraryLayout.restored", {
            count: r.data.dateien,
          });
          const trash = r.data.papierkorb
            ? " " +
              t("settings.app.libraryLayout.restoredTrash", {
                path: r.data.papierkorb,
              })
            : "";
          setNotice({ kind: "success", text: restored + trash });
        }
      } else {
        const r = await commands.libraryBackupDelete(job.backup.name);
        if (r.status === "error") fail(r.error);
        else
          setNotice({
            kind: "success",
            text: t("settings.app.libraryLayout.deleted"),
          });
      }
    } finally {
      setBusy(false);
      setPending(null);
      void load();
    }
  };

  const openPhase = status?.phase != null && OPEN_PHASES.has(status.phase);
  const shownProblems =
    problems.length > 0 ? problems : (status?.probleme ?? []);
  const stopped = shownProblems.length > 0;
  const needed = status?.platz_benoetigt ?? 0;
  const free = status?.platz_frei ?? null;
  const noSpace = free != null && free < needed;

  const statusText = () => {
    if (!status) return t("settings.app.libraryLayout.loading");
    if (status.zu_verschieben > 0)
      return t("settings.app.libraryLayout.pending", {
        count: status.zu_verschieben,
      });
    if (status.layout >= 2) return t("settings.app.libraryLayout.ordered");
    return t("settings.app.libraryLayout.nothing");
  };

  return (
    <>
      <SettingContainer
        title={t("settings.app.libraryLayout.title")}
        description={t("settings.app.libraryLayout.description")}
        descriptionMode="tooltip"
        grouped={grouped}
        layout="stacked"
      >
        <div className="space-y-3" data-testid="library-layout">
          <p className="text-sm" data-testid="library-layout-status">
            {loadError
              ? t("settings.app.libraryLayout.failed", { error: loadError })
              : statusText()}
          </p>
          {stopped && (
            <Alert variant="warning" contained>
              <p className="font-semibold">
                {t("settings.app.libraryLayout.stoppedTitle")}
              </p>
              <p>{t("settings.app.libraryLayout.stoppedText")}</p>
              <ul
                className="mt-2 list-disc pl-5"
                data-testid="library-layout-problems"
              >
                {shownProblems.map((p, i) => (
                  <li key={`${p.seite}-${i}`}>
                    {p.meldung}
                    {p.pfade.length > 0 && (
                      <span className="block break-all opacity-80">
                        {t("settings.app.libraryLayout.paths")}:{" "}
                        {p.pfade.join(", ")}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </Alert>
          )}
          {notice?.kind === "success" && (
            <Alert variant="success" contained>
              {notice.text}
            </Alert>
          )}
          {notice?.kind === "error" && (
            <Alert variant="error" contained>
              {notice.text}
            </Alert>
          )}
          {notice?.kind === "noSync" && (
            <Alert variant="warning" contained>
              <p className="font-semibold" data-testid="library-layout-nosync">
                {t("settings.app.libraryLayout.noSyncTitle")}
              </p>
              <p>{t("settings.app.libraryLayout.noSyncText")}</p>
              <Button
                variant="secondary"
                size="sm"
                className="mt-2"
                onClick={() => void commands.openAppDataDir()}
              >
                {t("settings.app.libraryLayout.openDataDir")}
              </Button>
            </Alert>
          )}
          <div className="flex flex-wrap gap-2">
            {status && (status.zu_verschieben > 0 || openPhase) && (
              <Button
                onClick={() => setPending({ kind: "migrate" })}
                disabled={busy}
                data-testid="library-layout-open"
              >
                {openPhase
                  ? t("settings.app.libraryLayout.continue")
                  : t("settings.app.libraryLayout.open")}
              </Button>
            )}
            {status && status.layout >= 2 && (
              <Button
                variant="secondary"
                onClick={() => setPending({ kind: "rollback" })}
                disabled={busy}
                data-testid="library-layout-rollback"
              >
                {t("settings.app.libraryLayout.rollback")}
              </Button>
            )}
          </div>
          {status && status.hinweise.length > 0 && (
            <details className="text-sm">
              <summary className="cursor-pointer">
                {t("settings.app.libraryLayout.hints")} (
                {status.hinweise.length})
              </summary>
              <ul className="mt-1 list-disc pl-5">
                {status.hinweise.map((h, i) => (
                  <li key={i}>{h}</li>
                ))}
              </ul>
            </details>
          )}
        </div>
      </SettingContainer>

      <SettingContainer
        title={t("settings.app.libraryLayout.backupsTitle")}
        description={t("settings.app.libraryLayout.backupsDescription")}
        descriptionMode="tooltip"
        grouped={grouped}
        layout="stacked"
      >
        {backups.length === 0 ? (
          <p className="text-sm" data-testid="library-backups-empty">
            {t("settings.app.libraryLayout.noBackups")}
          </p>
        ) : (
          <ul className="space-y-2" data-testid="library-backups">
            {backups.map((b) => (
              <li
                key={b.name}
                className="flex flex-wrap items-center gap-2 text-sm"
                data-testid="library-backup"
              >
                <span className="min-w-0 flex-1">
                  {t("settings.app.libraryLayout.backupRow", {
                    date: date(b.erstellt_ms),
                    files: b.dateien,
                    size: size(b.bytes),
                  })}
                  {b.abgelaufen && (
                    <span className="ml-2 font-semibold">
                      {t("settings.app.libraryLayout.expired")}
                    </span>
                  )}
                </span>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onClick={() => setPending({ kind: "restore", backup: b })}
                >
                  {t("settings.app.libraryLayout.restore")}
                </Button>
                {b.abgelaufen && (
                  <Button
                    variant="danger-ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() => setPending({ kind: "delete", backup: b })}
                  >
                    {t("settings.app.libraryLayout.delete")}
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </SettingContainer>

      <Dialog
        open={pending != null}
        onOpenChange={(open) => {
          if (!open && !busy) setPending(null);
        }}
        dismissible={!busy}
        title={dialogTitle(pending, t, date)}
        closeLabel={t("settings.app.libraryLayout.close")}
        className="max-w-lg"
        footer={
          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setPending(null)}
            >
              {t("settings.app.libraryLayout.later")}
            </Button>
            <Button
              variant={pending?.kind === "delete" ? "danger" : "primary"}
              disabled={busy || (pending?.kind === "migrate" && noSpace)}
              onClick={() => void run()}
              data-testid="library-layout-apply"
            >
              {busy
                ? t("settings.app.libraryLayout.running")
                : applyLabel(pending, t)}
            </Button>
          </div>
        }
      >
        <div className="space-y-3 text-sm">
          {pending?.kind === "migrate" && (
            <>
              <p>
                {t("settings.app.libraryLayout.dialogText", {
                  count: status?.zu_verschieben ?? 0,
                })}
              </p>
              <ul className="list-disc pl-5">
                <li>
                  {t("settings.app.libraryLayout.dialogBackup", {
                    needed: size(needed),
                  })}
                </li>
                <li>{t("settings.app.libraryLayout.dialogKeep")}</li>
                <li>{t("settings.app.libraryLayout.dialogStop")}</li>
              </ul>
              {noSpace && (
                <Alert variant="error" contained>
                  {t("settings.app.libraryLayout.dialogNoSpace", {
                    needed: size(needed),
                    free: size(free ?? 0),
                  })}
                </Alert>
              )}
            </>
          )}
          {pending?.kind === "rollback" && (
            <p>{t("settings.app.libraryLayout.rollbackText")}</p>
          )}
          {pending?.kind === "restore" && (
            <p>{t("settings.app.libraryLayout.restoreText")}</p>
          )}
          {pending?.kind === "delete" && (
            <p>{t("settings.app.libraryLayout.deleteText")}</p>
          )}
        </div>
      </Dialog>
    </>
  );
};

type T = ReturnType<typeof useTranslation>["t"];

function dialogTitle(
  pending: Pending | null,
  t: T,
  date: (ms: number) => string,
): string {
  switch (pending?.kind) {
    case "rollback":
      return t("settings.app.libraryLayout.rollbackTitle");
    case "restore":
      return t("settings.app.libraryLayout.restoreTitle", {
        date: date(pending.backup.erstellt_ms),
      });
    case "delete":
      return t("settings.app.libraryLayout.deleteTitle", {
        date: date(pending.backup.erstellt_ms),
      });
    default:
      return t("settings.app.libraryLayout.dialogTitle");
  }
}

function applyLabel(pending: Pending | null, t: T): string {
  switch (pending?.kind) {
    case "rollback":
      return t("settings.app.libraryLayout.rollbackApply");
    case "restore":
      return t("settings.app.libraryLayout.restoreApply");
    case "delete":
      return t("settings.app.libraryLayout.deleteApply");
    default:
      return t("settings.app.libraryLayout.now");
  }
}
