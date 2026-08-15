import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { EyeOff, X } from "lucide-react";
import { api, type IgnoreRule } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { displayAppName } from "../../utils/displayName";
import { logError } from "../../lib/logger";
import categoriesStyles from "../Categories/Categories.module.css";
import styles from "./IgnoredWindows.module.css";

/**
 * 「已忽略的窗口」管理区：列出全部 ignore 规则（进程 + 标题关键词），可移除。
 *
 * 规则在 AppDetailDrawer 的标题行就地创建；这里是唯一的回看与撤销全集——
 * 就地创建的东西必须有地方看到全部，否则统计悄悄少一块、查不出原因。
 * 挂在应用页与「隐藏应用」共处一页：两者是同一语义（不计入统计）的两个粒度。
 * 没有规则时整个区块不渲染（多数用户永远用不到，不占版面）。
 */
export function IgnoredWindowsSection() {
  const { t } = useTranslation();
  const [rules, setRules] = useState<IgnoreRule[] | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void api
      .getSettings()
      .then((s) => {
        if (!cancelled) setRules(s.ignoreRules ?? []);
      })
      .catch((e) => {
        logError("apps.ignored.load", e);
        if (!cancelled) setRules([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const remove = useCallback(
    async (rule: IgnoreRule) => {
      if (busy) return;
      setBusy(true);
      try {
        const res = await api.removeIgnoreRule(
          rule.processName,
          rule.titleKeyword,
        );
        setRules(res.rules);
        setNotice(t("apps.ignored.removed", { count: res.reappliedRows }));
        window.setTimeout(() => setNotice(null), 3500);
      } catch (e) {
        logError("apps.ignored.remove", e);
      } finally {
        setBusy(false);
      }
    },
    [busy, t],
  );

  if (rules === null || rules.length === 0) return null;

  return (
    <section className={categoriesStyles.card}>
      <header className={styles.head}>
        <EyeOff size={15} strokeWidth={2} aria-hidden />
        <span className={styles.title}>{t("apps.ignored.title")}</span>
        <span className={styles.hint}>{t("apps.ignored.hint")}</span>
      </header>
      {notice && (
        <div className={styles.notice} role="status">
          {notice}
        </div>
      )}
      <ul className={styles.list}>
        {rules.map((r) => (
          <li
            key={`${r.processName}|${r.titleKeyword ?? ""}`}
            className={styles.row}
          >
            <AppIcon
              processName={r.processName}
              fallbackColor="#94a3b8"
              size={20}
            />
            <span className={styles.proc}>{displayAppName(r.processName)}</span>
            <span className={styles.kw} title={r.titleKeyword ?? undefined}>
              {r.titleKeyword ?? t("apps.ignored.wholeApp")}
            </span>
            <button
              type="button"
              className={styles.removeBtn}
              disabled={busy}
              aria-label={t("apps.ignored.remove")}
              title={t("apps.ignored.remove")}
              onClick={() => void remove(r)}
            >
              <X size={14} strokeWidth={2} />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
