import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  CHAT_ANSWER_READY_EVENT,
  type ChatConversationMeta,
  type MemoryPendingStats,
} from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { useAiProfiles } from "../../state/aiProfiles";
import { logError } from "../../lib/logger";
import { getUiState, setUiState } from "../../state/uiState";
import ChatView from "./ChatView";
import ConversationList from "./ConversationList";
import { chatLocalModelName, chatUsesCloud } from "./chatRouting";
import { ChatPrivacyDialog } from "./ChatPrivacyDialog";
import BackfillBanner from "../../components/BackfillBanner/BackfillBanner";
import styles from "./ChatPage.module.css";

// 最后打开的会话,存在界面状态里:跳页回来、甚至 webview 销毁重建(macOS 关窗
// 收托盘)后都自动恢复原会话——配合 chatInflight 查询,"关窗等答案再打开"能
// 直接回到生成中的现场。删除/失效由列表加载时校验回退。
const LAST_CONVERSATION_KEY = "hindsight.chat.lastConversation";

function readLastActive(): number | null {
  const raw = getUiState(LAST_CONVERSATION_KEY);
  const n = raw === null ? NaN : Number(raw);
  return Number.isInteger(n) ? n : null;
}

function writeLastActive(id: number | null) {
  setUiState(LAST_CONVERSATION_KEY, id === null ? null : String(id));
}

/**
 * The chat page: the title, the banner for screenshots not yet indexed, then
 * two columns (conversations on the left, the chat on the right).
 * The model, thinking and data pickers are in the chat's input box.
 * Conversations and messages are saved in memory.sqlite, so a restart keeps them.
 */
export default function ChatPage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  // AI 能看到哪些数据;不存,进页面回到默认分类。换了只影响后面的提问
  const profiles = useAiProfiles("chat");
  const [conversations, setConversations] = useState<ChatConversationMeta[]>([]);
  const [activeId, setActiveIdState] = useState<number | null>(readLastActive);
  const [privacyOpen, setPrivacyOpen] = useState(false);
  // 隐私弹窗的挂起 resolver:发送流程 await 它,确认/取消后继续/中止
  const privacyResolver = useRef<((ok: boolean) => void) | null>(null);
  // 当前本地 chat 模型未声明工具调用时 = 模型文件名(顶部黄条警示用);
  // 这类模型已从模型下拉隐藏,但历史上选中的仍会生效,得指名提醒
  const [noToolsModel, setNoToolsModel] = useState<string | null>(null);

  useEffect(() => {
    if (!settings || chatUsesCloud(settings.ai)) {
      setNoToolsModel(null);
      return;
    }
    const name = chatLocalModelName(settings.ai);
    if (!name) {
      setNoToolsModel(null);
      return;
    }
    let stale = false;
    api
      .listLocalModels()
      .then((all) => {
        if (stale) return;
        const hit = all.find((m) => m.filename === name && m.supportsTools === false);
        setNoToolsModel(hit ? name : null);
      })
      .catch(() => {});
    return () => {
      stale = true;
    };
  }, [settings]);

  const setActiveId = useCallback((id: number | null) => {
    writeLastActive(id);
    setActiveIdState(id);
  }, []);

  const refreshConversations = useCallback(() => {
    api
      .chatListConversations()
      .then((rows) => {
        setConversations(rows);
        // 记住的会话可能已被删除(或库被清):回退空态,避免打开不存在的会话
        const last = readLastActive();
        if (last !== null && !rows.some((r) => r.id === last)) {
          setActiveId(null);
        }
      })
      .catch((e) => {
        logError("chat.listConversations", e);
      });
  }, [setActiveId]);

  // 答案落库广播 → 刷新列表(会话排序按最近更新;跳页期间完成的新会话也靠它现身)
  useEffect(() => {
    const un = listen(CHAT_ANSWER_READY_EVENT, () => refreshConversations());
    return () => {
      void un.then((f) => f());
    };
  }, [refreshConversations]);

  // 未入索引提示(空闲态的"N 张查不到"只在对话页显示——这里才有语境;
  // 回填进行中的全局横幅由 AppLayout 负责,本页自带所以布局层避开了聊天路由)
  const [pendingStats, setPendingStats] = useState<MemoryPendingStats | null>(null);
  const refreshPendingStats = useCallback(() => {
    api
      .memoryPendingStats()
      .then(setPendingStats)
      .catch((e) => logError("chat.pendingStats", e));
  }, []);

  useEffect(() => {
    refreshConversations();
    refreshPendingStats();
  }, [refreshConversations, refreshPendingStats]);

  /** 发送前的隐私门:已确认过直接放行,否则弹窗等用户决定。 */
  const ensurePrivacyAck = useCallback((): Promise<boolean> => {
    if (settings?.chatPrivacyAcknowledged) return Promise.resolve(true);
    setPrivacyOpen(true);
    return new Promise<boolean>((resolve) => {
      privacyResolver.current = resolve;
    });
  }, [settings?.chatPrivacyAcknowledged]);

  const onPrivacyConfirm = () => {
    setPrivacyOpen(false);
    update({ chatPrivacyAcknowledged: true });
    privacyResolver.current?.(true);
    privacyResolver.current = null;
  };

  const onPrivacyCancel = () => {
    setPrivacyOpen(false);
    privacyResolver.current?.(false);
    privacyResolver.current = null;
  };

  if (!settings) return null;

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>{t("chat.title")}</h1>
      </header>

      {noToolsModel && (
        <p className={styles.localHint}>{t("chat.localModelNoTools", { model: noToolsModel })}</p>
      )}

      {pendingStats && <BackfillBanner stats={pendingStats} onRefresh={refreshPendingStats} />}

      <div className={styles.columns}>
        <div className={styles.side}>
          <ConversationList
            conversations={conversations}
            activeId={activeId}
            onSelect={setActiveId}
            onNew={() => setActiveId(null)}
            onChanged={refreshConversations}
            onDeletedActive={() => setActiveId(null)}
          />
        </div>
        <div className={styles.main}>
          <ChatView
            conversationId={activeId}
            onConversationCreated={(id) => {
              setActiveId(id);
              refreshConversations();
            }}
            onConversationTouched={refreshConversations}
            ensurePrivacyAck={ensurePrivacyAck}
            profiles={profiles}
          />
        </div>
      </div>

      <ChatPrivacyDialog
        open={privacyOpen}
        ai={settings.ai}
        onConfirm={onPrivacyConfirm}
        onCancel={onPrivacyCancel}
      />
    </div>
  );
}
