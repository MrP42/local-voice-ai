// node --experimental-strip-types --test tests/unit/meetingMinutes.test.ts
import test from "node:test";
import assert from "node:assert/strict";
import {
  minutesErrorCode,
  minutesErrorDetail,
} from "../../src/lib/meetingMinutes.ts";
import {
  aiReleaseConfirmation,
  requestOpenMeetingAiRelease,
  onOpenMeetingAiRelease,
} from "../../src/lib/meetingAiRelease.ts";

test("Cloud-Sperren haben einen eigenen Code und geben keine Details preis", () => {
  for (const message of [
    "besprechung_cloud_gesperrt",
    "besprechung_cloud_gesperrt:nicht_sichtbar",
    "besprechung_cloud_gesperrt: SYNTHETISCHER-INHALT",
    "cloud_gesperrt",
    "cloud_gesperrt: SYNTHETISCHER-INHALT",
  ]) {
    assert.equal(minutesErrorCode(message), "cloud_gesperrt");
    assert.equal(minutesErrorDetail(message), "");
  }
  assert.equal(minutesErrorCode("besprechung_cloud_gesperrt_extra"), "llm_failed");
  assert.equal(minutesErrorCode("llm_failed: Test"), "llm_failed");
  assert.equal(minutesErrorDetail("nur_notizen: offen"), "offen");
});

test("Ad-hoc kann als unbekannt bestätigt werden; Terminverhalten bleibt gleich", () => {
  for (const [aus_termin, einstufung, expected] of [
    [false, "unbekannt", "unbekannt"],
    [false, "extern", "extern"],
    [true, "unbekannt", null],
    [true, "intern", "intern"],
    [true, "extern", "extern"],
  ] as const) {
    assert.equal(aiReleaseConfirmation({ aus_termin, einstufung }), expected);
  }
});

test("Freigabe-Anforderung öffnet nur die Details der gewünschten Aufnahme", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: new EventTarget(),
  });
  let opened = 0;
  const unsubscribe = onOpenMeetingAiRelease("synthetic-m1", () => opened++);
  try {
    requestOpenMeetingAiRelease("synthetic-m2");
    assert.equal(opened, 0);
    requestOpenMeetingAiRelease("synthetic-m1");
    assert.equal(opened, 1);
    unsubscribe();
    requestOpenMeetingAiRelease("synthetic-m1");
    assert.equal(opened, 1);
  } finally {
    unsubscribe();
    if (original) Object.defineProperty(globalThis, "window", original);
    else Reflect.deleteProperty(globalThis, "window");
  }
});
