import { useTranslation } from "react-i18next";
import { estimateVramGB } from "./engineParams";
import styles from "../AISettings.module.css";

/** 估算行——三档配色：< 16 GB 绿、16-24 橙、> 24 红。
 *  跟 DebugTab 的 VramEstimateLine 视觉一致。 */
export function VramEstimateLine({
  modelName,
  parallelSlots,
  ctxSize,
}: {
  modelName: string;
  parallelSlots: number;
  ctxSize: number;
}) {
  const { t } = useTranslation();
  if (!modelName.trim()) {
    return null;
  }
  const est = estimateVramGB(modelName, parallelSlots, ctxSize);
  let levelClass = styles.vramEstOk;
  if (est.totalGB > 24) levelClass = styles.vramEstDanger;
  else if (est.totalGB > 16) levelClass = styles.vramEstWarn;
  return (
    <div className={`${styles.vramEst} ${levelClass}`}>
      <span className={styles.vramEstLabel}>
        {t("aiSettings.engineParams.vramLabel")}
      </span>
      <span className={styles.vramEstValue}>~{est.totalGB.toFixed(1)} GB</span>
      <span className={styles.vramEstBreakdown}>
        {t("aiSettings.engineParams.vramBreakdown", {
          weights: est.weightsGB.toFixed(1),
          kv: est.kvGB.toFixed(1),
          params: est.params,
          ctx: (ctxSize / 1024) | 0,
          slots: parallelSlots,
        })}
      </span>
    </div>
  );
}
