import React, { useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ProgressBar } from "../shared";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { MarkdownContent } from "../whats-new/MarkdownContent";
import { useSettings } from "../../hooks/useSettings";
import { commands, type LocalUpdate } from "../../bindings";
import { useFirmenrichtlinie } from "../../hooks/useFirmenrichtlinie";
import { RELEASE_SEITE } from "../../lib/firmenrichtlinie";
import { useUpdatePolling } from "./useUpdatePolling";
import { beginUpdateInstallation } from "./updateInstallation";
import { promptDue } from "./updatePolicy";

/** Wunsch aus dem Profilmenü ▸ Hilfe: jetzt nach Updates suchen. */
export const UPDATE_CHECK_EVENT = "lv-update-check";

interface UpdateCheckerProps {
  className?: string;
  /** Laufende Version; ein Klick darauf sucht nach Updates. */
  version: string;
  /** Statusleiste (ADR-0027 E7): Version nur zeigen, wenn ein Update
      bereitsteht oder gerade geprüft/installiert wird; sonst steht sie in
      „Über“. Gesucht wird weiter beim Start und auf Wunsch. */
  nurBeiUpdate?: boolean;
}

/** Semver-Vergleich der numerischen Teile: > 0 wenn a neuer als b. */
export const compareVersions = (a: string, b: string): number => {
  const parse = (v: string) =>
    v
      .replace(/^v/, "")
      .split(/[.-]/)
      .map((part) => Number.parseInt(part, 10))
      .map((n) => (Number.isFinite(n) ? n : 0));
  const pa = parse(a);
  const pb = parse(b);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
};

/**
 * Version in der Fußleiste: ein Klick sucht nach Updates (online und im lokalen
 * Ordner). Findet die Suche beim Start eine neuere Version, fragt ein Fenster,
 * ob sie installiert werden soll — mit den Release-Notes (online aus dem
 * Release, lokal aus `<installer>.notes.md`). Ohne Update steht nur die Version da.
 */
const UpdateChecker: React.FC<UpdateCheckerProps> = ({
  className = "",
  version,
  nurBeiUpdate = false,
}) => {
  const { t } = useTranslation();
  // Update checking state
  const [isChecking, setIsChecking] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  /** Abschnitt der Online-Installation für die Fußleiste (lva#869 H3). */
  const [installPhase, setInstallPhase] = useState<
    "laden" | "warten" | "installieren"
  >("laden");
  const [showUpToDate, setShowUpToDate] = useState(false);
  const [showPortableUpdateDialog, setShowPortableUpdateDialog] =
    useState(false);
  // Firmenprofil (lva#882): kein Selbstupdate. Die neue Version wird nur
  // angezeigt; installiert wird ueber die Release-Seite bzw. durch die IT.
  const selbstupdateGesperrt =
    useFirmenrichtlinie().gesperrt("kein_selbstupdate");
  // A newer installer in the configured local folder (see
  // LocalUpdateDirectory). Independent of the GitHub check: no network, no
  // signature, works for acceptance builds that never get published.
  const [localUpdateFound, setLocalUpdateFound] = useState<LocalUpdate | null>(
    null,
  );
  // Version des GitHub-Updates, damit lokal und online verglichen werden
  // koennen: es wird immer das NEUERE angeboten. Vorher gewann der lokale
  // Ordner unbesehen — 0.18.17 lokal verdeckte 0.19.0 auf GitHub (17.09.2026).
  const [githubVersion, setGithubVersion] = useState<string | null>(null);
  /** Release-Notes des Online-Updates (Markdown). */
  const [githubNotes, setGithubNotes] = useState<string | null>(null);
  /** Fenster „neue Version installieren?“. */
  const [dialogOpen, setDialogOpen] = useState(false);
  /** Automatische Nachfrage einmal je Version und Programmlauf. */
  const promptedRef = useRef(new Set<string>());
  const [gateBelegt, setGateBelegt] = useState(true);
  const [verlauf, setVerlauf] = useState<Awaited<ReturnType<typeof commands.updateVerlauf>> | null>(null);
  const [aktualisiertAuf, setAktualisiertAuf] = useState<string | null>(null);
  const [aufschubRevision, setAufschubRevision] = useState(0);
  const installAbort = useRef(new AbortController());
  const checkingRef = useRef(false);
  const lastCheckRef = useRef(0);
  const localUpdate =
    localUpdateFound &&
    (githubVersion === null ||
      compareVersions(localUpdateFound.version, githubVersion) >= 0)
      ? localUpdateFound
      : null;

  const { settings, isLoading } = useSettings();
  const settingsLoaded = !isLoading && settings !== null;
  const updateChecksEnabled = settings?.update_checks_enabled ?? false;

  const upToDateTimeoutRef = useRef<ReturnType<typeof setTimeout>>();
  const isManualCheckRef = useRef(false);
  const downloadedBytesRef = useRef(0);
  const contentLengthRef = useRef(0);

  useEffect(() => {
    installAbort.current = new AbortController();
    let alive = true;
    let revision = 0;
    const unlisten = listen<{ belegt: boolean }>("lv-update-gate", ({ payload }) => {
      revision++;
      if (alive) setGateBelegt(payload.belegt);
    });
    void unlisten.then(async () => {
      const seen = revision;
      const status = await commands.updateGateStatus().catch(() => null);
      if (alive && seen === revision) setGateBelegt(status?.status !== "ok" || status.data.belegt);
    });
    void commands.updateVerlauf().then((result) => {
      if (!alive) return;
      setVerlauf(result);
      if (result.status === "ok" && result.data.aktualisiert_auf) {
        setAktualisiertAuf(result.data.aktualisiert_auf);
        window.dispatchEvent(new CustomEvent("lv-update-installed", { detail: { version: result.data.aktualisiert_auf } }));
      }
    }).catch(() => setVerlauf(null));
    return () => {
      alive = false;
      installAbort.current.abort();
      void unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    // Wait for settings to load before doing anything
    if (!settingsLoaded) return;

    if (!updateChecksEnabled) {
      if (upToDateTimeoutRef.current) {
        clearTimeout(upToDateTimeoutRef.current);
      }
      setIsChecking(false);
      setUpdateAvailable(false);
      setShowUpToDate(false);
      // The local folder needs no network and no GitHub, so it is still
      // consulted when online checks are off.
      void checkLocalUpdate();
      const localUnlisten = listen("check-for-updates", () => {
        void versionClickRef.current();
      });
      return () => {
        localUnlisten.then((fn) => fn());
      };
    }

    // Listen for update check events
    const updateUnlisten = listen("check-for-updates", () => {
      handleManualUpdateCheck();
    });

    return () => {
      if (upToDateTimeoutRef.current) {
        clearTimeout(upToDateTimeoutRef.current);
      }
      updateUnlisten.then((fn) => fn());
    };
  }, [settingsLoaded, updateChecksEnabled]);

  const checkLocalUpdate = async (): Promise<boolean> => {
    try {
      const result = await commands.localUpdateCheck();
      const found = result.status === "ok" ? result.data : null;
      setLocalUpdateFound(found);
      return found !== null;
    } catch {
      console.error("update_local_check_failed");
      setLocalUpdateFound(null);
      return false;
    }
  };

  // Update checking functions
  const checkForUpdates = async () => {
    if (!updateChecksEnabled || checkingRef.current || isInstalling) return;
    checkingRef.current = true;
    lastCheckRef.current = Date.now();

    try {
      setIsChecking(true);
      // GitHub and the local folder are checked side by side; a failing
      // GitHub check (offline, unsigned release) must not hide a local one.
      const [update, hasLocal] = await Promise.all([
        commands
          .updateProxy()
          .catch(() => null)
          .then((proxy) => check(proxy ? { proxy } : undefined))
          .catch(() => {
            console.error("Failed to check for updates");
            return null;
          }),
        checkLocalUpdate(),
      ]);

      if (update) {
        setUpdateAvailable(true);
        setGithubVersion(update.version);
        setGithubNotes(update.body ?? null);
        setShowUpToDate(false);
        if (isManualCheckRef.current) {
          setDialogOpen(true);
        }
        await update.close();
      } else {
        setUpdateAvailable(false);
        setGithubVersion(null);

        if (isManualCheckRef.current && hasLocal) setDialogOpen(true);

        if (isManualCheckRef.current && !hasLocal) {
          setShowUpToDate(true);
          if (upToDateTimeoutRef.current) {
            clearTimeout(upToDateTimeoutRef.current);
          }
          upToDateTimeoutRef.current = setTimeout(() => {
            setShowUpToDate(false);
          }, 3000);
        }
      }
    } catch {
      console.error("Failed to check for updates");
    } finally {
      checkingRef.current = false;
      setIsChecking(false);
      isManualCheckRef.current = false;
    }
  };

  useUpdatePolling(settingsLoaded && updateChecksEnabled, checkForUpdates, lastCheckRef);

  const handleManualUpdateCheck = () => {
    if (!updateChecksEnabled) return;
    isManualCheckRef.current = true;
    checkForUpdates();
  };

  // „Nach Updates suchen“ im Profilmenü wirkt wie ein Klick auf die Version.
  const versionClickRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    const onWunsch = () => void versionClickRef.current();
    window.addEventListener(UPDATE_CHECK_EVENT, onWunsch);
    return () => window.removeEventListener(UPDATE_CHECK_EVENT, onWunsch);
  }, []);

  /** Klick auf die Version: vorhandenes Update zeigen, sonst suchen. */
  const onVersionClick = async () => {
    if (isChecking || isInstalling) return;
    if (localUpdate || updateAvailable) {
      setDialogOpen(true);
      return;
    }
    if (updateChecksEnabled) {
      handleManualUpdateCheck();
      return;
    }
    // Online-Suche aus: nur der lokale Ordner.
    const found = await checkLocalUpdate();
    if (found) {
      setDialogOpen(true);
    } else {
      setShowUpToDate(true);
      if (upToDateTimeoutRef.current) clearTimeout(upToDateTimeoutRef.current);
      upToDateTimeoutRef.current = setTimeout(
        () => setShowUpToDate(false),
        3000,
      );
    }
  };

  const installLocalUpdate = async () => {
    if (!localUpdate || selbstupdateGesperrt) return;
    let ownsGate = false;
    try {
      setIsInstalling(true);
      await beginUpdateInstallation(localUpdate.version, installAbort.current.signal);
      ownsGate = true;
      if (installAbort.current.signal.aborted) throw new Error("update_abgebrochen");
      const result = await commands.localUpdateInstall(localUpdate.path);
      if (result.status === "error") {
        console.error("update_local_install_failed");
      }
      // Der lokale Weg zeigt nur den Ordner; er beendet die App nicht.
    } catch {
      console.error("update_local_install_failed");
    } finally {
      if (ownsGate) await commands.updateInstallAbort();
      setIsInstalling(false);
    }
  };

  const installUpdate = async (confirmedVersion: string) => {
    if (!updateChecksEnabled) return;

    const portable = await commands.isPortable();
    if (portable || selbstupdateGesperrt) {
      setShowPortableUpdateDialog(true);
      return;
    }

    let ownsGate = false;
    let candidate: Awaited<ReturnType<typeof check>> = null;
    try {
      setIsInstalling(true);
      setInstallPhase("laden");
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
      const proxy = await commands.updateProxy().catch(() => null);
      const update = await check(proxy ? { proxy } : undefined);
      candidate = update;

      if (!update) {
        console.log("No update available during install attempt");
        return;
      }
      // Nur die Version installieren, die im Fenster bestätigt wurde. Ist inzwischen
      // eine andere erschienen, zuerst deren Änderungen zeigen (lva#869).
      if (update.version !== confirmedVersion) {
        setGithubVersion(update.version);
        setGithubNotes(update.body ?? null);
        setDialogOpen(true);
        return;
      }

      await update.download((event) => {
        switch (event.event) {
          case "Started":
            downloadedBytesRef.current = 0;
            contentLengthRef.current = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloadedBytesRef.current += event.data.chunkLength;
            if (contentLengthRef.current <= 0) {
              // Neu zeichnen, damit „Wird heruntergeladen …“ erscheint.
              setDownloadProgress((p) => (p === 0 ? 0.5 : p));
              break;
            }
            const progress =
              contentLengthRef.current > 0
                ? Math.round(
                    (downloadedBytesRef.current / contentLengthRef.current) *
                      100,
                  )
                : 0;
            setDownloadProgress(Math.min(progress, 100));
            break;
        }
      });
      // Herunterladen darf während einer Aufnahme laufen, Installieren beendet
      // die App: erst nach dem Ende der Aufnahme (lva#869).
      setInstallPhase("warten");
      await beginUpdateInstallation(update.version, installAbort.current.signal);
      ownsGate = true;
      setInstallPhase("installieren");
      if (installAbort.current.signal.aborted) throw new Error("update_abgebrochen");
      const committed = await commands.updateInstallCommit(update.version);
      if (committed.status === "error") throw new Error(committed.error.code);
      await update.install();
      await relaunch();
    } catch {
      if (ownsGate) await commands.updateInstallAbort();
      console.error("Failed to install update");
    } finally {
      await candidate?.close().catch(() => console.error("update_resource_failed"));
      setIsInstalling(false);
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
    }
  };

  // Status neben der Version nur, solange etwas passiert.
  const getUpdateStatusText = (): string | null => {
    if (isInstalling && localUpdate) return t("footer.localInstalling");
    if (isInstalling) {
      if (installPhase === "installieren") return t("footer.installing");
      if (installPhase === "warten")
        return gateBelegt
          ? t("footer.waitingForRecording")
          : t("footer.installing");
      // Ohne Content-Length gibt es keinen Prozentwert: dann ohne Zahl.
      if (downloadedBytesRef.current > 0 && contentLengthRef.current <= 0)
        return t("footer.downloadingNoProgress");
      return downloadProgress > 0 && downloadProgress < 100
        ? t("footer.downloading", {
            progress: downloadProgress.toString().padStart(3),
          })
        : t("footer.preparing");
    }
    if (isChecking) return t("footer.checkingUpdates");
    if (showUpToDate) return t("footer.upToDate");
    return null;
  };

  const offered = localUpdate
    ? { version: localUpdate.version, notes: localUpdate.notes }
    : updateAvailable && githubVersion
      ? { version: githubVersion, notes: githubNotes }
      : null;
  const offeredVersion = offered?.version ?? null;
  /** Patch ≥ 1 ist per Definition ein Hotfix (lva#869). */
  const istHotfix =
    !!offeredVersion &&
    /^\d+\.\d+\.[1-9]\d*$/.test(offeredVersion.replace(/^v/, ""));

  // „Aktualisiert auf X“ einmal nach dem Neustart melden (lva#869 H3).
  useEffect(() => {
    if (!aktualisiertAuf) return;
    toast.success(t("footer.updatedTo.title", { version: aktualisiertAuf }), {
      id: "lv-update-installed",
      description: t("footer.updatedTo.description"),
    });
  }, [aktualisiertAuf, t]);
  useEffect(() => {
    if (!offeredVersion || gateBelegt || isChecking || isInstalling || dialogOpen || verlauf?.status !== "ok") return;
    const spaeter = verlauf.data.spaeter;
    if (spaeter?.version === offeredVersion && spaeter.bis > Date.now()) {
      // Nach einem Neustart weckt kein Online-Polling den lokalen Aufschub.
      const timer = setTimeout(() => {
        setAufschubRevision((revision) => revision + 1);
      }, Math.min(spaeter.bis - Date.now(), 2_147_483_647));
      return () => clearTimeout(timer);
    }
    if (!promptDue(offeredVersion, gateBelegt, promptedRef.current, spaeter, Date.now())) return;
    promptedRef.current.add(offeredVersion);
    setDialogOpen(true);
  }, [offeredVersion, gateBelegt, isChecking, isInstalling, dialogOpen, verlauf, aufschubRevision]);

  const spaeter = () => {
    setDialogOpen(false);
    if (!offeredVersion) return;
    const aufschub = { version: offeredVersion, bis: Date.now() + 4 * 60 * 60_000 };
    setVerlauf((alt) => alt?.status === "ok" ? { status: "ok", data: { ...alt.data, spaeter: aufschub } } : alt);
    void commands.updateSpaeter(aufschub.version, aufschub.bis).then((result) => {
      if (result.status === "error") console.error("update_speichern_failed");
    }).catch(() => console.error("update_speichern_failed"));
  };
  versionClickRef.current = onVersionClick;
  const status = getUpdateStatusText();
  const versionSichtbar =
    !!version && (!nurBeiUpdate || !!offered || isChecking || isInstalling);
  const versionTitle = offered
    ? t("footer.updateAvailableTitle", { version: offered.version })
    : t("footer.checkForUpdates");

  return (
    <>
      {showPortableUpdateDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-background border border-mid-gray/20 rounded-lg p-6 max-w-md w-full mx-4 space-y-4">
            <h2 className="text-base font-semibold">
              {t("footer.portableUpdateTitle")}
            </h2>
            <p className="text-sm text-text/70">
              {selbstupdateGesperrt
                ? t("firmenrichtlinie.gesperrt.kein_selbstupdate")
                : t("footer.portableUpdateMessage")}
            </p>
            <div className="flex gap-2 justify-end">
              <button
                className="px-3 py-1.5 text-sm rounded border border-mid-gray/20 hover:bg-mid-gray/10 transition-colors"
                onClick={() => setShowPortableUpdateDialog(false)}
              >
                {t("common.close")}
              </button>
              <button
                className="px-3 py-1.5 text-sm rounded bg-logo-primary text-on-accent hover:bg-logo-primary/80 transition-colors"
                onClick={() => {
                  openUrl(
                    "https://github.com/MrP42/local-voice-ai/releases/latest",
                  );
                  setShowPortableUpdateDialog(false);
                }}
              >
                {t("footer.portableUpdateButton")}
              </button>
            </div>
          </div>
        </div>
      )}
      {offered && (
        <Dialog
          open={dialogOpen}
          onOpenChange={(open) => { if (open) setDialogOpen(true); else spaeter(); }}
          title={t(
            istHotfix
              ? "footer.updateDialog.titleHotfix"
              : "footer.updateDialog.title",
            { version: offered.version },
          )}
          description={t("footer.updateDialog.description", {
            current: version,
            version: offered.version,
          })}
          closeLabel={t("footer.updateDialog.later")}
          footer={
            <>
              <Button variant="secondary" onClick={spaeter}>
                {t("footer.updateDialog.later")}
              </Button>
              {selbstupdateGesperrt ? (
                <Button
                  onClick={() => {
                    setDialogOpen(false);
                    void openUrl(RELEASE_SEITE);
                  }}
                  data-testid="update-release-seite"
                >
                  {t("footer.portableUpdateButton")}
                </Button>
              ) : (
                <Button
                  onClick={() => {
                    setDialogOpen(false);
                    void (localUpdate ? installLocalUpdate() : installUpdate(offered.version));
                  }}
                  data-testid="update-install"
                >
                  {t("footer.updateDialog.install")}
                </Button>
              )}
            </>
          }
        >
          {selbstupdateGesperrt && (
            <p className="mb-3 text-sm" data-testid="update-richtlinie">
              {t("firmenrichtlinie.gesperrt.kein_selbstupdate")}
            </p>
          )}
          {gateBelegt && !selbstupdateGesperrt && (
            <p
              className="mb-3 rounded-md bg-mid-gray/10 px-3 py-2 text-sm text-text/80"
              data-testid="update-wartet"
            >
              {t("footer.updateDialog.recordingActive")}
            </p>
          )}
          <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-text/60">
            {t("footer.updateDialog.changes")}
          </h3>
          <div
            className="max-h-[50vh] overflow-y-auto text-sm"
            data-testid="update-notes"
          >
            {offered.notes ? (
              <MarkdownContent markdown={offered.notes} />
            ) : (
              <p className="text-text/60">{t("footer.updateDialog.noNotes")}</p>
            )}
          </div>
        </Dialog>
      )}
      <div className={`flex items-center gap-2 ${className}`} data-aktualisiert-auf={aktualisiertAuf ?? undefined}>
        {status && (
          <span
            className="text-text/60 tabular-nums"
            role="status"
            aria-live="polite"
          >
            {status}
          </span>
        )}
        {isInstalling &&
          installPhase === "laden" &&
          contentLengthRef.current > 0 &&
          downloadProgress > 0 &&
          downloadProgress < 100 && (
          <ProgressBar
            progress={[
              {
                id: "update",
                percentage: downloadProgress,
              },
            ]}
            size="large"
          />
        )}
        {versionSichtbar && (
          <button
            type="button"
            onClick={() => void onVersionClick()}
            disabled={isChecking || isInstalling}
            title={versionTitle}
            aria-label={`v${version}, ${versionTitle}`}
            className={`lv-status-btn tabular-nums transition-colors disabled:opacity-50 ${
              offered
                ? "text-logo-primary font-medium hover:text-logo-primary/80"
                : "text-text/60 hover:text-text/80"
            }`}
            data-testid="footer-version"
            data-angebot={offered ? "" : undefined}
          >
            {`v${version}`}
            {offered && (
              <span className="ms-1" aria-hidden="true">
                ↑
              </span>
            )}
          </button>
        )}
      </div>
    </>
  );
};

export default UpdateChecker;
