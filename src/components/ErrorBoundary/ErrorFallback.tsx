import { useTranslation } from "react-i18next";
import styles from "./ErrorBoundary.module.css";

// 兜底 UI 单独成文件，才能用 useTranslation（class 边界不能用 hook），
// 也避免与 ErrorBoundary 混在一个文件触发 react-refresh 警告。
// i18n 在 main.tsx 以副作用初始化（非 Provider），即便某页崩了这里仍可取到文案。
export function ErrorFallback() {
  const { t } = useTranslation();
  return (
    <div className={styles.wrap} role="alert">
      <h1 className={styles.title}>{t("common.errorBoundary.title")}</h1>
      <p className={styles.message}>{t("common.errorBoundary.message")}</p>
      <button
        type="button"
        className={styles.btn}
        onClick={() => window.location.reload()}
      >
        {t("common.errorBoundary.reload")}
      </button>
    </div>
  );
}
