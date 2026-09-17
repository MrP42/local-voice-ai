import React, { useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ProgressBar } from "../shared";
import { useSettings } from "../../hooks/useSettings";
import { commands, type LocalUpdate } from "../../bindings";

interface UpdateCheckerProps {
  className?: string;
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

const UpdateChecker: React.FC<UpdateCheckerProps> = ({ className = "" }) => {
  const { t } = useTranslation();
  // Update checking state
  const [isChecking, setIsChecking] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [showUpToDate, setShowUpToDate] = useState(false);
  const [showPortableUpdateDialog, setShowPortableUpdateDialog] =
    useState(false);
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
        void checkLocalUpdate();
      });
      return () => {
        localUnlisten.then((fn) => fn());
      };
    }

    checkForUpdates();

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
    } catch (error) {
      console.error("Failed to check local update folder:", error);
      setLocalUpdateFound(null);
      return false;
    }
  };

  // Update checking functions
  const checkForUpdates = async () => {
    if (!updateChecksEnabled || isChecking) return;

    try {
      setIsChecking(true);
      // GitHub and the local folder are checked side by side; a failing
      // GitHub check (offline, unsigned release) must not hide a local one.
      const [update, hasLocal] = await Promise.all([
        check().catch((error) => {
          console.error("Failed to check for updates:", error);
          return null;
        }),
        checkLocalUpdate(),
      ]);

      if (update) {
        setUpdateAvailable(true);
        setGithubVersion(update.version);
        setShowUpToDate(false);
      } else {
        setUpdateAvailable(false);
        setGithubVersion(null);

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
    } catch (error) {
      console.error("Failed to check for updates:", error);
    } finally {
      setIsChecking(false);
      isManualCheckRef.current = false;
    }
  };

  const handleManualUpdateCheck = () => {
    if (!updateChecksEnabled) return;
    isManualCheckRef.current = true;
    checkForUpdates();
  };

  const installLocalUpdate = async () => {
    if (!localUpdate) return;
    try {
      setIsInstalling(true);
      const result = await commands.localUpdateInstall(localUpdate.path);
      if (result.status === "error") {
        console.error("Failed to start local installer:", result.error);
        setIsInstalling(false);
      }
      // On success the installer takes over and the app exits.
    } catch (error) {
      console.error("Failed to start local installer:", error);
      setIsInstalling(false);
    }
  };

  const installUpdate = async () => {
    if (!updateChecksEnabled) return;

    const portable = await commands.isPortable();
    if (portable) {
      setShowPortableUpdateDialog(true);
      return;
    }

    try {
      setIsInstalling(true);
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
      const update = await check();

      if (!update) {
        console.log("No update available during install attempt");
        return;
      }

      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            downloadedBytesRef.current = 0;
            contentLengthRef.current = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloadedBytesRef.current += event.data.chunkLength;
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
      await relaunch();
    } catch (error) {
      console.error("Failed to install update:", error);
    } finally {
      setIsInstalling(false);
      setDownloadProgress(0);
      downloadedBytesRef.current = 0;
      contentLengthRef.current = 0;
    }
  };

  // Update status functions
  const getUpdateStatusText = () => {
    if (isInstalling && localUpdate) {
      return t("footer.localInstalling");
    }
    if (localUpdate && !isInstalling) {
      return t("footer.localUpdateAvailable", { version: localUpdate.version });
    }
    if (!updateChecksEnabled) {
      return t("footer.updateCheckingDisabled");
    }
    if (isInstalling) {
      return downloadProgress > 0 && downloadProgress < 100
        ? t("footer.downloading", {
            progress: downloadProgress.toString().padStart(3),
          })
        : downloadProgress === 100
          ? t("footer.installing")
          : t("footer.preparing");
    }
    if (isChecking) return t("footer.checkingUpdates");
    if (showUpToDate) return t("footer.upToDate");
    if (updateAvailable) return t("footer.updateAvailableShort");
    return t("footer.checkForUpdates");
  };

  const getUpdateStatusAction = () => {
    if (localUpdate && !isInstalling) return installLocalUpdate;
    // Online-Suche aus: ein Klick prueft trotzdem den lokalen Ordner.
    if (!updateChecksEnabled) return () => void checkLocalUpdate();
    if (updateAvailable && !isInstalling) return installUpdate;
    if (!isChecking && !isInstalling && !updateAvailable)
      return handleManualUpdateCheck;
    return undefined;
  };

  const isUpdateDisabled = isChecking || isInstalling;
  const isUpdateClickable =
    !isUpdateDisabled &&
    (localUpdate !== null ||
      updateAvailable ||
      !updateChecksEnabled ||
      (!isChecking && !showUpToDate));

  return (
    <>
      {showPortableUpdateDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-background border border-mid-gray/20 rounded-lg p-6 max-w-md w-full mx-4 space-y-4">
            <h2 className="text-base font-semibold">
              {t("footer.portableUpdateTitle")}
            </h2>
            <p className="text-sm text-text/70">
              {t("footer.portableUpdateMessage")}
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
      <div className={`flex items-center gap-3 ${className}`}>
        {isUpdateClickable ? (
          <button
            onClick={getUpdateStatusAction()}
            disabled={isUpdateDisabled}
            className={`transition-colors disabled:opacity-50 tabular-nums ${
              updateAvailable || localUpdate
                ? "text-logo-primary hover:text-logo-primary/80 font-medium"
                : "text-text/60 hover:text-text/80"
            }`}
          >
            {getUpdateStatusText()}
          </button>
        ) : (
          <span className="text-text/60 tabular-nums">
            {getUpdateStatusText()}
          </span>
        )}

        {isInstalling && downloadProgress > 0 && downloadProgress < 100 && (
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
      </div>
    </>
  );
};

export default UpdateChecker;
