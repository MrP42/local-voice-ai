import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  commands,
  type LibraryBackup,
  type LibraryLayoutProblem,
  type LibraryLayoutStatus,
  type LibrarySyncCheck,
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
 * Zurückspielen und – nach 30 Tagen – Löschen. Vor dem Zurückspielen wird das
 * Laufwerk geprüft (tatsächliches Sync-Ergebnis, lva#371): Kann es nicht sicher
 * schreiben, erklärt der Dialog das und bietet Auswege an – Sicherung in einen
 * neuen Ordner exportieren, Datenordner verschieben oder ausdrücklich ohne Schutz
 * bei Absturz zurückspielen.
 */

type Pending =
  | { kind: "migrate" }
  | { kind: "rollback" }
  | { kind: "restore"; backup: LibraryBackup }
  | { kind: "delete"; backup: LibraryBackup };

type Notice =
  | { kind: "success"; text: string; warn?: string }
  | { kind: "error"; text: string }
  | { kind: "noSync"; backup?: LibraryBackup };

/**
 * Fehlertext des Kerns, wenn das Dateisystem Sync nicht kennt (nicht: ein
 * anderer Fehler beim Sichern, der bleibt eine gewöhnliche Fehlermeldung).
 */
const isNoSync = (error: string) =>
  error.includes("lässt sich auf diesem Dateisystem nicht dauerhaft sichern");

/** Phasen, in denen eine Neuordnung offen ist (Spec 4.3). */
const OPEN_PHASES = new Set(["geplant", "gesichert", "rueckfall"]);
/** Touch-Ziel ≥ 44 px auf schmalen und groben Zeigegeräten. */
const TOUCH = "max-sm:min-h-[44px] pointer-coarse:min-h-[44px]";

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
  // Prüfung des Laufwerks für den offenen Zurückspielen-Dialog (lva#371).
  const [syncCheck, setSyncCheck] = useState<LibrarySyncCheck | null>(null);
  const [bestEffort, setBestEffort] = useState(false);
  const restoreName = pending?.kind === "restore" ? pending.backup.name : null;

  useEffect(() => {
    setSyncCheck(null);
    setBestEffort(false);
    if (!restoreName) return;
    let current = true;
    void commands.libraryBackupSyncCheck().then((r) => {
      if (!current) return;
      setSyncCheck(
        r.status === "ok" ? r.data : { ergebnis: "fehler", meldung: r.error },
      );
    });
    return () => {
      current = false;
    };
  }, [restoreName]);

  const noSyncDrive = syncCheck?.ergebnis === "nicht_unterstuetzt";
  const restoreBlocked =
    pending?.kind === "restore" &&
    (syncCheck == null ||
      syncCheck.ergebnis === "fehler" ||
      (noSyncDrive && !bestEffort));

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

  const fail = (error: string, backup?: LibraryBackup) =>
    setNotice(
      isNoSync(error)
        ? { kind: "noSync", backup }
        : {
            kind: "error",
            text: t("settings.app.libraryLayout.failed", { error }),
          },
    );

  const running = useRef(false);
  const run = async () => {
    if (!pending || running.current) return;
    running.current = true;
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
        const r = bestEffort
          ? await commands.libraryBackupRestoreBestEffort(job.backup.name)
          : await commands.libraryBackupRestore(job.backup.name);
        if (r.status === "error") {
          fail(r.error, job.backup);
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
      running.current = false;
      setBusy(false);
      setPending(null);
      void load();
    }
  };

  // Ausweg (lva#371): die Sicherung in einen neuen Ordner kopieren und prüfen.
  const exportBackup = async (backup: LibraryBackup) => {
    if (running.current) return;
    const target = await openDialog({
      directory: true,
      multiple: false,
      title: t("settings.app.libraryLayout.exportTitle"),
    });
    if (typeof target !== "string") return;
    running.current = true;
    setBusy(true);
    try {
      const r = await commands.libraryBackupExport(backup.name, target);
      if (r.status === "error") {
        setNotice({
          kind: "error",
          text: t("settings.app.libraryLayout.failed", { error: r.error }),
        });
      } else {
        setNotice({
          kind: "success",
          text: t("settings.app.libraryLayout.exported", {
            count: r.data.dateien,
            path: r.data.ordner,
          }),
          warn: r.data.dauerhaft
            ? undefined
            : t("settings.app.libraryLayout.exportedNotDurable"),
        });
      }
    } finally {
      running.current = false;
      setBusy(false);
      setPending(null);
    }
  };

  const openPhase = status?.phase != null && OPEN_PHASES.has(status.phase);
  // Ein angehaltener Rückfall wird per Rückfall fortgesetzt, nie per Neuordnung.
  const rollbackOpen = status?.phase === "rueckfall";
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
            <Alert variant={notice.warn ? "warning" : "success"} contained>
              <p data-testid="library-layout-success">{notice.text}</p>
              {notice.warn && (
                <p className="mt-1 font-semibold">{notice.warn}</p>
              )}
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
              <p>{t("settings.app.libraryLayout.noSyncWays")}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {notice.backup && (
                  <Button
                    variant="secondary"
                    size="sm"
                    className={TOUCH}
                    disabled={busy}
                    onClick={() =>
                      notice.backup && void exportBackup(notice.backup)
                    }
                  >
                    {t("settings.app.libraryLayout.exportBackup")}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  size="sm"
                  className={TOUCH}
                  onClick={() => void commands.openAppDataDir()}
                >
                  {t("settings.app.libraryLayout.openDataDir")}
                </Button>
              </div>
            </Alert>
          )}
          <div className="flex flex-wrap gap-2">
            {status && (status.zu_verschieben > 0 || openPhase) && (
              <Button
                className={TOUCH}
                onClick={() =>
                  setPending({ kind: rollbackOpen ? "rollback" : "migrate" })
                }
                disabled={busy}
                data-testid="library-layout-open"
              >
                {rollbackOpen
                  ? t("settings.app.libraryLayout.continueRollback")
                  : openPhase
                    ? t("settings.app.libraryLayout.continue")
                    : t("settings.app.libraryLayout.open")}
              </Button>
            )}
            {status && !rollbackOpen && (status.layout >= 2 || openPhase) && (
              <Button
                variant="secondary"
                className={TOUCH}
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
                  className={TOUCH}
                  disabled={busy}
                  onClick={() => setPending({ kind: "restore", backup: b })}
                >
                  {t("settings.app.libraryLayout.restore")}
                </Button>
                {b.abgelaufen && (
                  <Button
                    variant="danger-ghost"
                    size="sm"
                    className={TOUCH}
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
              disabled={
                busy ||
                (pending?.kind === "migrate" && noSpace) ||
                restoreBlocked
              }
              onClick={() => void run()}
              data-testid="library-layout-apply"
            >
              {busy
                ? t("settings.app.libraryLayout.running")
                : pending?.kind === "restore" && noSyncDrive && bestEffort
                  ? t("settings.app.libraryLayout.bestEffortApply")
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
            <>
              <p>{t("settings.app.libraryLayout.restoreText")}</p>
              {syncCheck == null && (
                <p data-testid="library-restore-checking">
                  {t("settings.app.libraryLayout.checking")}
                </p>
              )}
              {syncCheck?.ergebnis === "fehler" && (
                <Alert variant="error" contained>
                  {t("settings.app.libraryLayout.checkFailed", {
                    error: syncCheck.meldung ?? "",
                  })}
                </Alert>
              )}
              {noSyncDrive && (
                <Alert variant="warning" contained>
                  <p
                    className="font-semibold"
                    data-testid="library-restore-nosync"
                  >
                    {t("settings.app.libraryLayout.noSyncTitle")}
                  </p>
                  <p>{t("settings.app.libraryLayout.noSyncWays")}</p>
                  <div className="mt-2 flex flex-wrap gap-2">
                    <Button
                      variant="secondary"
                      size="sm"
                      className={TOUCH}
                      disabled={busy}
                      onClick={() => void exportBackup(pending.backup)}
                      data-testid="library-restore-export"
                    >
                      {t("settings.app.libraryLayout.exportBackup")}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      className={TOUCH}
                      onClick={() => void commands.openAppDataDir()}
                    >
                      {t("settings.app.libraryLayout.openDataDir")}
                    </Button>
                  </div>
                  <label
                    className={`mt-3 flex items-start gap-2 ${TOUCH}`}
                    data-testid="library-restore-besteffort"
                  >
                    <input
                      type="checkbox"
                      className="mt-1"
                      checked={bestEffort}
                      disabled={busy}
                      onChange={(e) => setBestEffort(e.target.checked)}
                    />
                    <span>
                      <span className="font-semibold">
                        {t("settings.app.libraryLayout.bestEffortLabel")}
                      </span>
                      <span className="block">
                        {t("settings.app.libraryLayout.bestEffortWarning")}
                      </span>
                    </span>
                  </label>
                </Alert>
              )}
            </>
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
