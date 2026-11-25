import type { AiPicks, ModelRef } from "../../../../api/hindsight";
import type { AiTask } from "../../../../state/reportModel";

const TASKS: AiTask[] = ["daily", "weekly", "chat"];

function same(a: ModelRef | null, b: ModelRef): boolean {
  if (!a || a.kind !== b.kind) return false;
  return a.kind === "local"
    ? a.file.trim() === (b as { file: string }).file.trim()
    : a.connection === (b as { connection: string }).connection;
}

/**
 * Return the tasks that use the model, in the order daily, weekly, chat.
 *
 * The weekly report uses the daily report's model when it is set to follow it.
 */
export function tasksUsing(picks: AiPicks, model: ModelRef): AiTask[] {
  const weekly = picks.weekly.kind === "inherit" ? picks.daily : picks.weekly;
  const chosen: Record<AiTask, ModelRef | null> = {
    daily: picks.daily,
    weekly,
    chat: picks.chat,
  };
  return TASKS.filter((task) => same(chosen[task], model));
}
