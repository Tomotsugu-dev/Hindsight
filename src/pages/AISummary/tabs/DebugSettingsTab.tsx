// AI 总结的「调试设置」tab——参数配置面板，跟「调试」tab 平级。
//
// 含 4 个 Section：过滤分类 / 抽帧参数 / 引擎参数 / 云端 API。
// state 来自 DebugStateContext，跟 DebugTab 共享同一份；用户在这里改值、
// 切到「调试」tab 跑总结。picker 选项映射来自纯 TS 工具文件
// [debugTabOptions.ts](./debugTabOptions.ts)；VramEstimateLine 直接复用
// AISettings 共享版（公式 + 系统 VRAM 红绿灯都在那边维护一处）。

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Cloud, Filter, Image as ImageIcon, Server } from "lucide-react";
import { useDebugState } from "../DebugStateContext";
import { Section } from "../../../components/FormLayout/Section";
import { Row } from "../../../components/FormLayout/Row";
import { Toggle } from "../../../components/FormControls/Toggle";
import { Slider } from "../../../components/FormControls/Slider";
import { CategoryChipMultiSelect } from "../../../components/FormControls/CategoryChipMultiSelect";
import { SimplePicker } from "../../../components/SimplePicker/SimplePicker";
import { useSettings } from "../../../state/settings";
import { api, type EngineStatus } from "../../../api/hindsight";
import { logError } from "../../../lib/logger";
import {
  BATCH_OPTIONS,
  buildCtxOptions,
  buildSlotsOptions,
  batchToOption,
  ctxToOption,
  slotsToOption,
  optionToBatch,
  optionToCtx,
  optionToSlots,
  type BatchKey,
  type CtxKey,
  type SlotsKey,
} from "./debugTabOptions";
import { VramEstimateLine } from "../../AISettings/shared/VramEstimate";
import {
  isRecommendedApplied,
  recommendEngineParams,
} from "../../AISettings/shared/engineParams";
import styles from "./DebugTab.module.css";

export default function DebugSettingsTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const activeMain = settings?.ai.activeMain ?? "";

  // 系统 VRAM + platformId：mount 时拉一次（后端 OnceLock 缓存，重复调几乎零成本），
  // 给 VramEstimateLine 的"绿/橙/红 OOM 风险"和"应用推荐"按钮算法用
  const [hwSnapshot, setHwSnapshot] = useState<{
    systemVram: EngineStatus["systemVram"];
    platformId: string | undefined;
  }>({ systemVram: null, platformId: undefined });
  useEffect(() => {
    api
      .getEngineStatus()
      .then((s) =>
        setHwSnapshot({ systemVram: s.systemVram, platformId: s.platformId }),
      )
      .catch((e) => logError("debugSettings.getStatus.hwSnapshot", e));
  }, []);

  const SLOTS_OPTIONS = useMemo(() => buildSlotsOptions(t), [t]);
  const CTX_OPTIONS = useMemo(() => buildCtxOptions(t), [t]);

  const {
    debugExcluded,
    setDebugExcluded,
    debugHashThreshold,
    setDebugHashThreshold,
    debugHashWindow,
    setDebugHashWindow,
    debugDescribeBatchSize,
    setDebugDescribeBatchSize,
    debugDescribeParallelSlots,
    setDebugDescribeParallelSlots,
    debugDescribeCtxSize,
    setDebugDescribeCtxSize,
    debugSummaryBatchSize,
    setDebugSummaryBatchSize,
    debugSummaryParallelSlots,
    setDebugSummaryParallelSlots,
    debugSummaryCtxSize,
    setDebugSummaryCtxSize,
    debugExternalEnabled,
    setDebugExternalEnabled,
  } = useDebugState();

  // 双套推荐——跟 EngineTab 一致；写到 DebugStateContext 的本地 state，不动 settings.ai
  const recommendDescribe = useMemo(
    () =>
      recommendEngineParams(
        hwSnapshot.systemVram,
        activeMain,
        hwSnapshot.platformId,
        "describe",
      ),
    [hwSnapshot, activeMain],
  );
  const recommendSummary = useMemo(
    () =>
      recommendEngineParams(
        hwSnapshot.systemVram,
        activeMain,
        hwSnapshot.platformId,
        "summary",
      ),
    [hwSnapshot, activeMain],
  );
  const describeApplied = isRecommendedApplied(recommendDescribe, {
    batchSize: debugDescribeBatchSize,
    parallelSlots: debugDescribeParallelSlots,
    ctxSize: debugDescribeCtxSize,
  });
  const summaryApplied = isRecommendedApplied(recommendSummary, {
    batchSize: debugSummaryBatchSize,
    parallelSlots: debugSummaryParallelSlots,
    ctxSize: debugSummaryCtxSize,
  });
  const handleApplyDescribe = () => {
    setDebugDescribeBatchSize(recommendDescribe.batchSize);
    setDebugDescribeParallelSlots(recommendDescribe.parallelSlots);
    setDebugDescribeCtxSize(recommendDescribe.ctxSize);
  };
  const handleApplySummary = () => {
    setDebugSummaryBatchSize(recommendSummary.batchSize);
    setDebugSummaryParallelSlots(recommendSummary.parallelSlots);
    setDebugSummaryCtxSize(recommendSummary.ctxSize);
  };

  return (
    <div className={styles.wrap}>
      <Section
        title={t("aiSummary.debug.filter.title")}
        icon={Filter}
        description={t("aiSummary.debug.filter.description")}
      >
        <Row
          label={t("aiSummary.debug.filter.categoriesLabel")}
          labelHint={t("aiSummary.debug.filter.categoriesHint")}
          block
        >
          <CategoryChipMultiSelect
            selectedIds={debugExcluded}
            onChange={setDebugExcluded}
          />
        </Row>
      </Section>

      <Section
        title={t("aiSummary.debug.frame.title")}
        icon={ImageIcon}
        description={t("aiSummary.debug.frame.description")}
      >
        <Row
          label={t("aiSummary.debug.frame.hashThresholdLabel")}
          labelHint={t("aiSummary.debug.frame.hashThresholdHint")}
        >
          <Slider
            value={debugHashThreshold}
            onChange={setDebugHashThreshold}
            min={0}
            max={32}
            step={1}
          />
        </Row>
        <Row
          label={t("aiSummary.debug.frame.hashWindowLabel")}
          labelHint={t("aiSummary.debug.frame.hashWindowHint")}
        >
          <Slider
            value={debugHashWindow}
            onChange={setDebugHashWindow}
            min={0}
            max={30}
            step={1}
            suffix={t("aiSummary.debug.frame.hashWindowSuffix")}
          />
        </Row>
      </Section>

      <Section
        title={t("aiSummary.debug.describeEngine.title")}
        icon={Server}
      >
        <div className={styles.engineParamRow}>
          <SimplePicker<BatchKey>
            value={batchToOption(debugDescribeBatchSize)}
            options={BATCH_OPTIONS}
            onChange={(next) => setDebugDescribeBatchSize(optionToBatch(next))}
          />
          <SimplePicker<SlotsKey>
            value={slotsToOption(debugDescribeParallelSlots)}
            options={SLOTS_OPTIONS}
            onChange={(next) => setDebugDescribeParallelSlots(optionToSlots(next))}
          />
          <SimplePicker<CtxKey>
            value={ctxToOption(debugDescribeCtxSize)}
            options={CTX_OPTIONS}
            onChange={(next) => setDebugDescribeCtxSize(optionToCtx(next))}
          />
        </div>
        <VramEstimateLine
          modelName={activeMain}
          parallelSlots={debugDescribeParallelSlots}
          ctxSize={debugDescribeCtxSize ?? 8192}
          systemVram={hwSnapshot.systemVram}
          recommended={recommendDescribe}
          recommendedApplied={describeApplied}
          onApplyRecommended={handleApplyDescribe}
        />
      </Section>

      <Section
        title={t("aiSummary.debug.summaryEngine.title")}
        icon={Server}
      >
        <div className={styles.engineParamRow}>
          <SimplePicker<BatchKey>
            value={batchToOption(debugSummaryBatchSize)}
            options={BATCH_OPTIONS}
            onChange={(next) => setDebugSummaryBatchSize(optionToBatch(next))}
          />
          <SimplePicker<SlotsKey>
            value={slotsToOption(debugSummaryParallelSlots)}
            options={SLOTS_OPTIONS}
            onChange={(next) => setDebugSummaryParallelSlots(optionToSlots(next))}
          />
          <SimplePicker<CtxKey>
            value={ctxToOption(debugSummaryCtxSize)}
            options={CTX_OPTIONS}
            onChange={(next) => setDebugSummaryCtxSize(optionToCtx(next))}
          />
        </div>
        <VramEstimateLine
          modelName={activeMain}
          parallelSlots={debugSummaryParallelSlots}
          ctxSize={debugSummaryCtxSize ?? 8192}
          systemVram={hwSnapshot.systemVram}
          recommended={recommendSummary}
          recommendedApplied={summaryApplied}
          onApplyRecommended={handleApplySummary}
        />
      </Section>

      <Section
        title={t("aiSummary.debug.cloudApi.title")}
        icon={Cloud}
        description={t("aiSummary.debug.cloudApi.description")}
      >
        <Row
          label={t("aiSummary.debug.cloudApi.enableLabel")}
          description={t("aiSummary.debug.cloudApi.enableHint")}
        >
          <Toggle
            checked={debugExternalEnabled}
            onChange={setDebugExternalEnabled}
          />
        </Row>
      </Section>
    </div>
  );
}
