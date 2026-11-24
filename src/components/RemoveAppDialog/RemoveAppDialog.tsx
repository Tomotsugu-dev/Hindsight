import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Trans, useTranslation } from "react-i18next";
import { EyeOff, Trash2 } from "lucide-react";
import { api, type DataCount, type PurgeTarget } from "../../api/hindsight";
import { Button } from "../Button/Button";
import { ChartOffIcon } from "../icons/ChartOffIcon";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { logError } from "../../lib/logger";
import { describeCount, sumCounts } from "../../utils/privateCategory";
import styles from "./RemoveAppDialog.module.css";

/** Which texts the options use: one app, several apps, or websites */
export type RemoveTexts = "app" | "apps" | "sites";

const DELETE_DESC: Record<RemoveTexts, string> = {
  app: "apps.remove.deleteDesc",
  apps: "apps.remove.batch.deleteDescApps",
  sites: "apps.remove.batch.deleteDescSites",
};

interface RemoveAppDialogProps {
  open: boolean;
  title: string;
  /** What the delete options delete, to count how much */
  targets: PurgeTarget[];
  texts: RemoveTexts;
  onDelete: () => void;
  onMakePrivate: () => void;
  onExclude: () => void;
  onCancel: () => void;
}

/**
 * 从应用列表移除应用或网站时的选择弹窗，可以一次移除几个。三条路后果不同，说明写在正文里，不推荐哪一条。
 * 说明和按钮都按从轻到重排：
 *   - 不计入统计：归入内置的 `hidden` 分类，照常记录，只是不算进统计；
 *   - 删除：删掉已有的数据，不可逆。这个程序下次运行时还会出现在列表里；
 *   - 设为隐私：删掉已有的数据，以后也不记录。不可逆。
 * 后两条都删数据，按钮和图标是红的；打开时焦点在「取消」上。
 *
 * **刻意不做「不再提示」**：记住「删除」意味着以后每次点行尾按钮都直接不可逆
 * 地删数据，省下的一次确认远不值这个风险。每次都问。
 */
export function RemoveAppDialog({
  open,
  title,
  targets,
  texts,
  onDelete,
  onMakePrivate,
  onExclude,
  onCancel,
}: RemoveAppDialogProps) {
  const { t } = useTranslation();
  // null：还在统计，或者统计出错
  const [count, setCount] = useState<DataCount | null>(null);

  // The caller passes a new array on each render, so count again only when its content changes
  const targetsKey = JSON.stringify(targets);
  useEffect(() => {
    if (!open) return;
    setCount(null);
    let live = true;
    const list = JSON.parse(targetsKey) as PurgeTarget[];
    void Promise.all(list.map((target) => api.countTargetData(target))).then(
      (counts) => {
        if (live) setCount(sumCounts(counts));
      },
      (e) => logError("removeApp.count", e),
    );
    return () => {
      live = false;
    };
  }, [open, targetsKey]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onCancel]);

  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(open, dialogRef);

  if (!open) return null;
  const data = describeCount(t, count);

  return createPortal(
    <div className={styles.backdrop} onMouseDown={onCancel} role="presentation">
      {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
      <div
        ref={dialogRef}
        className={styles.dialog}
        data-part="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="remove-app-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h2 id="remove-app-title" className={styles.title}>
          {title}
        </h2>

        <ul className={styles.options}>
          <li className={styles.option}>
            <ChartOffIcon size={15} strokeWidth={2} className={styles.optionIcon} />
            <div>
              <p className={styles.optionName}>{t("apps.remove.excludeName")}</p>
              <p className={styles.optionDesc}>
                {t(texts === "app" ? "apps.remove.excludeDesc" : "apps.remove.batch.excludeDesc")}
              </p>
            </div>
          </li>
          <li className={styles.option}>
            <Trash2
              size={15}
              strokeWidth={2}
              className={`${styles.optionIcon} ${styles.optionIconDanger}`}
            />
            <div>
              <p className={styles.optionName}>{t("apps.remove.deleteName")}</p>
              <p className={styles.optionDesc}>
                <Trans i18nKey={DELETE_DESC[texts]} values={{ data }} components={{ b: <b /> }} />
              </p>
            </div>
          </li>
          <li className={styles.option}>
            <EyeOff
              size={15}
              strokeWidth={2}
              className={`${styles.optionIcon} ${styles.optionIconDanger}`}
            />
            <div>
              <p className={styles.optionName}>{t("apps.remove.privateName")}</p>
              <p className={styles.optionDesc}>
                <Trans
                  i18nKey="apps.remove.privateDesc"
                  values={{ data }}
                  components={{ b: <b /> }}
                />
              </p>
            </div>
          </li>
        </ul>

        <div className={styles.actions}>
          <Button
            onClick={onCancel}
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus
          >
            {t("common.cancel")}
          </Button>
          <Button icon={ChartOffIcon} onClick={onExclude}>
            {t("apps.remove.excludeAction")}
          </Button>
          {/* 删除、设为隐私都删数据:不给实心红(那会把视线拉过去),用描边警示 */}
          <Button variant="outline" tone="danger" icon={Trash2} onClick={onDelete}>
            {t("apps.remove.deleteAction")}
          </Button>
          <Button variant="outline" tone="danger" icon={EyeOff} onClick={onMakePrivate}>
            {t("apps.remove.privateAction")}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
