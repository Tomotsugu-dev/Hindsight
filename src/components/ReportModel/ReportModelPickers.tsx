import { useTranslation } from "react-i18next";
import { Brain, Cloud, HardDrive, Link2 } from "lucide-react";
import { hasLevels, thinkingLabelKey } from "../../lib/thinkingModes";
import { useReportModel, type ReportModule } from "../../state/reportModel";
import { InlinePicker } from "../InlinePicker/InlinePicker";
import { ReportCloudConfirm } from "./ReportCloudConfirm";
import { checkedModel, modelOptions, routeName } from "./reportModelOptions";

const KIND_ICON = { follow: Link2, cloud: Cloud, local: HardDrive } as const;

/**
 * 报告页上的两个小下拉框：用哪个模型、什么思考模式。选了马上存进设置，自动生成也用。
 * 和 AI 设置 → 模型 顶部的是同一个设置。
 */
export function ReportModelPickers({
  module,
  disabled,
}: {
  module: ReportModule;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const state = useReportModel(module);
  if (!state) return null;

  const models = modelOptions(state, module, t).map((o) => ({
    value: o.value,
    label: o.label,
    detail: o.detail,
    icon: KIND_ICON[o.kind],
    cloud: o.kind === "cloud",
    title: o.kind === "local" ? o.value : undefined,
  }));
  const thinkingNote = hasLevels(state.modes)
    ? "chat.thinkingMode.hintLevels"
    : "chat.thinkingMode.hint";

  return (
    <>
      <InlinePicker
        value={checkedModel(state, module)}
        options={models}
        onChange={state.chooseModel}
        label={routeName(state.route, t)}
        icon={state.route.cloud ? Cloud : HardDrive}
        cloud={Boolean(state.route.cloud)}
        title={t("aiSummary.model.modelGroup")}
        note={models.length === 0 ? t("chat.model.empty") : undefined}
        disabled={disabled}
        onOpen={state.refreshLocal}
      />
      <InlinePicker
        value={state.thinking}
        options={state.modes.map((m) => ({ value: m, label: t(thinkingLabelKey(m, state.modes)) }))}
        onChange={state.chooseThinking}
        label={t(thinkingLabelKey(state.thinking, state.modes))}
        icon={Brain}
        title={t("chat.thinkingMode.label")}
        note={t(thinkingNote)}
        disabled={disabled}
      />
      <ReportCloudConfirm state={state} />
    </>
  );
}
