import { useTranslation } from "react-i18next";
import styles from "./EmptyHint.module.css";

interface EmptyHintProps {
  /** 自定义文案；不传走 t("common.empty") */
  message?: string;
}

/** 空数据占位文案。抽自 Today/Week/Month 三页内嵌的 EmptyHint 函数（一字不差）。 */
export function EmptyHint({ message }: EmptyHintProps) {
  const { t } = useTranslation();
  return <div className={styles.empty}>{message ?? t("common.empty")}</div>;
}
