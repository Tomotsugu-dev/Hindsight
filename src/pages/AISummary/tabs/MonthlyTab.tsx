import { useTranslation } from "react-i18next";
import { PlaceholderTab } from "./PlaceholderTab";

export default function MonthlyTab() {
  const { t } = useTranslation();
  return (
    <PlaceholderTab
      title={t("aiSummary.monthly.title")}
      hint={t("aiSummary.monthly.hint")}
    />
  );
}
