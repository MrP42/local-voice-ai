import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { useModelStore } from "@/stores/modelStore";
import { PageShell } from "@/components/ui/PageShell";
import { TabList } from "@/components/ui/TabList";
import { usePersistentState } from "@/hooks/usePersistentState";
import { LlmSection } from "./LlmSection";
import { DictationSection } from "./DictationSection";
import { VoicesSection } from "./VoicesSection";

export type ModelsArea = "llm" | "dictation" | "voices";
type Area = ModelsArea;
const AREAS: Area[] = ["llm", "dictation", "voices"];
const AREA_KEY = "models.area";

/**
 * Zur Modellseite springen und dort einen Bereich zeigen (z. B. "Vorlesen"
 * vom Einrichten-Hinweis der Vorlesen-Seite aus). Der Bereich wird vor dem
 * Sprung gemerkt -- die Seite liest ihn beim Oeffnen.
 */
export const openModelsArea = (area: ModelsArea) => {
  try {
    window.localStorage.setItem(`lva.ui.${AREA_KEY}`, area);
  } catch {
    /* ohne Speicher landet man im zuletzt gewaehlten Bereich */
  }
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "models" } }),
  );
};

/**
 * Die Modellseite: drei Bereiche zum Umschalten -- Sprachmodelle (KI),
 * Diktat, Vorlesen. Jeder zeigt oben das aktive Modell, darunter das
 * Installierte als Zeilen, und zugeklappt, was man laden kann. Vorher stand
 * alles untereinander als grosse Karten (06.10.2026: "muss unbedingt
 * uebersichtlicher").
 */
export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const { loading } = useModelStore();
  const [area, setArea] = usePersistentState<Area>(AREA_KEY, "llm", (v) =>
    (AREAS as string[]).includes(v),
  );
  const [query, setQuery] = useState("");
  // Beim Wechsel des Bereichs beginnt die Suche von vorn: ein Suchwort aus
  // "Diktat" ergibt bei den Sprachmodellen meist eine leere Liste.
  useEffect(() => setQuery(""), [area]);

  if (loading) {
    return (
      <div className="w-full">
        <div className="flex items-center justify-center py-16">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-logo-primary border-t-transparent" />
        </div>
      </div>
    );
  }

  return (
    <PageShell
      title={t("sidebar.models")}
      description={t("workspace.modelsHint")}
      help="modelle"
    >
      <div className="flex flex-wrap items-end justify-between gap-3 border-b border-mid-gray/20">
        <TabList
          tabs={AREAS.map((id) => ({
            id,
            label: t(`settings.models.areas.${id}`),
          }))}
          value={area}
          onChange={setArea}
          ariaLabel={t("sidebar.models")}
        />
        {/* Suche im gewaehlten Bereich -- die Lupe als Geschwister im Fluss,
            nicht als Ueberlagerung (die verrutschte frueher). */}
        <label className="mb-2 flex w-full items-center gap-2 rounded-lg border border-mid-gray/40 bg-mid-gray/10 px-3 py-1.5 focus-within:ring-1 focus-within:ring-logo-primary sm:w-64">
          <Search
            className="h-4 w-4 shrink-0 text-text/40"
            aria-hidden="true"
          />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("settings.models.searchPlaceholder")}
            className="min-w-0 flex-1 bg-transparent text-sm placeholder:text-text/40 focus:outline-none"
          />
        </label>
      </div>

      <div role="tabpanel">
        {area === "llm" && <LlmSection query={query} />}
        {area === "dictation" && <DictationSection query={query} />}
        {area === "voices" && <VoicesSection query={query} />}
      </div>
    </PageShell>
  );
};

export default ModelsSettings;
