import { useState } from "react";
import { useTranslation } from "react-i18next";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  Activity,
  Check,
  Cloud,
  Download,
  FileBox,
  FolderOpen,
  Loader2,
  Pencil,
  Trash2,
  XCircle,
} from "lucide-react";
import { Section } from "../../../../components/FormLayout/Section";
import { Button } from "../../../../components/Button/Button";
import { ConfirmDialog } from "../../../../components/ConfirmDialog/ConfirmDialog";
import { RowMenu } from "../../../../components/RowMenu/RowMenu";
import { api, type CloudConnection, type TaskConnectionTest } from "../../../../api/hindsight";
import { isCloudConnectionReady } from "../../../Chat/chatRouting";
import { useAiSettings } from "../../../AISettings/shared/useAiSettings";
import { profileLabel } from "../../../../utils/profileLabel";
import { localizeAiError } from "../../../../lib/aiErrors";
import { logError } from "../../../../lib/logger";
import type { AiTask } from "../../../../state/reportModel";
import type { LocalItem } from "./localFiles";
import type { LocalModels } from "./useLocalModels";
import { tasksUsing } from "./usage";
import { gb } from "./catalog";
import { CapTags, ModelLogo } from "./modelBits";
import legacy from "../../../AISettings/AISettings.module.css";
import s from "./aiModels.module.css";

type TestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; items: TaskConnectionTest[] }
  | { kind: "fail"; message: string };

const TASK_LABEL: Record<AiTask, string> = {
  daily: "aiSummary.tabs.daily",
  weekly: "aiSummary.tabs.week",
  chat: "nav.items.chat",
};

interface MyModelsSectionProps {
  models: LocalModels;
  /** Reads the engine's state again after a test started and stopped it. */
  refreshEngine: () => Promise<void>;
  onEditCloud: (id: string) => void;
  onDeleteCloud: (id: string) => void;
  onAddCloud: () => void;
  onGoToCatalog: () => void;
}

/**
 * The models that can be used now: downloaded files first, then cloud API connections.
 *
 * Each row shows which tasks use the model. Choosing them is in AI settings → Models; here the
 * user tests, reveals, edits and deletes.
 */
export function MyModelsSection({
  models,
  refreshEngine,
  onEditCloud,
  onDeleteCloud,
  onAddCloud,
  onGoToCatalog,
}: MyModelsSectionProps) {
  const { t } = useTranslation();
  const { ai } = useAiSettings();
  const [test, setTest] = useState<TestState>({ kind: "idle" });
  const [deleting, setDeleting] = useState<LocalItem | null>(null);
  const [deletingCloud, setDeletingCloud] = useState<CloudConnection | null>(null);

  if (!ai) return null;

  const empty = models.items.length === 0 && ai.cloud.length === 0;
  const taskNames = (tasks: AiTask[]) =>
    tasks.map((task) => t(TASK_LABEL[task])).join(t("settings.aiModels.mine.listSep"));

  const runTest = async () => {
    setTest({ kind: "running" });
    try {
      setTest({ kind: "done", items: await api.testAiConnections() });
    } catch (e) {
      setTest({ kind: "fail", message: e instanceof Error ? e.message : String(e) });
    } finally {
      // 测试会拉起本地引擎再停掉
      await refreshEngine();
    }
  };

  const usage = (tasks: AiTask[]) =>
    tasks.length > 0 ? (
      tasks.map((task) => (
        <span key={task} className={s.use}>
          {t(TASK_LABEL[task])}
        </span>
      ))
    ) : (
      <span className={s.unused}>{t("settings.aiModels.mine.unused")}</span>
    );

  const localRow = (item: LocalItem) => {
    const main = item.kind === "orphanMmproj" ? item.file : item.main;
    const mmproj = item.kind === "orphanMmproj" ? null : item.mmproj;
    const name = item.kind === "recommended" ? item.rec.displayName : main.filename;
    const size = gb(main.sizeBytes + (mmproj?.sizeBytes ?? 0));
    const tasks =
      item.kind === "orphanMmproj"
        ? []
        : tasksUsing(ai.picks, { kind: "local", file: main.filename });
    const meta =
      item.kind === "orphanMmproj"
        ? t("aiSettings.models.local.orphanMmproj")
        : [
            t(
              item.kind === "recommended"
                ? "settings.aiModels.mine.local"
                : "settings.aiModels.mine.imported",
              { size },
            ),
            ...(mmproj ? [t("settings.aiModels.mine.withMmproj")] : []),
          ].join(" · ");
    return (
      <div key={main.filename} className={s.item}>
        <div className={s.row}>
          {item.kind === "recommended" ? (
            <ModelLogo rec={item.rec} />
          ) : (
            <span className={s.tile}>
              <FileBox aria-hidden />
            </span>
          )}
          <div className={s.main}>
            <span className={s.name}>
              {name}
              {item.kind === "recommended" ? <CapTags caps={item.rec.caps} /> : null}
              {main.supportsTools === false ? (
                <span
                  className={`${s.cap} ${s.capWarn}`}
                  title={t("aiSettings.models.local.noToolsTip")}
                >
                  {t("aiSettings.models.local.noTools")}
                </span>
              ) : null}
            </span>
            <span className={s.meta} title={mmproj?.filename}>
              {meta}
            </span>
          </div>
          <div className={s.side}>
            {item.kind === "orphanMmproj" ? null : usage(tasks)}
            <RowMenu
              label={t("settings.aiModels.mine.more", { name })}
              items={[
                {
                  label: t("settings.aiModels.mine.reveal"),
                  icon: FolderOpen,
                  onClick: () =>
                    void revealItemInDir(main.path).catch((e) => logError("models.reveal", e)),
                },
                {
                  label: t("settings.aiModels.mine.delete"),
                  icon: Trash2,
                  danger: true,
                  onClick: () => setDeleting(item),
                },
              ]}
            />
          </div>
        </div>
      </div>
    );
  };

  const cloudRow = (conn: CloudConnection) => {
    let host = conn.endpoint;
    try {
      host = new URL(conn.endpoint).host;
    } catch {
      // 地址填得不完整时照原样显示
    }
    const name = profileLabel(conn);
    return (
      <div key={conn.id} className={s.item}>
        <div className={s.row}>
          <span className={s.tile}>
            <Cloud aria-hidden />
          </span>
          <div className={s.main}>
            <span className={s.name}>
              {name}
              {isCloudConnectionReady(conn) ? null : (
                <span className={`${s.cap} ${s.capWarn}`}>
                  {t("settings.aiModels.mine.cloudIncomplete")}
                </span>
              )}
            </span>
            <span className={s.meta}>{t("settings.aiModels.mine.cloud", { host })}</span>
          </div>
          <div className={s.side}>
            {usage(tasksUsing(ai.picks, { kind: "cloud", connection: conn.id }))}
            <RowMenu
              label={t("settings.aiModels.mine.more", { name })}
              items={[
                {
                  label: t("settings.aiModels.mine.edit"),
                  icon: Pencil,
                  onClick: () => onEditCloud(conn.id),
                },
                {
                  label: t("settings.aiModels.mine.deleteCloud"),
                  icon: Trash2,
                  danger: true,
                  onClick: () => setDeletingCloud(conn),
                },
              ]}
            />
          </div>
        </div>
      </div>
    );
  };

  const deletingName =
    deleting?.kind === "recommended"
      ? deleting.rec.displayName
      : deleting?.kind === "file"
        ? deleting.main.filename
        : (deleting?.file.filename ?? "");
  const deletingTasks =
    deleting && deleting.kind !== "orphanMmproj"
      ? tasksUsing(ai.picks, { kind: "local", file: deleting.main.filename })
      : [];
  const cloudTasks = deletingCloud
    ? tasksUsing(ai.picks, { kind: "cloud", connection: deletingCloud.id })
    : [];

  return (
    <Section
      title={t("settings.aiModels.mine.title")}
      description={t("settings.aiModels.mine.desc")}
      headerAction={
        empty ? null : (
          <Button
            variant="outline"
            icon={Activity}
            loading={test.kind === "running"}
            onClick={() => void runTest()}
            title={t("settings.aiModels.mine.testTip")}
          >
            {t("aiSettings.engine.actions.testConnection")}
          </Button>
        )
      }
    >
      {test.kind === "done" || test.kind === "fail" ? (
        <div className={s.tests} role="status">
          {test.kind === "fail" ? (
            <span className={`${s.test} ${s.testFail}`}>
              <XCircle size={14} strokeWidth={2.2} />
              {test.message}
            </span>
          ) : (
            test.items.map((item) => {
              const label = item.model
                ? `${t(TASK_LABEL[item.task])} · ${item.model}`
                : t(TASK_LABEL[item.task]);
              return (
                <span key={item.task} className={`${s.test} ${item.ok ? s.testOk : s.testFail}`}>
                  {item.ok ? (
                    <Check size={14} strokeWidth={2.2} />
                  ) : (
                    <XCircle size={14} strokeWidth={2.2} />
                  )}
                  {item.ok ? label : `${label}：${localizeAiError(item.message)}`}
                </span>
              );
            })
          )}
        </div>
      ) : test.kind === "running" ? (
        <div className={s.tests} role="status">
          <span className={`${s.test} ${s.testRunning}`}>
            <Loader2 size={14} strokeWidth={2.2} className={legacy.testSpin} />
            {t("settings.aiModels.mine.testing")}
          </span>
        </div>
      ) : null}
      {models.error?.place === "mine" ? (
        <div className={`${legacy.engineError} ${s.error}`}>{models.error.message}</div>
      ) : null}

      {empty ? (
        <div className={s.start}>
          <p className={s.startLead}>{t("settings.aiModels.mine.empty")}</p>
          <div className={s.choice}>
            <span className={s.choiceTitle}>
              <Download size={14} strokeWidth={2} aria-hidden />
              {t("settings.aiModels.mine.emptyLocal")}
            </span>
            <p className={s.choiceText}>{t("settings.aiModels.mine.emptyLocalDesc")}</p>
            <Button variant="outline" tone="accent" onClick={onGoToCatalog}>
              {t("settings.aiModels.mine.emptyLocalAction")}
            </Button>
          </div>
          <div className={s.choice}>
            <span className={s.choiceTitle}>
              <Cloud size={14} strokeWidth={2} aria-hidden />
              {t("settings.aiModels.mine.emptyCloud")}
            </span>
            <p className={s.choiceText}>{t("settings.aiModels.mine.emptyCloudDesc")}</p>
            <Button variant="outline" tone="accent" onClick={onAddCloud}>
              {t("settings.aiModels.mine.emptyCloudAction")}
            </Button>
          </div>
        </div>
      ) : (
        <div>
          {models.items.map(localRow)}
          {ai.cloud.map(cloudRow)}
        </div>
      )}

      <ConfirmDialog
        open={deleting !== null}
        title={t("settings.aiModels.mine.deleteTitle", { name: deletingName })}
        message={
          deletingTasks.length > 0
            ? t("settings.aiModels.mine.deleteMessageInUse", { tasks: taskNames(deletingTasks) })
            : t("settings.aiModels.mine.deleteMessage")
        }
        variant="danger"
        onConfirm={() => {
          const item = deleting;
          setDeleting(null);
          if (item) void models.remove(item);
        }}
        onCancel={() => setDeleting(null)}
      />
      <ConfirmDialog
        open={deletingCloud !== null}
        title={t("settings.aiModels.mine.deleteTitle", {
          name: deletingCloud ? profileLabel(deletingCloud) : "",
        })}
        message={
          cloudTasks.length > 0
            ? t("settings.aiModels.mine.deleteCloudMessageInUse", { tasks: taskNames(cloudTasks) })
            : t("settings.aiModels.mine.deleteCloudMessage")
        }
        variant="danger"
        onConfirm={() => {
          const conn = deletingCloud;
          setDeletingCloud(null);
          if (conn) onDeleteCloud(conn.id);
        }}
        onCancel={() => setDeletingCloud(null)}
      />
    </Section>
  );
}
