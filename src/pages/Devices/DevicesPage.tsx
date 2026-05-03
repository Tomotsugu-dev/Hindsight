import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Cloud, CloudOff, LogIn, LogOut, Pencil } from "lucide-react";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { useSettings } from "../../state/settings";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { api, type AuthState } from "../../api/hindsight";
import styles from "./DevicesPage.module.css";

export default function DevicesPage() {
  const { devices, renameSelf, recolorSelf, reiconSelf } = useDeviceFilter();
  const self = devices.find((d) => d.current);
  const others = devices.filter((d) => !d.current);

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title}>设备</h1>
        <p className={styles.meta}>管理本机名称，与其他设备的同步状态。</p>
      </header>

      <CloudSyncCard />

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
            登录同账号后，其他设备会出现在这里。
          </div>
        ) : (
          others.map((d) => <OtherRow key={d.id} device={d} />)
        )}
      </section>
    </div>
  );
}

function CloudSyncCard() {
  const { settings, update } = useSettings();
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .authStatus()
      .then(setAuth)
      .catch(() => setAuth(null));
  }, []);

  const refreshAuth = () => {
    api
      .authStatus()
      .then(setAuth)
      .catch(() => setAuth(null));
  };

  const onSignIn = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.signInWithGoogle();
      setAuth(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshAuth();
    } finally {
      setBusy(false);
    }
  };

  const onSignOut = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.signOut();
      refreshAuth();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!settings) return null;

  const credsConfigured = !!(
    settings.firebaseClientId.trim() &&
    settings.firebaseClientSecret.trim() &&
    settings.firebaseApiKey.trim()
  );
  const signedIn = auth?.signedIn ?? false;

  return (
    <div
      className={`${styles.syncCard} ${signedIn ? styles.syncCardConnected : ""}`}
    >
      <div className={styles.syncHeader}>
        <div
          className={
            signedIn
              ? styles.syncIcon
              : `${styles.syncIcon} ${styles.syncIconMuted}`
          }
        >
          {signedIn ? (
            <Cloud size={20} strokeWidth={1.6} />
          ) : (
            <CloudOff size={20} strokeWidth={1.6} />
          )}
        </div>
        <div className={styles.syncBody}>
          <div className={styles.syncTitle}>
            {signedIn ? "已连接 Firebase" : "未连接"}
          </div>
          <div className={styles.syncMeta}>
            {signedIn
              ? auth?.email ?? auth?.uid ?? ""
              : "数据存进你自己的 Firebase 项目，多设备同账号互通。截图不上传。"}
          </div>
        </div>
        <div className={styles.syncActions}>
          {signedIn ? (
            <button
              type="button"
              className={`${styles.smallBtn} ${styles.smallBtnDanger}`}
              onClick={onSignOut}
              disabled={busy}
            >
              <LogOut size={13} strokeWidth={1.85} />
              退出
            </button>
          ) : (
            <button
              type="button"
              className={styles.connectBtn}
              onClick={onSignIn}
              disabled={busy || !credsConfigured}
            >
              <LogIn size={13} strokeWidth={2} />
              {busy ? "浏览器中…" : "用 Google 登录"}
            </button>
          )}
        </div>
      </div>

      <div className={styles.credForm}>
        <label className={styles.credField}>
          <span className={styles.credLabel}>Client ID</span>
          <input
            type="text"
            className={styles.credInput}
            value={settings.firebaseClientId}
            onChange={(e) => update({ firebaseClientId: e.target.value })}
            placeholder="xxxxxx.apps.googleusercontent.com"
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <label className={styles.credField}>
          <span className={styles.credLabel}>Client Secret</span>
          <input
            type="password"
            className={styles.credInput}
            value={settings.firebaseClientSecret}
            onChange={(e) =>
              update({ firebaseClientSecret: e.target.value })
            }
            placeholder="GOCSPX-..."
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <label className={styles.credField}>
          <span className={styles.credLabel}>Web API Key</span>
          <input
            type="text"
            className={styles.credInput}
            value={settings.firebaseApiKey}
            onChange={(e) => update({ firebaseApiKey: e.target.value })}
            placeholder="AIzaSy..."
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <div className={styles.credHint}>
          需要在自己的 Firebase 项目里启用 Google 登录方法。
          {!credsConfigured && signedIn === false ? " 三项都填好后才能点登录。" : ""}
        </div>
      </div>

      {error && <div className={styles.syncError}>{error}</div>}
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
