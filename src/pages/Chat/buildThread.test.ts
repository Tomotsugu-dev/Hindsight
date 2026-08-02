// buildThread(消息树 → 当前路径)的行为测试:
// 编辑分支的选择与隔离、重试链折叠、坏数据防环。
import { describe, expect, it } from "vitest";
import { buildThread } from "./thread";
import type { ChatStoredMessage } from "../../api/hindsight";

let nextId = 1;
function row(
  role: "user" | "assistant",
  content: string,
  guid: string,
  parentGuid: string | null,
): ChatStoredMessage {
  return {
    id: nextId++,
    role,
    content,
    citations: [],
    degraded: false,
    createdTs: "2026-08-01T10:00:00+08:00",
    promptTokens: null,
    completionTokens: null,
    reasoningTokens: null,
    elapsedMs: null,
    guid,
    parentGuid,
  };
}

describe("buildThread", () => {
  it("线性链按序走完,叶子是最后一行", () => {
    const rows = [
      row("user", "问1", "u1", null),
      row("assistant", "答1", "a1", "u1"),
      row("user", "问2", "u2", "a1"),
      row("assistant", "答2", "a2", "u2"),
    ];
    const t = buildThread(rows, {});
    expect(t.messages.map((m) => (m.role === "user" ? m.text : m.versions[0].text))).toEqual([
      "问1",
      "答1",
      "问2",
      "答2",
    ]);
    expect(t.leafGuid).toBe("a2");
  });

  it("编辑分支:默认走最新兄弟,choice 可切回旧分支且互相隔离", () => {
    const rows = [
      row("user", "原问", "u1", null),
      row("assistant", "原答", "a1", "u1"),
      row("user", "编辑后的问", "u2", null), // u1 的兄弟(编辑首条提问)
      row("assistant", "新答", "a2", "u2"),
    ];
    // 默认:最新分支
    const latest = buildThread(rows, {});
    expect(latest.messages[0]).toMatchObject({
      role: "user",
      text: "编辑后的问",
      branchIdx: 1,
      branchCount: 2,
    });
    expect(latest.leafGuid).toBe("a2");
    // 选回旧分支:整段下游随之切换
    const old = buildThread(rows, { "": "u1" });
    expect(old.messages[0]).toMatchObject({ text: "原问", branchIdx: 0 });
    expect(
      old.messages[1].role === "assistant" && old.messages[1].versions[0].text,
    ).toBe("原答");
    expect(old.leafGuid).toBe("a1");
  });

  it("重试链折叠为版本组,leafGuid 指向最新版", () => {
    const rows = [
      row("user", "问", "u1", null),
      row("assistant", "答·旧", "a1", "u1"),
      row("assistant", "答·新", "a2", "a1"), // 重试:链式续行
    ];
    const t = buildThread(rows, {});
    expect(t.messages).toHaveLength(2);
    const asst = t.messages[1];
    if (asst.role !== "assistant") throw new Error("应为回答组");
    expect(asst.versions.map((v) => v.text)).toEqual(["答·旧", "答·新"]);
    expect(asst.leafGuid).toBe("a2");
    expect(t.leafGuid).toBe("a2");
  });

  it("choice 指向不存在的 guid 时回落最新分支", () => {
    const rows = [
      row("user", "问A", "u1", null),
      row("user", "问B", "u2", null),
    ];
    const t = buildThread(rows, { "": "ghost" });
    expect(t.messages[0]).toMatchObject({ text: "问B" });
  });

  it("坏数据成环不死循环", () => {
    const rows = [
      row("user", "环1", "x1", "x2"),
      row("user", "环2", "x2", "x1"),
    ];
    // 根下没有可走的孩子(全在环里悬空)→ 空路径,函数正常返回
    const t = buildThread(rows, {});
    expect(t.messages).toEqual([]);
    expect(t.leafGuid).toBeNull();
  });
});
