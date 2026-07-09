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
import { logError } from "../../lib/logger";
import ChatView from "./ChatView";
import ConversationList from "./ConversationList";
import ModelBadge from "./ModelBadge";
import BackfillBanner from "./BackfillBanner";
import { chatUsesCloud } from "./chatRouting";
import { ChatPrivacyDialog } from "./ChatPrivacyDialog";
import styles from "./ChatPage.module.css";

// 最后打开的会话,存 localStorage:跳页回来、甚至 webview 销毁重建(macOS 关窗
// 收托盘)后都自动恢复原会话——配合 chatInflight 查询,"关窗等答案再打开"能
// 直接回到生成中的现场。删除/失效由列表加载时校验回退。
const LAST_CONVERSATION_KEY = "hindsight.chat.lastConversation";

function readLastActive(): number | null {
  const raw = localStorage.getItem(LAST_CONVERSATION_KEY);
  const n = raw === null ? NaN : Number(raw);
  return Number.isInteger(n) ? n : null;
}

function writeLastActive(id: number | null) {
  if (id === null) localStorage.removeItem(LAST_CONVERSATION_KEY);
  else localStorage.setItem(LAST_CONVERSATION_KEY, String(id));
}

/**
 * 对话页外壳:标题行(右侧当前模型 badge)→ 未入索引 banner → 左右两栏
 * (左会话列表 / 右聊天区)。会话与消息都落 memory.sqlite,重启不丢。
 */
export default function ChatPage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [conversations, setConversations] = useState<ChatConversationMeta[]>([]);
  const [activeId, setActiveIdState] = useState<number | null>(readLastActive);
  const [pendingStats, setPendingStats] = useState<MemoryPendingStats | null>(null);
  const [privacyOpen, setPrivacyOpen] = useState(false);
  // 隐私弹窗的挂起 resolver:发送流程 await 它,确认/取消后继续/中止
  const privacyResolver = useRef<((ok: boolean) => void) | null>(null);

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

  const refreshPendingStats = useCallback(() => {
    api.memoryPendingStats().then(setPendingStats).catch((e) => {
      logError("chat.pendingStats", e);
    });
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
        <ModelBadge />
      </header>

      {!chatUsesCloud(settings.ai) && (
        <p className={styles.localHint}>{t("chat.localModelHint")}</p>
      )}

      {pendingStats && (
        <BackfillBanner stats={pendingStats} onRefresh={refreshPendingStats} />
      )}

      <div className={styles.columns}>
        <ConversationList
          conversations={conversations}
          activeId={activeId}
          onSelect={setActiveId}
          onNew={() => setActiveId(null)}
          onChanged={refreshConversations}
          onDeletedActive={() => setActiveId(null)}
        />
        <div className={styles.main}>
          <ChatView
            conversationId={activeId}
            onConversationCreated={(id) => {
              setActiveId(id);
              refreshConversations();
            }}
            onConversationTouched={refreshConversations}
            ensurePrivacyAck={ensurePrivacyAck}
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
