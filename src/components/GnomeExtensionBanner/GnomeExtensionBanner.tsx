import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Puzzle } from "lucide-react";
import { api, type GnomeExtensionStatus } from "../../api/hindsight";
import { logError } from "../../lib/logger";
import styles from "./GnomeExtensionBanner.module.css";

/**
 * Hindsight 的 GNOME 扩展没在运行时的提示条。只有 Linux 上有查扩展的命令，所以只在 Linux
 * 上查；Linux 上只有 GNOME Wayland 要靠扩展记录，其他桌面后端返回 ready，这里不显示。
 * 扩展没打开时给「打开扩展」按钮，用户点了才打开；其余情况只说明怎么办。挂在 AppLayout，
 * 任何页面都看得到。窗口重新获得焦点时再查一次：用户可能刚在「扩展」应用里改过。
 */
export function GnomeExtensionBanner() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<GnomeExtensionStatus>("ready");
  const [enabling, setEnabling] = useState(false);
  const [errMsg, setErrMsg] = useState("");

  const refresh = useCallback(() => {
    api
      .getGnomeExtensionStatus()
      .then(setStatus)
      .catch((e) => logError("gnomeExtension.status", e));
  }, []);

  useEffect(() => {
    if (document.body.dataset.platform !== "linux") return;
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refresh]);

  if (status === "ready") return null;

  const enable = async () => {
    setEnabling(true);
    setErrMsg("");
    try {
      setStatus(await api.enableGnomeExtension());
    } catch (e) {
      logError("gnomeExtension.enable", e);
      setErrMsg(String(e));
    } finally {
      setEnabling(false);
    }
  };

  return (
    <div className={styles.wrap}>
      <div className={styles.banner} role="status">
        <Puzzle size={14} strokeWidth={2} className={styles.icon} />
        <span className={styles.text}>
          {errMsg
            ? t("components.gnomeExtension.enableFailed", { msg: errMsg })
            : t(`components.gnomeExtension.${status}`)}
        </span>
        {status === "off" && (
          <button
            type="button"
            className={styles.btn}
            onClick={() => void enable()}
            disabled={enabling}
          >
            {enabling
              ? t("components.gnomeExtension.enabling")
              : t("components.gnomeExtension.enable")}
          </button>
        )}
      </div>
    </div>
  );
}
