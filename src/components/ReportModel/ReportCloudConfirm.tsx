import { useTranslation } from "react-i18next";
import type { ReportModelState } from "../../state/reportModel";
import { ConfirmDialog } from "../ConfirmDialog/ConfirmDialog";

/** 报告换到云端前的确认框：说清楚会发出去什么。 */
export function ReportCloudConfirm({ state }: { state: ReportModelState }) {
  const { t } = useTranslation();
  const provider = state.pendingCloudConnection?.dialect || state.route.cloud?.dialect || "cloud";
  return (
    <ConfirmDialog
      open={state.askingCloud}
      title={t("aiSettings.models.cloud.summaryConfirmTitle")}
      message={t("aiSummary.model.cloudConfirmMessage", { provider })}
      confirmLabel={t("aiSettings.models.cloud.summaryConfirmAccept")}
      variant="primary"
      onConfirm={state.confirmCloud}
      onCancel={state.cancelCloud}
    />
  );
}
