/**
 * 统一前端错误日志格式：`[scope] err` —— 便于 devtools 里搜索。
 *
 * 故意不引入运行时收集 / 远端上报：那些是后端 logs 的事，前端 catch 主要给开发者
 * 看错误细节，prod build 这些日志也保留（无 noop 包装）。
 *
 * 调用约定：
 *   - scope 取一个能定位代码位置的小写 dot.path（如 "settings.load"）
 *   - err 直接传 catch 拿到的 e；console.{error,warn} 自带 stack 展开
 */
export function logError(scope: string, err: unknown): void {
  console.error(`[${scope}]`, err);
}

export function logWarn(scope: string, err: unknown): void {
  console.warn(`[${scope}]`, err);
}
