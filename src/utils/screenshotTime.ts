// 跟后端 capture/screenshot.rs 写文件名 + ai/summary_operations.rs::extract_time_label
// 同款规则：截图保存为 `HHMMSS_NNN.jpg`（本机时区），从中取出 `HH:MM` 给 UI 展示。
//
// 只到分级精度——逐图描述列表里精确到秒没意义（vision 模型本来就抽帧），
// 顺便跟 step 2 段总结里 LLM 看到的时间标签对齐。

/** 从截图绝对路径解析出 `HH:MM` 时间标签。失败返回 "??:??"。 */
export function extractScreenshotTime(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? "";
  const stem = file.replace(/\.[^.]+$/, "");
  const head = stem.split("_")[0] ?? "";
  if (head.length === 6 && /^\d{6}$/.test(head)) {
    return `${head.slice(0, 2)}:${head.slice(2, 4)}`;
  }
  return "??:??";
}
