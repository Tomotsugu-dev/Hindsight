// AI 总结的「调试设置」tab——参数配置面板，跟「调试」tab 平级。
//
// 含 4 个 Section：过滤分类 / 抽帧参数 / 引擎参数 / 云端 API。
// state 来自 DebugStateContext，跟 DebugTab 共享同一份；用户在这里改值、
// 切到「调试」tab 跑总结。helper functions（picker 选项映射、VramEstimateLine）
// 跟 DebugTab 共用 [DebugTab.tsx](./DebugTab.tsx) 的 export。

import { useMemo } from "react";
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
  VramEstimateLine,
  type BatchKey,
  type CtxKey,
  type SlotsKey,
} from "./DebugTab";
import styles from "./DebugTab.module.css";

export default function DebugSettingsTab() {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const activeMain = settings?.ai.activeMain ?? "";

  const SLOTS_OPTIONS = useMemo(() => buildSlotsOptions(t), [t]);
  const CTX_OPTIONS = useMemo(() => buildCtxOptions(t), [t]);

  const {
    debugExcluded,
    setDebugExcluded,
    debugHashThreshold,
    setDebugHashThreshold,
    debugHashWindow,
    setDebugHashWindow,
    debugBatchSize,
    setDebugBatchSize,
    debugParallelSlots,
    setDebugParallelSlots,
    debugCtxSize,
    setDebugCtxSize,
    debugExternalEnabled,
    setDebugExternalEnabled,
  } = useDebugState();

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
        title={t("aiSummary.debug.engine.title")}
        icon={Server}
        description={t("aiSummary.debug.engine.description")}
      >
        <div className={styles.engineParamRow}>
          <SimplePicker<BatchKey>
            value={batchToOption(debugBatchSize)}
            options={BATCH_OPTIONS}
            onChange={(next) => setDebugBatchSize(optionToBatch(next))}
          />
          <SimplePicker<SlotsKey>
            value={slotsToOption(debugParallelSlots)}
            options={SLOTS_OPTIONS}
            onChange={(next) => setDebugParallelSlots(optionToSlots(next))}
          />
          <SimplePicker<CtxKey>
            value={ctxToOption(debugCtxSize)}
            options={CTX_OPTIONS}
            onChange={(next) => setDebugCtxSize(optionToCtx(next))}
          />
        </div>
        <VramEstimateLine
          modelName={activeMain}
          parallelSlots={debugParallelSlots}
          ctxSize={debugCtxSize ?? 8192}
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
