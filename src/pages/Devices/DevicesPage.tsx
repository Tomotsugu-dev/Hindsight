import { useEffect, useRef, useState } from "react";
import { Check, Cloud, CloudOff, Monitor, Pencil, RefreshCw, Unplug, X } from "lucide-react";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import styles from "./DevicesPage.module.css";

/** 同步状态 — Phase A 用本地 state，Phase C 接到后端 */
interface SyncState {
  connected: boolean;
  account?: string;
  lastSyncAt?: string; // 人类可读
}

const MOCK_INITIAL_SYNC: SyncState = { connected: false };

export default function DevicesPage() {
  const { devices, renameSelf } = useDeviceFilter();
  const self = devices.find((d) => d.current);
  const others = devices.filter((d) => !d.current);

  const [sync, setSync] = useState<SyncState>(MOCK_INITIAL_SYNC);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>设备</h1>
        <p className={styles.meta}>
          管理本机名称，与其他设备的同步状态。
        </p>
      </header>

      <SyncCard sync={sync} onChange={setSync} />

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>本机</h2>
        {self && <SelfCard device={self} onRename={renameSelf} />}
      </section>

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>其他设备</h2>
        {others.length === 0 ? (
          <div className={styles.empty}>
            {sync.connected
              ? "暂无其他设备。在新设备上用同一账号登录即可加入。"
              : "未连接同步。连接后可在多台设备之间共享数据。"}
          </div>
        ) : (
          <div className={styles.cardList}>
            {others.map((d) => (
              <OtherCard key={d.id} device={d} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

/* —— 同步状态卡 —— */

interface SyncCardProps {
  sync: SyncState;
  onChange: (s: SyncState) => void;
}

function SyncCard({ sync, onChange }: SyncCardProps) {
  if (sync.connected) {
    return (
      <div className={`${styles.syncCard} ${styles.syncCardConnected}`}>
        <div className={styles.syncIcon}>
          <Cloud size={20} strokeWidth={1.6} />
        </div>
        <div className={styles.syncBody}>
          <div className={styles.syncTitle}>
            已连接 · {sync.account ?? "Google Drive"}
          </div>
          <div className={styles.syncMeta}>
            上次同步: {sync.lastSyncAt ?? "刚刚"}
          </div>
        </div>
        <div className={styles.syncActions}>
          <button
            type="button"
            className={styles.smallBtn}
            onClick={() => onChange({ ...sync, lastSyncAt: "刚刚" })}
            title="立即同步"
          >
            <RefreshCw size={13} strokeWidth={1.85} />
            立即同步
          </button>
          <button
            type="button"
            className={`${styles.smallBtn} ${styles.smallBtnDanger}`}
            onClick={() => onChange({ connected: false })}
            title="断开连接"
          >
            <Unplug size={13} strokeWidth={1.85} />
            断开
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.syncCard}>
      <div className={`${styles.syncIcon} ${styles.syncIconMuted}`}>
        <CloudOff size={20} strokeWidth={1.6} />
      </div>
      <div className={styles.syncBody}>
        <div className={styles.syncTitle}>未连接</div>
        <div className={styles.syncMeta}>
          连接 Google Drive 后，可在多台设备之间同步采集数据。
        </div>
      </div>
      <div className={styles.syncActions}>
        <button
          type="button"
          className={styles.connectBtn}
          onClick={() =>
            onChange({
              connected: true,
              account: "demo@gmail.com",
              lastSyncAt: "刚刚",
            })
          }
        >
          <Cloud size={13} strokeWidth={2} />
          连接 Google Drive
        </button>
      </div>
    </div>
  );
}

/* —— 本机卡 —— */

interface SelfCardProps {
  device: Device;
  onRename: (name: string) => void;
}

function SelfCard({ device, onRename }: SelfCardProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(device.name);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  useEffect(() => {
    if (!editing) setDraft(device.name);
  }, [device.name, editing]);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== device.name) {
      onRename(trimmed);
    } else {
      setDraft(device.name);
    }
    setEditing(false);
  };

  const cancel = () => {
    setDraft(device.name);
    setEditing(false);
  };

  return (
    <div className={styles.card}>
      <div className={styles.iconBox}>
        <Monitor size={20} strokeWidth={1.5} />
      </div>

      <div className={styles.body}>
        {editing ? (
          <div className={styles.editRow}>
            <input
              ref={inputRef}
              className={styles.input}
              value={draft}
              maxLength={32}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commit();
                if (e.key === "Escape") cancel();
              }}
              onBlur={commit}
            />
            <button
              type="button"
              className={`${styles.iconBtn} ${styles.iconBtnPrimary}`}
              onMouseDown={(e) => e.preventDefault()}
              onClick={commit}
              aria-label="保存"
            >
              <Check size={14} strokeWidth={2.25} />
            </button>
            <button
              type="button"
              className={styles.iconBtn}
              onMouseDown={(e) => e.preventDefault()}
              onClick={cancel}
              aria-label="取消"
            >
              <X size={14} strokeWidth={2.25} />
            </button>
          </div>
        ) : (
          <div className={styles.nameRow}>
            <span className={styles.name}>{device.name}</span>
            <span className={styles.tag}>本机</span>
            <button
              type="button"
              className={styles.renameBtn}
              onClick={() => setEditing(true)}
              aria-label="改名"
              title="改名"
            >
              <Pencil size={12} strokeWidth={1.85} />
              改名
            </button>
          </div>
        )}
        <div className={styles.metaRow}>
          <span>上次活动: 刚刚</span>
          <span className={styles.dot}>·</span>
          <span>今日采集 0 条</span>
        </div>
      </div>
    </div>
  );
}

function OtherCard({ device }: { device: Device }) {
  return (
    <div className={styles.card}>
      <div className={styles.iconBox}>
        <Monitor size={20} strokeWidth={1.5} />
      </div>
      <div className={styles.body}>
        <div className={styles.nameRow}>
          <span className={styles.name}>{device.name}</span>
        </div>
        <div className={styles.metaRow}>
          <span>上次同步: —</span>
        </div>
      </div>
    </div>
  );
}
