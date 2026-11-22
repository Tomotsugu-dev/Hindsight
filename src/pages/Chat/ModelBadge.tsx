import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Cloud, HardDrive } from "lucide-react";
import { api, type CloudConnection, type ModelEntry, type ModelRef } from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { profileLabel } from "../../utils/profileLabel";
import { logError } from "../../lib/logger";
import { chatCloudConnection, chatLocalModelName, readyCloudConnections } from "./chatRouting";
import styles from "./ChatPage.module.css";

/**
 * 当前 Chat 模型 badge + 下拉选择器：
 * - 云端 = 琥珀警告色（数据出设备），本地 = 灰；
 * - 点开可在任意可用云端连接与支持工具调用的本地 GGUF 之间切换，
 *   只改 `picks.chat`，不影响日报和周报的模型选择。
 */
export default function ModelBadge() {
  const { t } = useTranslation();
  const { settings, reload } = useSettings();
  const [open, setOpen] = useState(false);
  const [localModels, setLocalModels] = useState<ModelEntry[]>([]);
  const [hasToollessOnly, setHasToollessOnly] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);

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
  const activeCloud = chatCloudConnection(ai);
  const localName = chatLocalModelName(ai);
  const cloudList = readyCloudConnections(ai);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) {
      api
        .listLocalModels()
        .then((all) => {
          const mains = all.filter((m) => !m.isMmproj);
          const usable = mains.filter((m) => m.supportsTools !== false);
          setLocalModels(usable);
          setHasToollessOnly(usable.length === 0 && mains.length > 0);
        })
        .catch((e) => logError("chat.listModels", e));
    }
  };

  const selectModel = async (model: ModelRef) => {
    setOpen(false);
    try {
      await api.setTaskModel("chat", model);
      await reload();
    } catch (e) {
      logError("chat.setModel", e);
    }
  };

  const selectCloud = (conn: CloudConnection) => {
    if (activeCloud?.id === conn.id) {
      setOpen(false);
      return;
    }
    void selectModel({ kind: "cloud", connection: conn.id });
  };

  return (
    <span ref={wrapRef} className={styles.badgeWrap}>
      <button
        type="button"
        className={`${styles.badge} ${activeCloud ? styles.badgeCloud : ""}`}
        title={activeCloud?.endpoint}
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("chat.model.pickAria")}
      >
        {activeCloud ? (
          <Cloud size={12} strokeWidth={2.2} />
        ) : (
          <HardDrive size={12} strokeWidth={2.2} />
        )}
        {activeCloud
          ? t("chat.badge.cloud", { model: activeCloud.model })
          : localName
            ? t("chat.badge.local", { model: localName })
            : t("chat.badge.none")}
        <ChevronDown size={12} strokeWidth={2.2} />
      </button>

      {open && (
        <div className={styles.badgeMenu} role="menu">
          {cloudList.map((conn) => (
            <button
              key={conn.id}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              title={conn.endpoint}
              onClick={() => selectCloud(conn)}
            >
              <Cloud size={12} strokeWidth={2.2} className={styles.badgeMenuCloudIcon} />
              <span className={styles.badgeMenuLabel}>{profileLabel(conn)}</span>
              {activeCloud?.id === conn.id && <Check size={12} strokeWidth={2.4} />}
            </button>
          ))}
          {localModels.map((m) => (
            <button
              key={m.filename}
              type="button"
              role="menuitem"
              className={styles.badgeMenuItem}
              onClick={() => void selectModel({ kind: "local", file: m.filename })}
            >
              <HardDrive size={12} strokeWidth={2.2} />
              <span className={styles.badgeMenuLabel}>{m.filename}</span>
              {!activeCloud && localName === m.filename && <Check size={12} strokeWidth={2.4} />}
            </button>
          ))}
          {localModels.length === 0 && cloudList.length === 0 && (
            <p className={styles.badgeMenuEmpty}>
              {hasToollessOnly ? t("chat.model.noToolUseModels") : t("chat.model.empty")}
            </p>
          )}
        </div>
      )}
    </span>
  );
}
