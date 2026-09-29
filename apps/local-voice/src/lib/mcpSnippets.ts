/**
 * Konfig-Schnipsel fuer den lokalen MCP-Server (M6-P6e, F21). Reine Logik:
 * der Client startet `local-voice-ai.exe --mcp` selbst, der Nutzer traegt nur
 * den Pfad der EXE ein.
 */

export type McpClient = "claude-code" | "claude-desktop" | "codex";

export const MCP_CLIENTS: McpClient[] = [
  "claude-code",
  "claude-desktop",
  "codex",
];

/** Name, unter dem der Server im Client erscheint. */
export const MCP_SERVER_NAME = "local-voice";

/** Platzhalter, solange der Pfad der EXE nicht bekannt ist. */
export const MCP_EXE_PLACEHOLDER = "<Pfad zu local-voice-ai.exe>";

/** TOML-Zeichenkette: woertlich (`'...'`, keine Maskierung von `\`), sonst maskiert. */
const tomlString = (value: string): string =>
  value.includes("'")
    ? `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`
    : `'${value}'`;

export const mcpSnippet = (
  client: McpClient,
  exePath: string | null,
): string => {
  const exe = exePath && exePath.trim() ? exePath : MCP_EXE_PLACEHOLDER;
  switch (client) {
    case "claude-code":
      return `claude mcp add ${MCP_SERVER_NAME} -- "${exe}" --mcp`;
    case "claude-desktop":
      return JSON.stringify(
        {
          mcpServers: {
            [MCP_SERVER_NAME]: { command: exe, args: ["--mcp"] },
          },
        },
        null,
        2,
      );
    case "codex":
      return [
        `[mcp_servers.${MCP_SERVER_NAME}]`,
        `command = ${tomlString(exe)}`,
        `args = ["--mcp"]`,
      ].join("\n");
  }
};
