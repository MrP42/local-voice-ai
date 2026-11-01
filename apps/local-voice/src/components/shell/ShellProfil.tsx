import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { getVersion } from "@tauri-apps/api/app";
import {
  ArrowLeft,
  Bell,
  BookOpen,
  Cog,
  Cpu,
  Gauge,
  HelpCircle,
  Info,
  Keyboard,
  Play,
  Plug,
  RefreshCw,
  Activity,
  Check,
} from "lucide-react";
import { commands, type BudgetState, type LlmConnection } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { formatKeyCombination } from "../../lib/utils/keyboard";
import { useSettings } from "../../hooks/useSettings";
import { togglePageHelp, useHasPageHelp } from "../help/pageHelp";
import { useIntroTourStore } from "@/stores/introTourStore";
import type { SidebarSection } from "../Sidebar";
import {
  Profilknopf,
  ProfilmenueEintrag,
  ProfilmenueTrenner,
  useProfilmenue,
} from "../konto/Profilknopf";
import { ProfilHinzufuegenDialog } from "../konto/ProfilHinzufuegenDialog";
import ModelSelector from "../model-selector";
import { UPDATE_CHECK_EVENT } from "../update-checker/UpdateChecker";
import { LlmSelector } from "../footer/LlmSelector";
import { useDiagnose } from "../footer/diagnose";
import { useKonto } from "../konto/useKonto";
import { useKontoLabels } from "../konto/labels";

const ICON = 16;
/** Taste der Seitenhilfe (ADR-0027 E4); Tastenname, nicht übersetzt. */
const HILFE_TASTE = "F1";
const IS_MAC =
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform);

interface ShellProfilProps {
  currentSection: SidebarSection;
  onNavigate: (section: SidebarSection) => void;
  /** Aktion ohne Seitenwechsel, die außerhalb der Navigation wirkt
   * (Seitenhilfe, Update-Suche): schmal schließt der Rahmen die Schublade. */
  onAktion?: () => void;
}

type Ebene =
  "haupt" | "hilfe" | "modelle" | "kontingent" | "benachrichtigungen";

/** Budgetstände der Cloud-Verbindungen; ohne Backend leer. */
function useBudgets(): BudgetState[] {
  const [budgets, setBudgets] = useState<BudgetState[]>([]);
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const result = await commands.usageBudgetStates();
        if (!cancelled && result.status === "ok") setBudgets(result.data);
      } catch {
        // Ohne Backend (Browser-Test) bleibt das Kontingent unbekannt.
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);
  return budgets;
}

interface EintraegeProps extends ShellProfilProps {
  onUeber: () => void;
  onKuerzel: () => void;
}

/**
 * Einträge des Profilmenüs (ADR-0027 E2). Die Komponente lebt nur, solange das
 * Menü offen ist; dadurch startet jedes Öffnen wieder auf der obersten Ebene.
 */
const Eintraege: React.FC<EintraegeProps> = ({
  currentSection,
  onNavigate,
  onAktion,
  onUeber,
  onKuerzel,
}) => {
  const { t } = useTranslation();
  const [ebene, setEbene] = useState<Ebene>("haupt");
  // Springt etwas im Menü auf eine andere Seite (z. B. die Anbieter-
  // Einstellungen aus der Modellwahl), schließt das Menü mit.
  const { schliessen } = useProfilmenue();
  const startSection = useRef(currentSection);
  useEffect(() => {
    if (currentSection !== startSection.current) schliessen();
  }, [currentSection, schliessen]);
  // Beim Ebenenwechsel verschwindet der geklickte Knopf; der Fokus muss
  // im Menü bleiben (sonst greifen Escape und Pfeiltasten nicht mehr):
  // Unterebene → „Zurück“, zurück → der Eintrag, aus dem man kam.
  const vorherigeEbene = useRef<Ebene>("haupt");
  useEffect(() => {
    const von = vorherigeEbene.current;
    if (von === ebene) return;
    vorherigeEbene.current = ebene;
    const ziel =
      ebene === "haupt"
        ? `[data-profilmenue="${von}"]`
        : '[data-profilmenue="zurueck"]';
    document.querySelector<HTMLElement>(`.wai-konto-menue ${ziel}`)?.focus();
  }, [ebene]);
  const [diagnose, setDiagnose] = useDiagnose();
  const hasPageHelp = useHasPageHelp();
  const startTour = useIntroTourStore((state) => state.start);
  const budgets = useBudgets();
  const { getSetting } = useSettings();
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const settingsKey = IS_MAC ? "⌘," : t("shell.settingsKey");

  const zurueck = (
    <>
      <ProfilmenueEintrag
        symbol={<ArrowLeft size={ICON} />}
        text={t("shell.zurueck")}
        bleibtOffen
        onClick={() => setEbene("haupt")}
        data-profilmenue="zurueck"
      />
      <ProfilmenueTrenner />
    </>
  );

  if (ebene === "hilfe") {
    return (
      <>
        {zurueck}
        <ProfilmenueEintrag
          symbol={<BookOpen size={ICON} />}
          text={t("titlebar.helpPage")}
          kuerzel={HILFE_TASTE}
          disabled={!hasPageHelp}
          onClick={() => {
            onAktion?.();
            togglePageHelp();
          }}
          data-profilmenue="seitenhilfe"
        />
        <ProfilmenueEintrag
          symbol={<Keyboard size={ICON} />}
          text={t("titlebar.shortcuts")}
          onClick={onKuerzel}
        />
        <ProfilmenueEintrag
          symbol={<Play size={ICON} />}
          text={t("introTour.restart")}
          onClick={startTour}
          data-testid="titlebar-start-tour"
        />
        <ProfilmenueEintrag
          symbol={
            diagnose !== "aus" ? (
              <Check size={ICON} />
            ) : (
              <Activity size={ICON} />
            )
          }
          text={t("shell.diagnoseZeigen")}
          aria-pressed={diagnose !== "aus"}
          bleibtOffen
          onClick={() => setDiagnose(diagnose === "aus" ? "auf" : "aus")}
          data-profilmenue="diagnose"
        />
        <ProfilmenueTrenner />
        <ProfilmenueEintrag
          symbol={<RefreshCw size={ICON} />}
          text={t("shell.updates")}
          onClick={() => {
            onAktion?.();
            window.dispatchEvent(new Event(UPDATE_CHECK_EVENT));
          }}
        />
        <ProfilmenueEintrag
          symbol={<Info size={ICON} />}
          text={t("shell.ueber")}
          onClick={onUeber}
          data-profilmenue="ueber"
        />
      </>
    );
  }

  if (ebene === "modelle") {
    // Die Modellwahl (Diktat- und Sprachmodell) lebt seit ADR-0027 E7 hier
    // statt in der Statusleiste; die Auswahllisten klappen rechts neben dem
    // Menü auf.
    return (
      <>
        {zurueck}
        <div
          className="lv-profilmenue__modelle"
          data-testid="profilmenue-modelle"
        >
          <ModelSelector />
          <LlmSelector />
        </div>
        <ProfilmenueTrenner />
        <ProfilmenueEintrag
          symbol={<Cpu size={ICON} />}
          text={t("shell.modelleVerwalten")}
          aria-current={currentSection === "models" ? "page" : undefined}
          onClick={() => onNavigate("models")}
          data-profilmenue="modelle-verwalten"
        />
      </>
    );
  }

  if (ebene === "benachrichtigungen") {
    return (
      <>
        {zurueck}
        <p className="lv-profilmenue__leer">{t("titlebar.noNotifications")}</p>
      </>
    );
  }

  if (ebene === "kontingent") {
    const zeilen = budgets.filter((b) => b.spent_micro > 0 || b.limit_micro);
    return (
      <>
        {zurueck}
        {zeilen.length === 0 ? (
          <p className="lv-profilmenue__leer">{t("shell.kontingentLeer")}</p>
        ) : (
          <ul className="lv-profilmenue__kontingent" data-testid="kontingent">
            {zeilen.map((b) => {
              const name =
                connections.find((c) => c.id === b.connection_id)?.label ??
                b.connection_id;
              const frei =
                b.ratio === null
                  ? null
                  : Math.max(0, Math.round((1 - b.ratio) * 100));
              return (
                <li key={b.connection_id}>
                  <span className="lv-profilmenue__kontingentkopf">
                    <span>{name}</span>
                    <span>
                      {frei === null
                        ? t("shell.kontingentOhneLimit", {
                            betrag: `${(b.spent_micro / 1e6).toFixed(2)} $`,
                          })
                        : t("shell.kontingentFrei", { prozent: frei })}
                    </span>
                  </span>
                  {frei !== null && (
                    <span className="lv-profilmenue__balken" aria-hidden="true">
                      <i style={{ width: `${frei}%` }} />
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </>
    );
  }

  return (
    <>
      <ProfilmenueEintrag
        symbol={<Gauge size={ICON} />}
        text={t("shell.kontingent")}
        weiter
        onClick={() => setEbene("kontingent")}
        data-profilmenue="kontingent"
      />
      <ProfilmenueEintrag
        symbol={<Bell size={ICON} />}
        text={t("shell.benachrichtigungen")}
        weiter
        onClick={() => setEbene("benachrichtigungen")}
        data-profilmenue="benachrichtigungen"
      />
      <ProfilmenueEintrag
        symbol={<Cpu size={ICON} />}
        text={t("shell.modelle")}
        weiter
        onClick={() => setEbene("modelle")}
        data-profilmenue="modelle"
      />
      <ProfilmenueEintrag
        symbol={<Plug size={ICON} />}
        text={t("shell.integrationen")}
        aria-current={currentSection === "integrations" ? "page" : undefined}
        onClick={() => onNavigate("integrations")}
        data-profilmenue="integrationen"
      />
      <ProfilmenueEintrag
        symbol={<Cog size={ICON} />}
        text={t("shell.einstellungen")}
        kuerzel={settingsKey}
        aria-current={currentSection === "settings" ? "page" : undefined}
        onClick={() => onNavigate("settings")}
        data-profilmenue="einstellungen"
      />
      <ProfilmenueTrenner />
      <ProfilmenueEintrag
        symbol={<HelpCircle size={ICON} />}
        text={t("shell.hilfe")}
        weiter
        onClick={() => setEbene("hilfe")}
        data-profilmenue="hilfe"
      />
    </>
  );
};

/**
 * Tastenkürzel der Shell (ADR-0027): F1 schaltet die Hilfe zur Seite, Strg+,
 * (⌘,) öffnet die Einstellungen. Im App-Rahmen eingehängt, damit sie auch bei
 * schmalem Fenster mit geschlossener Navigation wirken.
 */
export function useShellKuerzel(
  onNavigate: (section: SidebarSection) => void,
  aktiv = true,
): void {
  useEffect(() => {
    // Während der Einrichtung (Onboarding) keine Shell-Kürzel.
    if (!aktiv) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "F1" && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        togglePageHelp();
      } else if (
        (IS_MAC ? event.metaKey : event.ctrlKey) &&
        !event.altKey &&
        event.key === ","
      ) {
        event.preventDefault();
        onNavigate("settings");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onNavigate, aktiv]);
}

/**
 * Profilkreis unten in der Navigation mit dem Profilmenü (ADR-0027, Suite-
 * Standard nach ChatGPT-Vorbild). Einziger Ort für Einstellungen, Modelle,
 * Integrationen, Benachrichtigungen, Kontingent und Hilfe; die Titelleiste
 * trägt keine Werkzeuge mehr. Die Kürzel F1 und Strg+, hängt der App-Rahmen
 * über `useShellKuerzel` ein.
 */
export const ShellProfil: React.FC<ShellProfilProps> = ({
  currentSection,
  onNavigate,
  onAktion,
}) => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const konto = useKonto();
  const kontoLabels = useKontoLabels();
  const [profilDialog, setProfilDialog] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [version, setVersion] = useState("");

  useEffect(() => {
    if (!aboutOpen || version) return;
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
  }, [aboutOpen, version]);

  const shortcuts = Object.values(settings?.bindings ?? {}).filter(
    (b): b is NonNullable<typeof b> => !!b,
  );

  return (
    <div className="lv-shell-profil" data-tour="nav-profile">
      <Profilknopf
        ort="leiste"
        profil={konto.aktiv}
        status={konto.status}
        weitereProfile={konto.weitere}
        onMeinProfil={() => onNavigate("profile")}
        onProfilWechseln={(id) =>
          konto
            .aktivieren(id)
            .catch(() => toast.error(t("konto.wechselnFehler")))
        }
        onProfilHinzufuegen={() => setProfilDialog(true)}
        labels={kontoLabels}
        eintraege={
          <Eintraege
            currentSection={currentSection}
            onNavigate={onNavigate}
            onAktion={onAktion}
            onUeber={() => setAboutOpen(true)}
            onKuerzel={() => setShortcutsOpen(true)}
          />
        }
      />
      <ProfilHinzufuegenDialog
        open={profilDialog}
        onOpenChange={setProfilDialog}
        onAnlegen={konto.anlegen}
      />

      <Dialog
        open={aboutOpen}
        onOpenChange={setAboutOpen}
        title={t("titlebar.about")}
        closeLabel={t("titlebar.close")}
        className="max-w-sm"
      >
        <div className="space-y-1" data-about-dialog>
          <p className="text-base font-semibold">{t("titlebar.aboutName")}</p>
          <p className="text-sm text-text-muted">{t("titlebar.aboutSuite")}</p>
          {version && (
            <p className="text-sm text-text-muted">
              {t("titlebar.aboutVersion", { version })}
            </p>
          )}
        </div>
      </Dialog>

      <Dialog
        open={shortcutsOpen}
        onOpenChange={setShortcutsOpen}
        title={t("titlebar.shortcutsTitle")}
        closeLabel={t("titlebar.close")}
        className="max-w-md"
      >
        <dl className="lv-shortcuts">
          {shortcuts.map((b) => (
            <React.Fragment key={b.id}>
              <dt>{b.name}</dt>
              <dd>
                <kbd>
                  {formatKeyCombination(
                    b.current_binding,
                    IS_MAC ? "macos" : "windows",
                  )}
                </kbd>
              </dd>
            </React.Fragment>
          ))}
          <dt>{t("titlebar.helpPage")}</dt>
          <dd>
            <kbd>{HILFE_TASTE}</kbd>
          </dd>
          <dt>{t("shell.einstellungen")}</dt>
          <dd>
            <kbd>{IS_MAC ? "⌘," : t("shell.settingsKey")}</kbd>
          </dd>
        </dl>
      </Dialog>
    </div>
  );
};
