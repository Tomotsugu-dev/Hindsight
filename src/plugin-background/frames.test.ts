import { describe, expect, it } from "vitest";
import { diffFrames } from "./frames";

const url = (id: string, version: string) =>
  `hindsight-plugin://${id}.localhost/${version}/_launcher.html`;

describe("diffFrames", () => {
  it("adds new plugins and removes gone ones", () => {
    const running = new Map([
      ["focus", url("focus", "0.1.0")],
      ["pomodoro", url("pomodoro", "1.0.0")],
    ]);
    const next = [
      { id: "focus", launcherUrl: url("focus", "0.1.0") },
      { id: "weekly", launcherUrl: url("weekly", "0.2.0") },
    ];
    expect(diffFrames(running, next)).toEqual({
      remove: ["pomodoro"],
      add: [{ id: "weekly", launcherUrl: url("weekly", "0.2.0") }],
    });
  });

  it("replaces a plugin whose version changed", () => {
    const running = new Map([["focus", url("focus", "0.1.0")]]);
    const next = [{ id: "focus", launcherUrl: url("focus", "0.2.0") }];
    expect(diffFrames(running, next)).toEqual({ remove: ["focus"], add: next });
  });

  it("changes nothing when the list is the same", () => {
    const running = new Map([["focus", url("focus", "0.1.0")]]);
    const next = [{ id: "focus", launcherUrl: url("focus", "0.1.0") }];
    expect(diffFrames(running, next)).toEqual({ remove: [], add: [] });
  });
});
