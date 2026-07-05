import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { ConfirmDialog } from "../../components/ConfirmDialog/ConfirmDialog";
import { api, type ChatConversationMeta } from "../../api/hindsight";
import { logError } from "../../lib/logger";
import styles from "./ChatPage.module.css";

interface ConversationListProps {
  conversations: ChatConversationMeta[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onNew: () => void;
  /** 重命名/删除落库后让父组件刷新列表 */
  onChanged: () => void;
  /** 删除的是当前会话时,父组件把 activeId 置空 */
  onDeletedActive: () => void;
}

/** "2026-07-05T09:30:00+09:00" → "07-05 09:30" */
function fmtUpdated(ts: string): string {
  return ts.length >= 16 ? ts.slice(5, 16).replace("T", " ") : ts;
}

/**
 * 会话列表:「新对话」+ 会话项(hover 出重命名/删除)。
 * 重命名是内联 input(Enter 提交 / Esc 取消);删除走 danger 确认框。
 */
export default function ConversationList({
  conversations,
  activeId,
  onSelect,
  onNew,
  onChanged,
  onDeletedActive,
}: ConversationListProps) {
  const { t } = useTranslation();
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editText, setEditText] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<ChatConversationMeta | null>(null);

  const commitRename = async () => {
    const id = editingId;
    setEditingId(null);
    if (id === null || !editText.trim()) return;
    try {
      await api.chatRenameConversation(id, editText.trim());
      onChanged();
    } catch (e) {
      logError("chat.rename", e);
    }
  };

  const onEditKey = (e: KeyboardEvent) => {
    if (e.key === "Enter") void commitRename();
    if (e.key === "Escape") setEditingId(null);
  };

  const confirmDelete = async () => {
    const target = deleteTarget;
    setDeleteTarget(null);
    if (!target) return;
    try {
      await api.chatDeleteConversation(target.id);
      if (target.id === activeId) onDeletedActive();
      onChanged();
    } catch (e) {
      logError("chat.delete", e);
    }
  };

  return (
    <nav className={styles.convList} aria-label={t("chat.title")}>
      <button type="button" className={styles.newConvBtn} onClick={onNew}>
        <Plus size={14} strokeWidth={2.2} />
        {t("chat.conversations.new")}
      </button>

      {conversations.length === 0 ? (
        <p className={styles.convEmpty}>{t("chat.conversations.empty")}</p>
      ) : (
        <ul className={styles.convItems}>
          {conversations.map((c) => (
            <li key={c.id}>
              {editingId === c.id ? (
                <input
                  type="text"
                  className={styles.convRenameInput}
                  value={editText}
                  onChange={(e) => setEditText(e.target.value)}
                  onKeyDown={onEditKey}
                  onBlur={() => void commitRename()}
                  // 进入重命名态即可打字是该交互的全部意义
                  // eslint-disable-next-line jsx-a11y/no-autofocus
                  autoFocus
                />
              ) : (
                <div
                  className={`${styles.convItem} ${c.id === activeId ? styles.convItemActive : ""}`}
                >
                  <button
                    type="button"
                    className={styles.convSelect}
                    onClick={() => onSelect(c.id)}
                    title={c.title}
                  >
                    <span className={styles.convTitle}>{c.title}</span>
                    <span className={styles.convTime}>{fmtUpdated(c.updatedTs)}</span>
                  </button>
                  <span className={styles.convActions}>
                    <button
                      type="button"
                      className={styles.convActionBtn}
                      aria-label={t("chat.conversations.renameAria")}
                      title={t("chat.conversations.renameAria")}
                      onClick={() => {
                        setEditingId(c.id);
                        setEditText(c.title);
                      }}
                    >
                      <Pencil size={12} strokeWidth={2} />
                    </button>
                    <button
                      type="button"
                      className={styles.convActionBtn}
                      aria-label={t("chat.conversations.deleteAria")}
                      title={t("chat.conversations.deleteAria")}
                      onClick={() => setDeleteTarget(c)}
                    >
                      <Trash2 size={12} strokeWidth={2} />
                    </button>
                  </span>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        title={t("chat.conversations.deleteTitle")}
        message={t("chat.conversations.deleteMessage")}
        confirmLabel={t("chat.conversations.deleteConfirm")}
        variant="danger"
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </nav>
  );
}
