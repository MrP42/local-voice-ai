import React from "react";
import { useTranslation } from "react-i18next";
import { MarkdownContent } from "../whats-new/MarkdownContent";
import { useTtsModelStore } from "@/stores/ttsModelStore";

/**
 * Hilfe an Ort und Stelle: je Bereich ein Markdown-Text je Sprache unter
 * `src/content/help/<bereich>.<sprache>.md`. Fehlt die Sprache, kommt
 * Englisch; fehlt auch das, sagt das Panel, dass hier noch nichts steht —
 * statt leer zu bleiben.
 */
const helpModules = import.meta.glob<string>("../../content/help/*.md", {
  eager: true,
  import: "default",
  query: "?raw",
});

const helpByKey = new Map<string, string>();
for (const [path, markdown] of Object.entries(helpModules)) {
  const match = path.match(/help\/([^/.]+)\.([a-z]{2})\.md$/);
  if (match) helpByKey.set(`${match[1]}.${match[2]}`, markdown);
}

export const findHelp = (section: string, language: string): string | null => {
  const lang = language.split("-")[0]?.toLowerCase() ?? "en";
  return (
    helpByKey.get(`${section}.${lang}`) ??
    helpByKey.get(`${section}.en`) ??
    null
  );
};

/**
 * Bedingte Bloecke: `<!--if:fish-->...<!--/if:fish-->` steht nur, wenn Fish
 * Speech eingerichtet ist; `<!--if:nofish-->...<!--/if:nofish-->` nur, wenn
 * nicht. So bleibt die Hilfe zu nicht eingerichteten Funktionen aus dem Weg
 * (Issue #29), ohne die Texte zu duplizieren.
 */
export const filterHelp = (markdown: string, fish: boolean): string =>
  markdown.replace(
    /<!--if:(fish|nofish)-->\r?\n?([\s\S]*?)<!--\/if:\1-->\r?\n?/g,
    (_match, kind: string, body: string) =>
      (kind === "fish") === fish ? body : "",
  );

export const HelpPanel: React.FC<{ section: string }> = ({ section }) => {
  const { t, i18n } = useTranslation();
  // Unbekannt gilt als eingerichtet: kein Flackern, unveraenderte Hilfe ohne Antwort.
  const fishReady = useTtsModelStore((state) =>
    state.runtime ? state.runtime.fish.ready : true,
  );
  const raw = findHelp(section, i18n.language ?? "en");
  const markdown = raw === null ? null : filterHelp(raw, fishReady);
  return (
    <div className="help-panel space-y-3" data-testid="help-panel">
      {markdown ? (
        <MarkdownContent markdown={markdown} />
      ) : (
        <p className="text-xs text-text/40">{t("help.empty")}</p>
      )}
    </div>
  );
};
