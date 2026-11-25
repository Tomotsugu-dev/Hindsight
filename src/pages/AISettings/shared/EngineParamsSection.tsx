import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pilcrow } from "lucide-react";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { api, type EngineStatus, type ModelRef } from "../../../api/hindsight";
import { findAdvanced, patchAdvanced, type AiTask } from "../../../state/reportModel";
import { logError } from "../../../lib/logger";
import { useAiSettings } from "./useAiSettings";
import {
  ENGINE_BATCH_OPTIONS,
  ENGINE_CTX_OPTIONS,
  ENGINE_SLOTS_OPTIONS,
  type EngineBatchKey,
  type EngineCtxKey,
  type EngineSlotsKey,
  engineBatchToOption,
  engineCtxToOption,
  engineOptionToBatch,
  engineOptionToCtx,
  engineOptionToSlots,
  engineSlotsToOption,
  isRecommendedApplied,
  recommendEngineParams,
} from "./engineParams";
import { VramEstimateLine } from "./VramEstimate";

/**
 * Show the engine parameters of the local model a report uses, and a memory estimate.
 *
 * Shows nothing when no task uses a local model.
 */
export function EngineParamsSection() {
  const { t } = useTranslation();
  const { ai, updateAi } = useAiSettings();
  // mount 时拉一次：systemVram + platformId 给"应用推荐"算法用。
  // 后端 OnceLock 缓存，重复调几乎零成本
  const [hwSnapshot, setHwSnapshot] = useState<{
    systemVram: EngineStatus["systemVram"];
    platformId: string | undefined;
  }>({ systemVram: null, platformId: undefined });
  useEffect(() => {
    api
      .getEngineStatus()
      .then((s) => setHwSnapshot({ systemVram: s.systemVram, platformId: s.platformId }))
      .catch((e) => logError("engine.getStatus.hwSnapshot", e));
  }, []);

  // 找一个选中的本地模型来显示/调整高级引擎参数（优先 daily，再看 chat / weekly）
  const localTarget = useMemo<{
    task: AiTask;
    model: Extract<ModelRef, { kind: "local" }>;
  } | null>(() => {
    if (!ai) return null;
    if (ai.picks.daily?.kind === "local" && ai.picks.daily.file.trim()) {
      return { task: "daily", model: { kind: "local", file: ai.picks.daily.file.trim() } };
    }
    if (ai.picks.chat?.kind === "local" && ai.picks.chat.file.trim()) {
      return { task: "chat", model: { kind: "local", file: ai.picks.chat.file.trim() } };
    }
    if (ai.picks.weekly.kind === "local" && ai.picks.weekly.file.trim()) {
      return { task: "weekly", model: { kind: "local", file: ai.picks.weekly.file.trim() } };
    }
    return null;
  }, [ai]);

  const adv = ai && localTarget ? findAdvanced(ai, localTarget.task, localTarget.model) : undefined;
  const summaryBatch = adv?.batchSize ?? null;
  const summarySlots = adv?.parallelSlots ?? null;
  const summaryCtx = adv?.ctxSize ?? null;

  const recommendSummary = useMemo(() => {
    if (!localTarget) return null;
    return recommendEngineParams(
      hwSnapshot.systemVram,
      localTarget.model.file,
      hwSnapshot.platformId,
    );
  }, [localTarget, hwSnapshot]);

  // 聊天起引擎只用上下文；批大小和并发路数后端保存时会清掉，这里也不显示
  const ctxOnly = localTarget?.task === "chat";

  const summaryApplied =
    recommendSummary != null &&
    isRecommendedApplied(recommendSummary, {
      batchSize: ctxOnly ? recommendSummary.batchSize : summaryBatch,
      parallelSlots: ctxOnly ? recommendSummary.parallelSlots : summarySlots,
      ctxSize: summaryCtx,
    });

  const updateEngineParam = (patch: {
    batchSize?: number | null;
    parallelSlots?: number | null;
    ctxSize?: number | null;
  }) => {
    if (!ai || !localTarget) return;
    updateAi({
      advanced: patchAdvanced(ai, localTarget.task, localTarget.model, patch),
    });
  };

  const handleApplySummary = () => {
    if (!recommendSummary) return;
    updateEngineParam(
      ctxOnly
        ? { ctxSize: recommendSummary.ctxSize }
        : {
            batchSize: recommendSummary.batchSize,
            parallelSlots: recommendSummary.parallelSlots,
            ctxSize: recommendSummary.ctxSize,
          },
    );
  };

  if (!ai || !localTarget) return null;

  return (
    <Section title={t("aiSettings.summaryParams.sectionTitle")} icon={Pilcrow}>
      {!ctxOnly && (
        <>
          <Row label={t("aiSettings.engineParams.batch")}>
            <SimplePicker<EngineBatchKey>
              value={engineBatchToOption(summaryBatch)}
              options={ENGINE_BATCH_OPTIONS}
              onChange={(next) => updateEngineParam({ batchSize: engineOptionToBatch(next) })}
            />
          </Row>
          <Row label={t("aiSettings.engineParams.slots")}>
            <SimplePicker<EngineSlotsKey>
              value={engineSlotsToOption(summarySlots)}
              options={ENGINE_SLOTS_OPTIONS}
              onChange={(next) => updateEngineParam({ parallelSlots: engineOptionToSlots(next) })}
            />
          </Row>
        </>
      )}
      <Row label={t("aiSettings.engineParams.ctx")}>
        <SimplePicker<EngineCtxKey>
          value={engineCtxToOption(summaryCtx)}
          options={ENGINE_CTX_OPTIONS}
          onChange={(next) => updateEngineParam({ ctxSize: engineOptionToCtx(next) })}
        />
      </Row>
      <VramEstimateLine
        modelName={localTarget.model.file}
        parallelSlots={summarySlots ?? 1}
        ctxSize={summaryCtx ?? 8192}
        systemVram={hwSnapshot.systemVram}
        recommended={recommendSummary}
        recommendedApplied={summaryApplied}
        onApplyRecommended={handleApplySummary}
      />
    </Section>
  );
}
