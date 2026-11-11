export default {
  get language(): string {
    return (globalThis as unknown as { __lang?: string }).__lang ?? "de";
  },
  t: (key: string): string => key,
};
