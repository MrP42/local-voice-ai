/** JSON.parse allein verwirft doppelte Objektschluessel unbemerkt. */
export function parseUniqueJson(content: string): unknown {
  const data: unknown = JSON.parse(content);
  const tokens = content.match(/"(?:[^"\\]|\\.)*"|[{}\[\]:,]|[^\s{}\[\]:,]+/g) ?? [];
  const scopes: (Set<string> | null)[] = [];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (token === "{") scopes.push(new Set());
    else if (token === "[") scopes.push(null);
    else if (token === "}" || token === "]") scopes.pop();
    else if (token.startsWith('"') && tokens[i + 1] === ":") {
      const keys = scopes[scopes.length - 1];
      const key = JSON.parse(token) as string;
      if (keys?.has(key)) throw new Error("Duplicate JSON key");
      keys?.add(key);
    }
  }
  return data;
}
