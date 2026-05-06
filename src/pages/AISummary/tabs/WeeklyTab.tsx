import { useTranslation } from "react-i18next";
import { PlaceholderTab } from "./PlaceholderTab";

export default function WeeklyTab() {
  const { t } = useTranslation();
  return (
    <PlaceholderTab
      title={t("aiSummary.weekly.title")}
      hint={t("aiSummary.weekly.hint")}
    />
  );
}
