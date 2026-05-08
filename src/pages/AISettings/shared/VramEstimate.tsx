import { useTranslation } from "react-i18next";
import { Info, Sparkles } from "lucide-react";
import {
  classifyVramRisk,
  effectiveVramGB,
  estimateVramGB,
  type RecommendedEngineParams,
  type VramRisk,
} from "./engineParams";
import type { VramInfo } from "../../../api/hindsight";
import styles from "../AISettings.module.css";

/** 估算行——配色：
 *  - 不传 `systemVram` 时按绝对值（< 16 GB 绿 / 16-24 橙 / > 24 红，回退到旧行为）
 *  - 传了 `systemVram` 则按"估算 / 系统"比例（< 60% safe / < 85% near / 否则 danger），
 *    且追加第二行展示"系统 VRAM 总量 · 风险标签"。
 *
 *  另支持"应用推荐"按钮：传 `recommended` + `onApplyRecommended` 时，第三行渲染
 *  按钮 + 灰字"推荐：slots N · ctx XK · batch Y"。当推荐值跟当前 picker 完全相同时
 *  按钮 disabled。`recommendedApplied` 由调用方算好传进来——避免组件本身知道当前值。 */
export function VramEstimateLine({
  modelName,
  parallelSlots,
  ctxSize,
  systemVram,
  recommended,
  recommendedApplied,
  onApplyRecommended,
}: {
  modelName: string;
  parallelSlots: number;
  ctxSize: number;
  systemVram?: VramInfo | null;
  recommended?: RecommendedEngineParams | null;
  recommendedApplied?: boolean;
  onApplyRecommended?: () => void;
}) {
  const { t } = useTranslation();
  if (!modelName.trim()) {
    return null;
  }
  const est = estimateVramGB(modelName, parallelSlots, ctxSize);
  // OOM 判断走 effective（Apple unified × 0.7）；UI 显示走 raw（systemVram.totalGb）
  const risk = classifyVramRisk(
    est.totalGB,
    systemVram ? effectiveVramGB(systemVram) : null,
  );

  // 选 className：有 systemVram 走 risk；没 systemVram 走绝对阈值
  let levelClass: string = styles.vramEstOk;
  if (risk) {
    levelClass = riskClass(risk, styles);
  } else if (est.totalGB > 24) {
    levelClass = styles.vramEstDanger;
  } else if (est.totalGB > 16) {
    levelClass = styles.vramEstWarn;
  }

  return (
    <div className={`${styles.vramEst} ${levelClass}`}>
      <div className={styles.vramEstRow}>
        <span className={styles.vramEstLabel}>
          {t("aiSettings.engineParams.vramLabel")}
        </span>
        <span className={styles.vramEstValue}>
          ~{est.totalGB.toFixed(1)} GB
        </span>
        {/* 权重 / KV / 模型 / ctx / slots 拆分细节折进 Info icon——
            原本占整行的灰字噪声大，hover / focus 显示更干净 */}
        <button
          type="button"
          className={styles.vramEstInfo}
          aria-label={t("aiSettings.engineParams.vramBreakdown", {
            weights: est.weightsGB.toFixed(1),
            kv: est.kvGB.toFixed(1),
            params: est.params,
            ctx: (ctxSize / 1024) | 0,
            slots: parallelSlots,
          })}
          title={t("aiSettings.engineParams.vramBreakdown", {
            weights: est.weightsGB.toFixed(1),
            kv: est.kvGB.toFixed(1),
            params: est.params,
            ctx: (ctxSize / 1024) | 0,
            slots: parallelSlots,
          })}
        >
          <Info size={12} strokeWidth={2} aria-hidden />
        </button>
        {systemVram && (
          <span className={styles.vramSystemSource}>
            {t(
              systemVram.source === "discrete"
                ? "aiSettings.engineParams.vramSystemDiscrete"
                : "aiSettings.engineParams.vramSystemUnified",
              { total: systemVram.totalGb.toFixed(1) },
            )}
          </span>
        )}
        {recommended && onApplyRecommended && (
          <button
            type="button"
            className={styles.vramRecommendBtn}
            onClick={onApplyRecommended}
            disabled={recommendedApplied}
            title={t("aiSettings.engineParams.recommendedHint", {
              slots: recommended.parallelSlots,
              ctx: ctxLabel(recommended.ctxSize),
              batch: batchLabel(recommended.batchSize),
            })}
          >
            <Sparkles size={12} strokeWidth={2.2} aria-hidden />
            <span>
              {recommendedApplied
                ? t("aiSettings.engineParams.recommendedAlreadyApplied")
                : t("aiSettings.engineParams.applyRecommended")}
            </span>
          </button>
        )}
      </div>
    </div>
  );
}

function ctxLabel(ctx: number | null): string {
  if (ctx == null) return "8K";
  return `${ctx / 1024}K`;
}

function batchLabel(batch: number | null): string {
  if (batch == null) return "512";
  return String(batch);
}

function riskClass(
  risk: VramRisk,
  styles: Record<string, string>,
): string {
  if (risk === "safe") return styles.vramEstOk;
  if (risk === "near") return styles.vramEstWarn;
  return styles.vramEstDanger;
}
