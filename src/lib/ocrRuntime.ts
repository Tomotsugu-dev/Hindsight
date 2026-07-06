// 文字识别(OCR)组件的就绪判定——常驻开关/立即回填/Chat banner 三个入口共用。

import { api } from "../api/hindsight";

/** OCR 组件是否就绪。macOS 走系统 Vision 恒就绪;Windows/Linux 看 onnxruntime
 *  是否已安装(Windows 上含 DirectML.dll 检查,旧 CPU 构建判未装)。
 *  查询失败按就绪放行——让后续动作走原有错误链路,而不是卡死在引导上。 */
export async function ocrRuntimeReady(): Promise<boolean> {
  try {
    const s = await api.getEngineStatus();
    return s.platformId.startsWith("macos") || s.embeddingRuntime.installed;
  } catch {
    return true;
  }
}
