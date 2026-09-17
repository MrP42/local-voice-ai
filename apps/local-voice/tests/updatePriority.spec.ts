import { test, expect } from "@playwright/test";
import { compareVersions } from "../src/components/update-checker/UpdateChecker";

// Lokaler Ordner und GitHub werden verglichen: das Neuere gewinnt.
// 0.18.17 lokal darf 0.19.0 auf GitHub nicht verdecken (17.09.2026).
test("a GitHub release beats an older local installer", () => {
  expect(compareVersions("0.18.17", "0.19.0")).toBeLessThan(0);
});

test("a newer local acceptance build beats the GitHub release", () => {
  expect(compareVersions("0.19.1", "0.19.0")).toBeGreaterThan(0);
});

test("same version counts as local (no download needed)", () => {
  expect(compareVersions("0.19.0", "v0.19.0")).toBe(0);
});

test("numeric comparison, not string comparison", () => {
  expect(compareVersions("0.18.9", "0.18.17")).toBeLessThan(0);
});
