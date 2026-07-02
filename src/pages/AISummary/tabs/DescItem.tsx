import { useTranslation } from "react-i18next";
import { Clock, RotateCcw } from "lucide-react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import type { AiSegment, ImageDescriptionRow } from "../../../api/hindsight";
import { resolveSegmentDotColor } from "../../../utils/segmentColor";
import { extractScreenshotTime } from "../../../utils/screenshotTime";
import { logWarn } from "../../../lib/logger";
import styles from "./DebugTab.module.css";

/** 单条逐图描述项。
 *  - 段标识背景色用 settings.ai.segments[idx].color（用户在「时段划分」里配的色）
 *  - 文件名 click → openPath 用系统默认查看器预览原图
 *  - 耗时 / token 来自 ai_image_descriptions 行 + image_described 事件
 *  - 重跑按钮调 api.retrySingleImageDescription */
export function DescItem({
  row,
  segmentLabel,
  segment,
  onRetry,
  retryDisabled,
  onOpenError,
}: {
  row: ImageDescriptionRow;
  segmentLabel?: string;
  segment?: AiSegment;
  onRetry: () => void;
  retryDisabled: boolean;
  /** 打开图片失败时上报错误给父组件展示（顶部 errorBar） */
  onOpenError: (msg: string) => void;
}) {
  const { t } = useTranslation();
  const fileName = row.screenshotPath.split(/[\\/]/).pop() ?? row.screenshotPath;
  // 截图时间（HH:MM）：直接从文件名解析（capture 写入约定 HHMMSS_NNN.jpg）
  const captureTime = extractScreenshotTime(row.screenshotPath);
  // 段标签＝色点 + 中性文字（跟 DailyTab / DebugTab 同款）：颜色只出现在圆点上。
  // settings 还没加载 (segment === undefined) 时圆点退回中性灰。
  const dotColor = segment ? resolveSegmentDotColor(segment) : "#cbd5e1";

  // 耗时 / token 文本：null 时显示 "—"，让排版稳定
  const latencyStr = row.latencyMs != null ? `${row.latencyMs} ms` : "—";
  const tokenStr =
    row.promptTokens != null || row.completionTokens != null
      ? `${row.promptTokens ?? "—"} / ${row.completionTokens ?? "—"} t`
      : "— / — t";

  return (
    <div className={styles.descItem}>
      <div className={styles.descMeta}>
        <span
          className={styles.descIndex}
          title={t("aiSummary.debug.perImage.chipTitle", {
            seg: row.segmentIdx,
            img: row.imageIndex,
          })}
        >
          <span
            className={styles.segDot}
            style={{ background: dotColor }}
            aria-hidden
          />
          {segmentLabel ?? t("aiSummary.debug.perImage.segFallback", { idx: row.segmentIdx })}
          {t("aiSummary.debug.perImage.imageNoSuffix", { n: row.imageIndex + 1 })}
        </span>
        <span
          className={styles.descTime}
          title={t("aiSummary.debug.perImage.captureTimeTooltip", { time: captureTime })}
        >
          <Clock size={11} strokeWidth={2.2} />
          {captureTime}
        </span>
        <button
          type="button"
          className={styles.descPath}
          title={t("aiSummary.debug.perImage.pathTooltip", { path: row.screenshotPath })}
          onClick={() => {
            // 先试系统默认查看器；失败 fallback 到资源管理器选中文件
            // ——后者用 opener:default 自带的 allow-reveal-item-in-dir 权限，
            // 不依赖 capability 是否新加了 allow-open-path
            void (async () => {
              try {
                await openPath(row.screenshotPath);
                return;
              } catch (e) {
                logWarn("debug.openPathFallback", e);
              }
              try {
                await revealItemInDir(row.screenshotPath);
              } catch (e2) {
                const msg = e2 instanceof Error ? e2.message : String(e2);
                onOpenError(t("aiSummary.debug.perImage.openImageError", { msg }));
              }
            })();
          }}
        >
          {fileName}
        </button>
        <span
          className={styles.descStat}
          title={t("aiSummary.debug.perImage.statTooltip")}
        >
          {latencyStr} · {tokenStr}
        </span>
        <button
          type="button"
          className={styles.retryImg}
          onClick={onRetry}
          disabled={retryDisabled}
          title={
            retryDisabled
              ? t("aiSummary.debug.perImage.retryTooltipBusy")
              : t("aiSummary.debug.perImage.retryTooltipReady")
          }
        >
          <RotateCcw size={11} strokeWidth={2.2} />
          {t("aiSummary.debug.perImage.retry")}
        </button>
      </div>
      <div className={styles.descText}>
        {row.description || t("aiSummary.debug.perImage.descEmpty")}
      </div>
    </div>
  );
}
