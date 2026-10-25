import i18n from "@app/i18n";
import styles from "./PreviewError.module.css";

// 只有 demo 用的提示，不放进应用的语言文件
const MESSAGES: Record<string, string> = {
  "zh-CN": "读不到这个主题",
  "zh-TW": "讀不到這個主題",
  en: "Cannot load this theme",
  ja: "このテーマを読み込めません",
  "pt-BR": "Não foi possível carregar este tema",
  es: "No se pudo cargar este tema",
};

/** ?theme= 的主题读不到时，盖在 demo 顶上的一行提示。 */
export function PreviewError() {
  const message = MESSAGES[i18n.resolvedLanguage ?? "en"] ?? MESSAGES.en;
  return (
    <div className={styles.banner} role="alert">
      {message}
    </div>
  );
}
