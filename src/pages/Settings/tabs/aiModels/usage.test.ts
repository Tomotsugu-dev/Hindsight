import { describe, expect, it } from "vitest";
import type { AiPicks } from "../../../../api/hindsight";
import { tasksUsing } from "./usage";

const qwen = { kind: "local" as const, file: "qwen.gguf" };
const openai = { kind: "cloud" as const, connection: "c1" };

describe("tasksUsing", () => {
  it("lists every task that picked the model", () => {
    const picks: AiPicks = { daily: qwen, weekly: openai, chat: qwen };
    expect(tasksUsing(picks, qwen)).toEqual(["daily", "chat"]);
    expect(tasksUsing(picks, openai)).toEqual(["weekly"]);
  });

  it("counts the weekly report when it follows the daily report", () => {
    const picks: AiPicks = { daily: qwen, weekly: { kind: "inherit" }, chat: null };
    expect(tasksUsing(picks, qwen)).toEqual(["daily", "weekly"]);
  });

  it("ignores spaces around a file name", () => {
    // 旧配置里的文件名可能带空白；之前的「Text」按钮也是 trim 之后比的
    const picks: AiPicks = {
      daily: { kind: "local", file: " qwen.gguf " },
      weekly: { kind: "inherit" },
      chat: null,
    };
    expect(tasksUsing(picks, qwen)).toEqual(["daily", "weekly"]);
  });

  it("is empty for a model no task uses", () => {
    const picks: AiPicks = { daily: null, weekly: { kind: "inherit" }, chat: null };
    expect(tasksUsing(picks, qwen)).toEqual([]);
    // 本机文件和云端配置不会因为名字一样被当成同一个
    expect(
      tasksUsing(
        { daily: openai, weekly: { kind: "inherit" }, chat: null },
        { kind: "local", file: "c1" },
      ),
    ).toEqual([]);
  });
});
