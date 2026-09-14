import React from "react";
import { useTranslation } from "react-i18next";
import { MarkdownContent } from "../whats-new/MarkdownContent";

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

export const HelpPanel: React.FC<{ section: string }> = ({ section }) => {
  const { t, i18n } = useTranslation();
  const markdown = findHelp(section, i18n.language ?? "en");
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
