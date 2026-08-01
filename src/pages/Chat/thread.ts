// 消息树 → 当前路径:Chat 的分支模型纯逻辑(与渲染解耦,单测在 buildThread.test.ts)。
import type { ChatCitation, ChatStoredMessage } from "../../api/hindsight";

/** 助手回答的一个版本(「重试」按链式追加落库,渲染时折叠切换)。 */
export interface AssistantVersion {
  text: string;
  citations: ChatCitation[];
  degraded: boolean;
  /** 本轮上行/下行 token；旧数据为 null 不显示 */
  promptTokens: number | null;
  completionTokens: number | null;
}

/** 当前路径上的一条渲染消息。user 携带编辑分支信息(同父兄弟 = 编辑产生的
 *  分支,‹n/n› 切换);assistant 是重试版本组。 */
export type Message =
  | {
      id: string;
      role: "user";
      text: string;
      guid: string;
      /** 分支组 key(= parentGuid ?? "");"" 表示会话根 */
      parentKey: string;
      branchIdx: number;
      branchCount: number;
      /** 同组兄弟的 guid(按创建序,切换分支用) */
      siblings: string[];
    }
  | { id: string; role: "assistant"; versions: AssistantVersion[]; leafGuid: string };

/** 按分支选择走出的当前路径。leafGuid = 路径最后一行(发送与重试的挂点)。 */
export interface Thread {
  messages: Message[];
  leafGuid: string | null;
}

/** 库行(消息树)→ 当前路径:
 *  - 同父的多条 user = 编辑产生的分支,按 `choice`(缺省最新)选一条走;
 *  - assistant 沿链取,相邻的折叠成重试版本组;
 *  - 旧线性数据迁移后是单链,天然退化为原有渲染。 */
export function buildThread(
  rows: ChatStoredMessage[],
  choice: Record<string, string>,
): Thread {
  const byParent = new Map<string, ChatStoredMessage[]>();
  for (const m of rows) {
    const key = m.parentGuid ?? "";
    const arr = byParent.get(key);
    if (arr) arr.push(m);
    else byParent.set(key, [m]);
  }
  const messages: Message[] = [];
  let leafGuid: string | null = null;
  let parentKey = "";
  // 步数上限 = 行数:同步合并出脏环时不至于死循环
  for (let steps = 0; steps <= rows.length; steps++) {
    const kids = byParent.get(parentKey) ?? [];
    if (kids.length === 0) break;
    const users = kids.filter((k) => k.role === "user");
    let next: ChatStoredMessage;
    if (users.length > 0) {
      const chosen = choice[parentKey];
      const found = chosen ? users.findIndex((u) => u.guid === chosen) : -1;
      const pick = found >= 0 ? found : users.length - 1;
      next = users[pick];
      messages.push({
        id: `db-${next.id}`,
        role: "user",
        text: next.content,
        guid: next.guid,
        parentKey,
        branchIdx: pick,
        branchCount: users.length,
        siblings: users.map((u) => u.guid),
      });
    } else {
      next = kids[kids.length - 1];
      const v: AssistantVersion = {
        text: next.content,
        citations: next.citations,
        degraded: next.degraded,
        promptTokens: next.promptTokens,
        completionTokens: next.completionTokens,
      };
      const last = messages[messages.length - 1];
      if (last && last.role === "assistant") {
        last.versions.push(v);
        last.leafGuid = next.guid;
      } else {
        messages.push({
          id: `db-${next.id}`,
          role: "assistant",
          versions: [v],
          leafGuid: next.guid,
        });
      }
    }
    leafGuid = next.guid;
    parentKey = next.guid;
  }
  return { messages, leafGuid };
}
