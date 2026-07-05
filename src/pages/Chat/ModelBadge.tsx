import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Cloud, HardDrive } from "lucide-react";
import {
  api,
  SUMMARY_CLOUD_SENTINEL,
  type ModelEntry,
} from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { logError } from "../../lib/logger";
import { chatCloudReady, chatLocalModelName, chatUsesCloud } from "./chatRouting";
import styles from "./ChatPage.module.css";

/**
 * 当前 Chat 模型 badge + 下拉选择器:
 * - 云端 = 琥珀警告色(数据出设备),本地 = 灰;
 * - 点开可切换:云端 API(已配置时)/ 任一本地 GGUF。写入独立的 chat 槽位
 *   (setStepModel "chat"),不影响段总结的模型选择。
 */
export default function ModelBadge() {
  const { t } = useTranslation();
  const { settings, reload } = useSettings();
  const [open, setOpen] = useState(false);
  const [localModels, setLocalModels] = useState<ModelEntry[]>([]);
  const wrapRef = useRef<HTMLSpanElement>(null);

  // 点外面/Esc 关闭菜单
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!settings) return null;
  const ai = settings.ai;
  const cloud = chatUsesCloud(ai);
  const localName = chatLocalModelName(ai);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) {
      // 打开时拉本地模型清单(mmproj 是投影文件,不是可选主模型)
      api
        .listLocalModels()
        .then((all) => setLocalModels(all.filter((m) => !m.isMmproj)))
        .catch((e) => logError("chat.listModels", e));
    }
  };

  const select = async (value: string) => {
    setOpen(false);
    try {
      // 空 = 回自动;sentinel = 云端;文件名 = 本地。chat 纯文本不带 mmproj
      await api.setStepModel("chat", value, null);
      await reload();
    } catch (e) {
      logError("chat.setModel", e);
    }
  };

  return (
    <span ref={wrapRef} className={styles.badgeWrap}>
      <button
        type="button"
        className={`${styles.badge} ${cloud ? styles.badgeCloud : ""}`}
        title={cloud ? ai.endpoint : undefined}
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("chat.model.pickAria")}
      >
        {cloud ? (
          <Cloud size={12} strokeWidth={2.2} />
        ) : (
          <HardDrive size={12} strokeWidth={2.2} />
        )}
        {cloud
          ? t("chat.badge.cloud", { model: ai.model })
          : localName
            ? t("chat.badge.local", { model: localName })
            : t("chat.badge.none")}
        <ChevronDown size={12} strokeWidth={2.2} />
      </button>

      {open && (
        <div className={styles.badgeMenu} role="menu">
          {chatCloudReady(ai) && (
            <button
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => void select(SUMMARY_CLOUD_SENTINEL)}
            >
              <Cloud size={12} strokeWidth={2.2} className={styles.badgeMenuCloudIcon} />
              <span className={styles.badgeMenuLabel}>
                {t("chat.badge.cloud", { model: ai.model })}
              </span>
              {cloud && <Check size={12} strokeWidth={2.4} />}
            </button>
          )}
          {localModels.map((m) => (
            <button
              key={m.filename}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => void select(m.filename)}
            >
              <HardDrive size={12} strokeWidth={2.2} />
              <span className={styles.badgeMenuLabel}>{m.filename}</span>
              {!cloud && localName === m.filename && (
                <Check size={12} strokeWidth={2.4} />
              )}
            </button>
          ))}
          {localModels.length === 0 && !chatCloudReady(ai) && (
            <p className={styles.badgeMenuEmpty}>{t("chat.model.empty")}</p>
          )}
        </div>
      )}
    </span>
  );
}
