import { useEffect, useState, useRef, type ReactNode } from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { ModelStateEvent, RecordingErrorEvent } from "./lib/types/events";
import "./App.css";
import AccessibilityPermissions from "./components/AccessibilityPermissions";
import Footer from "./components/footer";
import { WorkspaceHome } from "./components/workspace/WorkspaceHome";
import Onboarding, { AccessibilityOnboarding } from "./components/onboarding";
import {
  Sidebar,
  SidebarSection,
  SECTIONS_CONFIG,
  isSidebarSection,
} from "./components/Sidebar";
import { WhatsNewGate } from "./components/whats-new";
import {
  Button as FluentButton,
  Tooltip as FluentTooltip,
} from "@fluentui/react-components";
import { Menu } from "lucide-react";
import { HelpDockSlotContext } from "./components/shell/PageHeaderSlot";
import { APP_NAME, AppMark, useIsNarrow } from "./components/shell/frame";
import { WindowControls } from "./components/shell/WindowControls";
import { ShellProfil, useShellKuerzel } from "./components/shell/ShellProfil";
import { ModelUpdateGate } from "./components/llm/ModelUpdateGate";
import { IntroTour } from "./components/intro-tour/IntroTour";
import {
  startFirstRunDictation,
  useFirstRunModelSelection,
} from "./components/intro-tour/firstRunSetup";
import { useIntroTourStore } from "./stores/introTourStore";
import { KopplungsDialog } from "./components/integrations/KopplungsDialog";
import { useSettings } from "./hooks/useSettings";
import { usePersistentState } from "./hooks/usePersistentState";
import { useSettingsStore } from "./stores/settingsStore";
import { commands } from "@/bindings";
import { installMainCallBridge } from "./components/voice-call/mainCall";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";
import { pasteNoticeKeys } from "@/lib/utils/pasteNotice";
import {
  releaseBackground,
  releaseBackgroundSoon,
} from "@/fluent/releaseBackground";

type OnboardingStep = "accessibility" | "model" | "done";

const renderSettingsContent = (section: SidebarSection) => {
  const ActiveComponent =
    SECTIONS_CONFIG[section]?.component || SECTIONS_CONFIG.settings.component;
  return <ActiveComponent />;
};

function App() {
  const { t, i18n } = useTranslation();
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(
    null,
  );
  // Track if this is a returning user who just needs to grant permissions
  // (vs a new user who needs full onboarding including model selection)
  const [isReturningUser, setIsReturningUser] = useState(false);
  const [currentSection, setCurrentSection] =
    usePersistentState<SidebarSection>("section", "home", isSidebarSection);
  const { settings, updateSetting } = useSettings();
  const direction = getLanguageDirection(i18n.language);
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);
  const firstRunStarted = useRef(false);
  const startIntroTour = useIntroTourStore((state) => state.startFirstTime);
  const firstRunFailed = useIntroTourStore(
    (state) => !!state.failedModelId && !state.open,
  );
  const pickerRequested = useIntroTourStore((state) => state.pickerRequested);
  useFirstRunModelSelection();
  // Download gescheitert und Tour geschlossen, oder „Modell selbst wählen“:
  // zurück zur Modellauswahl, solange die Ersteinrichtung offen ist.
  useEffect(() => {
    if (!firstRunFailed && !pickerRequested) return;
    if (isReturningUser) {
      // Kein Erststart mehr: alten Fehler nicht in spätere Touren tragen.
      useIntroTourStore.setState({
        failedModelId: null,
        pickerRequested: false,
      });
      return;
    }
    useIntroTourStore.setState({
      open: false,
      failedModelId: null,
      pickerRequested: false,
    });
    setOnboardingStep("model");
  }, [firstRunFailed, pickerRequested, isReturningUser]);
  // Rahmen: Navigation ein-/ausklappen (bleibt über Neustarts), schmal als
  // Overlay-Drawer.
  const [helpDock, setHelpDock] = useState<HTMLElement | null>(null);
  const [navCollapsedValue, setNavCollapsedValue] = usePersistentState<string>(
    "sidebar.collapsed",
    "0",
  );
  const navCollapsed = navCollapsedValue === "1";
  const narrow = useIsNarrow();
  // F1 (Seitenhilfe) und Strg+, (Einstellungen), auch bei geschlossener Navigation.
  useShellKuerzel(setCurrentSection);
  const [navDrawerOpen, setNavDrawerOpen] = useState(false);
  const hamburgerRef = useRef<HTMLButtonElement>(null);
  const drawerWasOpen = useRef(false);
  // Drawer zu (Esc, Hintergrund, Ziel gewählt): Hintergrund für Hilfstechnik
  // freigeben und den Fokus zum Menüknopf zurückgeben (Leitlinie §7), sofern
  // er nicht schon woanders steht.
  useEffect(() => {
    if (navDrawerOpen) {
      drawerWasOpen.current = true;
      return;
    }
    if (!drawerWasOpen.current) return;
    drawerWasOpen.current = false;
    // Der Drawer blendet noch aus (Fluent-Bewegung): Fokus sofort heraus,
    // aria-hidden erst nach dem Aushängen aufräumen.
    releaseBackgroundSoon();
    const late = setTimeout(releaseBackground, 700);
    const timer = setTimeout(() => {
      const active = document.activeElement;
      if (
        !active ||
        active === document.body ||
        !active.isConnected ||
        active.closest(".workspace-nav__drawer")
      )
        hamburgerRef.current?.focus();
    }, 0);
    return () => {
      clearTimeout(timer);
      clearTimeout(late);
    };
  }, [navDrawerOpen]);
  const hamburgerLabel = narrow
    ? t("workspace.navOpen")
    : navCollapsed
      ? t("workspace.navExpand")
      : t("workspace.navCollapse");

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  // Initialize RTL direction when language changes
  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  // Initialize Enigo, shortcuts, and refresh audio devices when main app loads
  useEffect(() => {
    if (onboardingStep === "done" && !hasCompletedPostOnboardingInit.current) {
      hasCompletedPostOnboardingInit.current = true;
      Promise.all([
        commands.initializeEnigo(),
        commands.initializeShortcuts(),
      ]).catch((e) => {
        console.warn("Failed to initialize:", e);
      });
      refreshAudioDevices();
      refreshOutputDevices();
    }
  }, [onboardingStep, refreshAudioDevices, refreshOutputDevices]);

  // Handle keyboard shortcuts for debug mode toggle
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      // Check for Ctrl+Shift+D (Windows/Linux) or Cmd+Shift+D (macOS)
      const isDebugShortcut =
        event.shiftKey &&
        event.key.toLowerCase() === "d" &&
        (event.ctrlKey || event.metaKey);

      if (isDebugShortcut) {
        event.preventDefault();
        const currentDebugMode = settings?.debug_mode ?? false;
        updateSetting("debug_mode", !currentDebugMode);
      }
    };

    // Add event listener when component mounts
    document.addEventListener("keydown", handleKeyDown);

    // Cleanup event listener when component unmounts
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [settings?.debug_mode, updateSetting]);

  // Listen for recording errors from the backend and show a toast
  useEffect(() => {
    const unlisten = listen<RecordingErrorEvent>("recording-error", (event) => {
      const { error_type, detail } = event.payload;

      if (error_type === "microphone_permission_denied") {
        const currentPlatform = platform();
        const platformKey = `errors.micPermissionDenied.${currentPlatform}`;
        const description = t(platformKey, {
          defaultValue: t("errors.micPermissionDenied.generic"),
        });
        toast.error(t("errors.micPermissionDeniedTitle"), { description });
      } else if (error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
      } else {
        toast.error(
          t("errors.recordingFailed", { error: detail ?? "Unknown error" }),
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Sprung aus einer Inhaltsseite in einen anderen Bereich (z. B. "Stimmen
  // verwalten" im Stimmen-Dropdown -> Einstellungen). Den Reiter setzt der
  // Absender vorher in localStorage, wie es der Start-Bereich schon tut.
  useEffect(() => {
    const handler = (event: Event) => {
      const section = (event as CustomEvent<{ section?: string }>).detail
        ?.section;
      if (section && isSidebarSection(section)) setCurrentSection(section);
    };
    window.addEventListener("lv-navigate", handler);
    return () => window.removeEventListener("lv-navigate", handler);
  }, [setCurrentSection]);

  // Hauptgespräch per Tastenkürzel: läuft im Hauptfenster, auch wenn es verborgen ist.
  useEffect(() => installMainCallBridge(), []);

  // "Vorbereiten" im Hinweisfenster (M5-P5e): das Hauptfenster kommt nach vorn
  // (Backend), hier wechselt es zu den Aufnahmen. Den Brief oeffnet dort die
  // Besprechungsseite selbst.
  useEffect(() => {
    const unlisten = listen("brief-request-event", () => {
      window.dispatchEvent(
        new CustomEvent("lv-navigate", { detail: { section: "meetings" } }),
      );
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Der Geraete-Sync hat Einstellungen von einem anderen Geraet uebernommen:
  // den Store neu lesen, sonst zeigt die Oberflaeche den alten Stand.
  const refreshSettings = useSettingsStore((state) => state.refreshSettings);
  useEffect(() => {
    const unlisten = listen("sync-changed", () => {
      void refreshSettings();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refreshSettings]);

  // Listen for paste failures and show a toast.
  // The technical error detail is logged to the app log on the Rust side
  // (see actions.rs `error!("Failed to paste transcription: ...")`),
  // so we show a localized, user-friendly message here instead of the raw error.
  useEffect(() => {
    const unlisten = listen("paste-error", () => {
      toast.error(t("errors.pasteFailedTitle"), {
        description: t("errors.pasteFailed"),
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // A finished dictation that was not inserted. The overlay carries the
  // primary, always-visible notice (the main window is usually hidden while
  // dictating); this toast is the second channel for when it is open.
  useEffect(() => {
    const unlisten = listen<{
      reason: string;
      transcript_in_clipboard: boolean;
      partial?: boolean;
    }>("paste-fallback", (event) => {
      const { reason, transcript_in_clipboard, partial } = event.payload;
      const keys = pasteNoticeKeys(partial, transcript_in_clipboard);
      toast.error(partial ? t(keys.title) : t("errors.pasteFallbackTitle"), {
        description: `${t(`overlay.notice.reason.${reason}`, {
          defaultValue: t("overlay.notice.reason.injection_failed"),
        })} ${t(keys.action)}`,
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for transcription failures and show a toast.
  // The payload is the backend error message (also logged to the app log).
  useEffect(() => {
    const unlisten = listen<string>("transcription-error", (event) => {
      toast.error(t("errors.transcriptionFailedTitle"), {
        description: event.payload,
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for model loading failures and show a toast
  useEffect(() => {
    const unlisten = listen<ModelStateEvent>("model-state-changed", (event) => {
      if (event.payload.event_type === "loading_failed") {
        toast.error(
          t("errors.modelLoadFailed", {
            model:
              event.payload.model_name || t("errors.modelLoadFailedUnknown"),
          }),
          {
            description: event.payload.error,
          },
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  const revealMainWindowForPermissions = async () => {
    try {
      await commands.showMainWindowCommand();
    } catch (e) {
      console.warn("Failed to show main window for permission onboarding:", e);
    }
  };

  const checkOnboardingStatus = async () => {
    try {
      const settingsResult = await commands.getAppSettings();
      const hasCompletedOnboarding =
        settingsResult.status === "ok" &&
        settingsResult.data.onboarding_completed === true;
      const currentPlatform = platform();

      if (hasCompletedOnboarding) {
        // Returning user - check if they need to grant permissions first
        setIsReturningUser(true);

        if (currentPlatform === "macos") {
          try {
            const [hasAccessibility, hasMicrophone] = await Promise.all([
              checkAccessibilityPermission(),
              checkMicrophonePermission(),
            ]);
            if (!hasAccessibility || !hasMicrophone) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check macOS permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        if (currentPlatform === "windows") {
          try {
            const microphoneStatus =
              await commands.getWindowsMicrophonePermissionStatus();
            if (
              microphoneStatus.supported &&
              microphoneStatus.overall_access === "denied"
            ) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check Windows microphone permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        setOnboardingStep("done");
      } else {
        // New user - start full onboarding
        setIsReturningUser(false);
        setOnboardingStep("accessibility");
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
      setOnboardingStep("accessibility");
    }
  };

  const handleAccessibilityComplete = async () => {
    // Returning users already have models, skip to main app
    if (isReturningUser) {
      setOnboardingStep("done");
      return;
    }
    // Erstinstallation: Diktatmodell (Vorgabe Nemotron 3.5) sofort laden und
    // ohne Auswahlbildschirm in die App, die Einführungstour zeigt den
    // Fortschritt. Hat der Installer das abgewählt, kommt die Modellauswahl.
    // Der Berechtigungsschritt kann zweimal melden: nur einmal einrichten.
    if (firstRunStarted.current) return;
    firstRunStarted.current = true;
    let automatic = false;
    try {
      automatic = await startFirstRunDictation();
    } catch (e) {
      console.warn("First-run dictation setup failed:", e);
    }
    setOnboardingStep(automatic ? "done" : "model");
    if (automatic) startIntroTour();
  };

  const handleModelSelected = () => {
    // Transition to main app - user has started a download
    setOnboardingStep("done");
    startIntroTour();
  };

  // Rendered once around every step below (including onboarding) so
  // toast.error() calls surface to the user. sonner renders via a portal, so
  // its position in the tree doesn't affect layout. Without this, errors during
  // onboarding (e.g. a model download failing because blob.handy.computer is
  // unreachable) are silently swallowed and the wizard just appears to "blink".
  const toaster = (
    <Toaster
      theme="system"
      toastOptions={{
        unstyled: true,
        classNames: {
          toast:
            "bg-background border border-mid-gray/20 rounded-lg shadow-lg px-4 py-3 flex items-center gap-3 text-sm",
          title: "font-medium",
          description: "text-mid-gray",
        },
      }}
    />
  );

  // Still checking onboarding status
  if (onboardingStep === null) {
    return null;
  }

  // Select the content for the current step. The Toaster is rendered once, in a
  // stable wrapper around this node, so crossing between onboarding steps and
  // the main app never remounts it (which would drop any in-flight toast).
  let content: ReactNode;
  if (onboardingStep === "accessibility" || onboardingStep === "model") {
    // Bildschirme vor der App: schlichte Titelleiste nur zum Ziehen und mit
    // den Fensterknöpfen (rahmenloses Fenster). Der Produktname steht als
    // Logo im Inhalt, deshalb hier kein zweiter Titel.
    content = (
      <div className="h-screen flex flex-col">
        <header
          className="lv-titlebar lv-titlebar--plain"
          data-tauri-drag-region="deep"
        >
          <WindowControls />
        </header>
        <div className="flex-1 min-h-0 overflow-auto">
          {onboardingStep === "accessibility" ? (
            <AccessibilityOnboarding onComplete={handleAccessibilityComplete} />
          ) : (
            <Onboarding onModelSelected={handleModelSelected} />
          )}
        </div>
      </div>
    );
  } else {
    content = (
      <div
        dir={direction}
        className="h-screen flex flex-col select-none cursor-default"
      >
        <WhatsNewGate />
        <ModelUpdateGate />
        <IntroTour />
        <KopplungsDialog />
        {/* Titelleiste 40px (ADR-0026 E3, Variante A): nur Hamburger, App-Symbol
            und Name, freie Ziehfläche, Fensterknöpfe. Seitentitel, Suche und
            Aktionen stehen im Modulkopf (PageShell), Einstellungen und Hilfe
            im Profilmenü unten links. Das Hauptfenster ist rahmenlos, dies ist
            die einzige Titelleiste: freie Flächen ziehen das Fenster, ein
            Doppelklick maximiert ("deep" = ganzer Teilbaum, Bedienelemente
            ausgenommen). */}
        <header className="lv-titlebar" data-tauri-drag-region="deep">
          <FluentTooltip content={hamburgerLabel} relationship="label">
            <FluentButton
              ref={hamburgerRef}
              appearance="subtle"
              className="lv-titlebar__hamburger"
              icon={<Menu size={20} aria-hidden="true" />}
              aria-expanded={narrow ? navDrawerOpen : !navCollapsed}
              aria-controls={narrow ? undefined : "workspace-nav-items"}
              onClick={() =>
                narrow
                  ? setNavDrawerOpen(true)
                  : setNavCollapsedValue(navCollapsed ? "0" : "1")
              }
            />
          </FluentTooltip>
          <span className="lv-titlebar__app">
            <AppMark />
            <span className="lv-titlebar__name">{APP_NAME}</span>
          </span>
          <span className="lv-titlebar__frei" />
          <WindowControls />
        </header>
        <div className="workspace-shell flex-1 flex overflow-hidden">
          <Sidebar
            activeSection={currentSection}
            onSectionChange={setCurrentSection}
            collapsed={navCollapsed}
            narrow={narrow}
            drawerOpen={navDrawerOpen}
            onDrawerOpenChange={setNavDrawerOpen}
            fuss={
              <ShellProfil
                currentSection={currentSection}
                onNavigate={(section) => {
                  setCurrentSection(section);
                  setNavDrawerOpen(false);
                }}
              />
            }
          />
          {/* Inhaltsschicht (heller, Radius 8 oben links), nur sie scrollt */}
          <div className="lv-content flex-1 flex flex-col overflow-hidden min-w-0">
            <main
              id="workspace-content"
              className={`flex-1 workspace-main ${currentSection === "tts" || currentSection === "meetings" || currentSection === "voice" ? "workspace-main--fill" : ""} ${currentSection === "meetings" || currentSection === "voice" ? "workspace-main--fixed" : ""}`}
              aria-label={t(SECTIONS_CONFIG[currentSection].labelKey)}
            >
              {/* Fluid: the content uses whatever width the window offers, up
                  to a readable ceiling, and never forces the page to scroll
                  sideways (`min-w-0` on the flex child above does the same for
                  the column itself). */}
              <div
                className={`flex flex-col w-full mx-auto p-3 sm:p-4 gap-4 min-w-0 ${
                  currentSection === "tts" ||
                  currentSection === "meetings" ||
                  currentSection === "voice"
                    ? "workspace-content--fill"
                    : ""
                } ${
                  currentSection === "meetings" || currentSection === "voice"
                    ? "workspace-content--fixed"
                    : ""
                } ${
                  // Vorlesen ist eine dreispaltige Arbeitsflaeche (Seiten,
                  // Inhalt, Dateien) — der Lese-Deckel wuerde dort die MITTE
                  // verjuengen, denn die Leisten haben feste Breiten.
                  // Besprechungen sind ebenfalls eine Arbeitsflaeche (Liste,
                  // Transkript, Protokoll) und nutzen die volle Breite. Alle
                  // anderen Bereiche behalten die lesefreundliche Obergrenze.
                  // Sprachgespräch: Verlauf zentriert, Einstellungen als Spalte rechts.
                  currentSection === "tts" ||
                  currentSection === "meetings" ||
                  currentSection === "voice"
                    ? ""
                    : "max-w-5xl"
                }`}
              >
                <AccessibilityPermissions />
                {currentSection === "home" ? (
                  <WorkspaceHome onNavigate={setCurrentSection} />
                ) : (
                  renderSettingsContent(currentSection)
                )}
              </div>
            </main>
          </div>
          {/* Seitenhilfe dockt am rechten Fensterrand an (Suite-Standard),
              neben der Inhaltsschicht: der Inhalt behält seine Breite. */}
          <div ref={setHelpDock} className="lv-helpdock" />
        </div>
        {/* Statusleiste (Leitlinie §2, 24 px): Modelle links; Mikrofon, Speicher, Version rechts */}
        <footer className="lv-statusbar">
          <Footer />
        </footer>
      </div>
    );
  }

  return (
    <HelpDockSlotContext.Provider value={helpDock}>
      {toaster}
      {content}
    </HelpDockSlotContext.Provider>
  );
}

export default App;
