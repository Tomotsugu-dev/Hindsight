import type { AiConfig, CloudConnection, ModelRef } from "../../api/hindsight";

/** 判断一条云端连接是否填齐了必填项（endpoint + model；apiKey 可选）。 */
export function isCloudConnectionReady(c: CloudConnection): boolean {
  return c.endpoint.trim() !== "" && c.model.trim() !== "";
}

/** 所有可用的云端连接。 */
export function readyCloudConnections(ai: AiConfig): CloudConnection[] {
  return ai.cloud.filter(isCloudConnectionReady);
}

/** 按 id 查找云端连接。 */
export function findCloudConnection(ai: AiConfig, id: string): CloudConnection | null {
  const key = id.trim();
  return ai.cloud.find((c) => c.id === key) ?? null;
}

/** 是否至少有一条可用的云端连接。 */
export function chatCloudReady(ai: AiConfig): boolean {
  return readyCloudConnections(ai).length > 0;
}

/** 对话任务选中的模型引用（去掉空白占位）。 */
export function chatModel(ai: AiConfig): ModelRef | null {
  const pick = ai.picks.chat;
  if (!pick) return null;
  if (pick.kind === "local") {
    const file = pick.file.trim();
    return file ? { kind: "local", file } : null;
  }
  const connection = pick.connection.trim();
  return connection ? { kind: "cloud", connection } : null;
}

/** 对话当前路由到的可用云端连接；走本地或未选时返回 null。 */
export function chatCloudConnection(ai: AiConfig): CloudConnection | null {
  const model = chatModel(ai);
  if (model?.kind !== "cloud") return null;
  const conn = findCloudConnection(ai, model.connection);
  return conn && isCloudConnectionReady(conn) ? conn : null;
}

/** 对话当前是否走云端。 */
export function chatUsesCloud(ai: AiConfig): boolean {
  return chatCloudConnection(ai) !== null;
}

/** 对话当前使用的本地模型文件名；走云端或未选时返回空串。 */
export function chatLocalModelName(ai: AiConfig): string {
  const model = chatModel(ai);
  return model?.kind === "local" ? model.file : "";
}
