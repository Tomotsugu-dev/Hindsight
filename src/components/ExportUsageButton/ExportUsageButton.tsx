import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Upload } from "lucide-react";
import {
  ExportUsageDialog,
  type QuickKey,
} from "../ExportUsageDialog/ExportUsageDialog";
import styles from "./ExportUsageButton.module.css";

interface Props {
  /** 预选的导出范围:日统计传 today、周统计传 week、月统计传 month。 */
  defaultQuick?: QuickKey;
}

/**
 * 统计页上的「导出」入口(日 / 周 / 月三页共用)。
 *
 * 为什么加这个:导出功能此前**只**藏在 设置 → 数据 → 导出使用数据 里,用户在
 * 看统计时根本想不到去设置页找它(issue #19 第 3 条)。这里把入口放到数据旁边
 * ——想导出的念头正是看着统计时产生的。
 *
 * 低侵入:复用 PeriodCard 已有的 `rightExtras` 插槽(原本只放 DevicePicker),
 * 不改任何布局;对话框状态自持有,页面只需插一个标签。
 */
export function ExportUsageButton({ defaultQuick }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  return (
    <>
      <button
        type="button"
        className={styles.btn}
        onClick={() => setOpen(true)}
        title={t("settings.data.export.rowLabel")}
        aria-label={t("settings.data.export.rowLabel")}
      >
        <Upload size={14} strokeWidth={2} />
      </button>
      <ExportUsageDialog
        open={open}
        onClose={() => setOpen(false)}
        defaultQuick={defaultQuick}
      />
    </>
  );
}
