import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Cloud, CloudOff, Pencil, RefreshCw, Unplug } from "lucide-react";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import styles from "./DevicesPage.module.css";

interface SyncState {
  connected: boolean;
  account?: string;
  lastSyncAt?: string;
}

const MOCK_INITIAL_SYNC: SyncState = { connected: false };

export default function DevicesPage() {
  const { devices, renameSelf, recolorSelf, reiconSelf } = useDeviceFilter();
  const self = devices.find((d) => d.current);
  const others = devices.filter((d) => !d.current);

  const [sync, setSync] = useState<SyncState>(MOCK_INITIAL_SYNC);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>设备</h1>
        <p className={styles.meta}>管理本机名称，与其他设备的同步状态。</p>
      </header>

      <SyncCard sync={sync} onChange={setSync} />

      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>本机</h2>
        {self && (
          <SelfRow
            device={self}
            onRename={renameSelf}
            onRecolor={recolorSelf}
            onReicon={reiconSelf}
          />
        )}
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
          others.map((d) => <OtherRow key={d.id} device={d} />)
        )}
      </section>
    </div>
  );
}

function SyncCard({
  sync,
  onChange,
}: {
  sync: SyncState;
  onChange: (s: SyncState) => void;
}) {
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

function SelfRow({
  device,
  onRename,
  onRecolor,
  onReicon,
}: {
  device: Device;
  onRename: (name: string) => void;
  onRecolor: (color: string) => void;
  onReicon: (icon: string) => void;
}) {
  const { status } = useCaptureStatus();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(device.name);
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const Icon = resolveCategoryIcon(device.icon);

  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  useEffect(() => {
    if (!editing) setDraft(device.name);
  }, [device.name, editing]);

  const commitName = () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== device.name) {
      onRename(trimmed);
    } else {
      setDraft(device.name);
    }
    setEditing(false);
  };

  const cancelName = () => {
    setDraft(device.name);
    setEditing(false);
  };

  const styleVar = { "--cat-color": device.color } as CSSProperties;
  const lastSeen = status?.lastCaptureAt ? "刚刚" : "—";
  const todayCount = status?.todayCount ?? 0;

  return (
    <div className={styles.deviceRow} style={styleVar}>
      <div className={styles.deviceIconWrap}>
        <button
          type="button"
          className={styles.deviceIconBtn}
          onClick={() => setPickerOpen((v) => !v)}
          aria-label="改外观"
          title="改颜色和图标"
        >
          <Icon size={28} strokeWidth={1.85} />
        </button>
        {pickerOpen && (
          <AppearancePicker
            color={device.color}
            icon={device.icon}
            onColorChange={onRecolor}
            onIconChange={(i) => {
              onReicon(i);
              setPickerOpen(false);
            }}
            onDismiss={() => setPickerOpen(false)}
          />
        )}
      </div>

      <div className={styles.deviceBody}>
        <div className={styles.deviceNameRow}>
          {editing ? (
            <input
              ref={inputRef}
              className={styles.deviceNameInput}
              value={draft}
              maxLength={32}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") cancelName();
              }}
            />
          ) : (
            <span
              className={styles.deviceName}
              onDoubleClick={() => setEditing(true)}
              title="双击改名"
            >
              {device.name}
            </span>
          )}
          <span className={styles.tag}>本机</span>
        </div>
        <div className={styles.metaRow}>
          <span>上次活动: {lastSeen}</span>
          <span className={styles.dotSep}>·</span>
          <span>今日采集 {todayCount} 条</span>
        </div>
      </div>

      <div className={styles.deviceActions}>
        <button
          type="button"
          className={styles.actionBtn}
          onClick={() => setEditing(true)}
          aria-label="改名"
          title="改名"
        >
          <Pencil size={14} strokeWidth={1.85} />
        </button>
      </div>
    </div>
  );
}

function OtherRow({ device }: { device: Device }) {
  const Icon = resolveCategoryIcon(device.icon);
  const styleVar = { "--cat-color": device.color } as CSSProperties;
  return (
    <div className={styles.deviceRow} style={styleVar}>
      <div className={styles.deviceIconWrap}>
        <div className={styles.deviceIconBtn} aria-hidden>
          <Icon size={28} strokeWidth={1.85} />
        </div>
      </div>
      <div className={styles.deviceBody}>
        <div className={styles.deviceNameRow}>
          <span className={styles.deviceName}>{device.name}</span>
        </div>
        <div className={styles.metaRow}>
          <span>上次同步: —</span>
        </div>
      </div>
    </div>
  );
}
