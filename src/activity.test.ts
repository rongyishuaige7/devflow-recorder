import { describe, expect, it } from "vitest";
import {
  countUsableProviders,
  eventDurationSeconds,
  formatDuration,
  formatTotalDuration,
  totalDurationSeconds
} from "./activity";

describe("activity duration", () => {
  it("prefers exact seconds and falls back to minutes", () => {
    expect(eventDurationSeconds({ durationSeconds: 45, durationMinutes: 9 })).toBe(45);
    expect(eventDurationSeconds({ durationMinutes: 3 })).toBe(180);
  });

  it("adds mixed second and minute events", () => {
    expect(totalDurationSeconds([
      { durationSeconds: 30, durationMinutes: 0 },
      { durationMinutes: 2 }
    ])).toBe(150);
    expect(totalDurationSeconds([])).toBe(0);
  });

  it("formats event and total durations at boundaries", () => {
    expect(formatDuration({ durationSeconds: 0, durationMinutes: 0 })).toBe("1s");
    expect(formatDuration({ durationSeconds: 119, durationMinutes: 0 })).toBe("1m");
    expect(formatTotalDuration(0)).toBe("0s");
    expect(formatTotalDuration(3_900)).toBe("1h 5m");
  });
});

describe("provider availability", () => {
  it("counts ready, available, and partial providers only", () => {
    expect(countUsableProviders([
      { state: "ready" },
      { state: "available" },
      { state: "partial" },
      { state: "planned" },
      { state: "standby" }
    ])).toBe(3);
  });
});
