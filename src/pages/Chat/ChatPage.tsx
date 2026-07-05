import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  api,
  type ChatConversationMeta,
  type MemoryPendingStats,
} from "../../api/hindsight";
import { useSettings } from "../../state/settings";
import { logError } from "../../lib/logger";
import ChatView from "./ChatView";
import ConversationList from "./ConversationList";
import ModelBadge from "./ModelBadge";
import BackfillBanner from "./BackfillBanner";
import { ChatPrivacyDialog } from "./ChatPrivacyDialog";
import styles from "./ChatPage.module.css";

/**
 * 对话页外壳:标题行(右侧当前模型 badge)→ 未入索引 banner → 左右两栏
 * (左会话列表 / 右聊天区)。会话与消息都落 memory.sqlite,重启不丢。
 */
export default function ChatPage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const [conversations, setConversations] = useState<ChatConversationMeta[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [pendingStats, setPendingStats] = useState<MemoryPendingStats | null>(null);
  const [privacyOpen, setPrivacyOpen] = useState(false);
  // 隐私弹窗的挂起 resolver:发送流程 await 它,确认/取消后继续/中止
  const privacyResolver = useRef<((ok: boolean) => void) | null>(null);

  const refreshConversations = useCallback(() => {
    api.chatListConversations().then(setConversations).catch((e) => {
      logError("chat.listConversations", e);
    });
  }, []);

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
