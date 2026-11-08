import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../ConfirmDialog/ConfirmDialog";
import { describeCount } from "../../utils/privateCategory";
import type { PrivacyDialogState } from "./usePrivacyDialog";
import styles from "./PrivacyDialog.module.css";

interface PrivacyDialogProps {
  state: PrivacyDialogState | null;
  onConfirm: () => void;
  onCancel: () => void;
}

const K = "categories.private.dialog";

/**
 * 「设为隐私」的确认框：先说会删多少，确认后存设置、删数据。删除失败可以重试。
 * 状态由 `usePrivacyDialog` 管。
 */
export function PrivacyDialog({ state, onConfirm, onCancel }: PrivacyDialogProps) {
  const { t } = useTranslation();
  if (!state) return null;
  const { step } = state;

  let lines: string[];
  let confirmLabel = t(`${K}.retry`);
  if (step.kind === "ask") {
    const { count } = step;
    const empty = count.kind === "done" && count.count.activities === 0;
    lines = [
      t(state.site ? `${K}.siteEffect` : `${K}.appEffect`),
      count.kind === "loading"
        ? t(`${K}.counting`)
        : count.kind === "failed"
          ? t(`${K}.countFailed`, { error: count.error })
          : empty
            ? t(`${K}.noData`)
            : t(`${K}.willDelete`, { data: describeCount(t, count.count) }),
      t(`${K}.everyDevice`),
      ...(state.site ? [t(`${K}.siteLimit`), t(`${K}.sitePermission`)] : []),
    ];
    confirmLabel = t(empty ? `${K}.makePrivate` : `${K}.deleteAndMakePrivate`);
  } else if (step.kind === "deleting") {
    lines = [t(`${K}.waitForSync`)];
    confirmLabel = t(`${K}.deleting`);
  } else if (step.kind === "saveFailed") {
    lines = [t(`${K}.saveFailed`, { error: step.error })];
  } else {
    lines = [t(`${K}.purgeFailed`, { error: step.error })];
  }

  return (
    <ConfirmDialog
      open
      title={state.name === null ? t(`${K}.titleNoName`) : t(`${K}.title`, { name: state.name })}
      message={
        <div className={styles.lines}>
          {lines.map((line) => (
            <p key={line}>{line}</p>
          ))}
        </div>
      }
      confirmLabel={confirmLabel}
      cancelLabel={step.kind === "purgeFailed" ? t("common.close") : undefined}
      variant="danger"
      confirmDisabled={step.kind === "ask" && step.count.kind === "loading"}
      busy={step.kind === "deleting"}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  );
}
