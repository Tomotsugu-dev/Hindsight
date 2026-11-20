import { useTranslation } from "react-i18next";
import { ListFilter } from "lucide-react";
import type { AiProfilesState } from "../../state/aiProfiles";
import type { ControlSize } from "../Button/Button";
import { SimplePicker } from "../SimplePicker/SimplePicker";

interface Props {
  state: AiProfilesState;
  disabled?: boolean;
  size?: ControlSize;
}

/** 下拉框里「默认分类」的值；配置 id 都以 p_ 开头，不会撞 */
const MAIN = "";

/** 报告页工具栏上的「AI 能看到的数据」下拉框：默认分类和各套配置。配置在 AI 设置 → 常规 里改。 */
export function AiProfilePicker({ state, disabled, size = "lg" }: Props) {
  const { t } = useTranslation();
  const options = [
    { value: MAIN, label: t("aiProfiles.main") },
    ...state.list.map(([id, p]) => ({ value: id, label: p.name })),
  ];

  return (
    <SimplePicker
      value={state.selected ?? MAIN}
      options={options}
      onChange={(v) => state.select(v === MAIN ? null : v)}
      disabled={disabled}
      icon={ListFilter}
      size={size}
      title={t("aiProfiles.pickerTitle")}
    />
  );
}
