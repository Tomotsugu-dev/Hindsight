import { useEffect, useRef, useState, type CSSProperties } from "react";
import {
  ChevronRight,
  Cloud,
  CloudOff,
  Eye,
  EyeOff,
  LogIn,
  LogOut,
  Pencil,
  RefreshCw,
  Settings2,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { useCaptureStatus } from "../../hooks/useCaptureStatus";
import { useSettings } from "../../state/settings";
import { AppearancePicker } from "../../components/AppearancePicker/AppearancePicker";
import { resolveCategoryIcon } from "../../config/categoryIcons";
import { api, type AuthState, type SyncStatus } from "../../api/hindsight";
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

function fmtRelative(iso: string): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "刚刚";
  const diff = Date.now() - t;
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

function CloudSyncCard() {
  const { settings, update } = useSettings();
  const { reload: reloadDevices } = useDeviceFilter();
  const [auth, setAuth] = useState<AuthState | null>(null);
  const [sync, setSync] = useState<SyncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [syncBusy, setSyncBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);

  useEffect(() => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
    const fetchSync = () => {
      api.syncStatus().then(setSync).catch(() => {});
    };
    fetchSync();
    const t = window.setInterval(fetchSync, 10_000);
    return () => window.clearInterval(t);
  }, []);

  // 没填凭证 → 默认展开设置面板；填好且未登录 → 收起
  useEffect(() => {
    if (auth && !auth.signedIn) {
      setSetupOpen(!auth.configured);
    } else if (auth?.signedIn) {
      setSetupOpen(false);
    }
  }, [auth?.configured, auth?.signedIn]);

  const refreshAuth = () => {
    api.authStatus().then(setAuth).catch(() => setAuth(null));
  };
  const refreshSync = () => {
    api.syncStatus().then(setSync).catch(() => {});
  };

  const onSignIn = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.signInWithGoogle();
      setAuth(next);
      refreshSync();
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
      refreshSync();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onSyncNow = async () => {
    setSyncBusy(true);
    setError(null);
    try {
      await api.syncNow();
      refreshSync();
      // 拉到新的远端活动后，让 device 列表也刷一下
      reloadDevices();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refreshSync();
    } finally {
      setSyncBusy(false);
    }
  };

  if (!settings) return null;

  const signedIn = auth?.signedIn ?? false;
  const configured = auth?.configured ?? false;
  // 用户改凭证后 auth.configured 可能没及时更新，所以 UI 也按本地 settings 算一遍
  const credsFilled = !!(
    settings.googleClientId.trim() && settings.googleClientSecret.trim()
  );
  const canSignIn = configured || credsFilled;

  // 当填好凭证后，刷新一下 auth 让 configured 同步过来（用于按钮可点态）
  useEffect(() => {
    if (credsFilled && !configured && !signedIn) {
      api.authStatus().then(setAuth).catch(() => {});
    }
  }, [credsFilled, configured, signedIn]);

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
            {signedIn
              ? "已连接 Google Drive"
              : canSignIn
                ? "未连接"
                : "未配置"}
          </div>
          <div className={styles.syncMeta}>
            {signedIn
              ? auth?.email ?? auth?.uid ?? ""
              : canSignIn
                ? "用 Google 账号登录，活动数据通过 Drive 在多设备间同步。截图不上传。"
                : "首次使用前需要在 Google Cloud Console 创建一个 OAuth 凭证（约 3 分钟）。"}
          </div>
          {signedIn && sync && (
            <div className={styles.syncStats}>
              {sync.pending > 0 ? (
                <>
                  <span
                    className={`${styles.syncDot} ${styles.syncDotPending}`}
                    aria-hidden
                  />
                  <span className={styles.syncStatPending}>
                    待发送 {sync.pending}
                  </span>
                </>
              ) : sync.lastPushedAt ? (
                <>
                  <span className={styles.syncDot} aria-hidden />
                  <span className={styles.syncStatOk}>
                    已同步 · {fmtRelative(sync.lastPushedAt)}
                  </span>
                </>
              ) : (
                <span>等待首次同步</span>
              )}
              {sync.deadLetter > 0 && (
                <span className={styles.syncStatErr}>
                  · {sync.deadLetter} 条失败
                </span>
              )}
            </div>
          )}
          {!signedIn && canSignIn && !setupOpen && (
            <button
              type="button"
              className={styles.editCredsLink}
              onClick={() => setSetupOpen(true)}
            >
              改凭证
            </button>
          )}
        </div>
        <div className={styles.syncActions}>
          {signedIn ? (
            <>
              <button
                type="button"
                className={styles.smallBtn}
                onClick={onSyncNow}
                disabled={syncBusy}
                title="立即同步"
              >
                <RefreshCw
                  size={13}
                  strokeWidth={1.85}
                  className={syncBusy ? styles.spinning : ""}
                />
                {syncBusy ? "同步中…" : "立即同步"}
              </button>
              <button
                type="button"
                className={`${styles.smallBtn} ${styles.smallBtnDanger}`}
                onClick={onSignOut}
                disabled={busy}
              >
                <LogOut size={13} strokeWidth={1.85} />
                退出
              </button>
            </>
          ) : canSignIn ? (
            <button
              type="button"
              className={styles.connectBtn}
              onClick={onSignIn}
              disabled={busy}
            >
              <LogIn size={13} strokeWidth={2} />
              {busy ? "浏览器中…" : "用 Google 登录"}
            </button>
          ) : (
            <button
              type="button"
              className={styles.connectBtn}
              onClick={() => setSetupOpen(true)}
            >
              <Settings2 size={13} strokeWidth={2} />
              配置 OAuth
            </button>
          )}
        </div>
      </div>

      <div
        className={`${styles.setupWrap} ${
          setupOpen && !signedIn ? styles.setupWrapOpen : ""
        }`}
        aria-hidden={!setupOpen || signedIn}
      >
        <div className={styles.setupInner}>
          <SetupPanel
            clientId={settings.googleClientId}
            clientSecret={settings.googleClientSecret}
            onChangeId={(v) => update({ googleClientId: v })}
            onChangeSecret={(v) => update({ googleClientSecret: v })}
            collapsible={canSignIn}
            onCollapse={() => setSetupOpen(false)}
          />
        </div>
      </div>

      {error && <div className={styles.syncError}>{error}</div>}
    </div>
  );
}

function SetupPanel({
  clientId,
  clientSecret,
  onChangeId,
  onChangeSecret,
  collapsible,
  onCollapse,
}: {
  clientId: string;
  clientSecret: string;
  onChangeId: (v: string) => void;
  onChangeSecret: (v: string) => void;
  collapsible: boolean;
  onCollapse: () => void;
}) {
  const [secretVisible, setSecretVisible] = useState(false);
  const open = (url: string) => {
    void openUrl(url).catch(() => {});
  };
  return (
    <div className={styles.setupPanel}>
      <div className={styles.setupHeader}>
        <Settings2 size={13} strokeWidth={2} />
        <span>OAuth 凭证设置</span>
        {collapsible && (
          <button
            type="button"
            className={styles.setupClose}
            onClick={onCollapse}
          >
            收起
          </button>
        )}
      </div>
      <ol className={styles.setupSteps}>
        <li>
          <span className={styles.stepNum}>1</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>启用 Google Drive API</div>
            <div className={styles.stepDesc}>
              在任意 Cloud 项目里 Enable 一下（没项目就新建一个，免费）。
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() =>
                open(
                  "https://console.cloud.google.com/apis/library/drive.googleapis.com",
                )
              }
            >
              打开 Drive API <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>2</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>配置 OAuth 同意页</div>
            <div className={styles.stepDesc}>
              左侧 <b>Audience</b> → User Type 选 External →{" "}
              <b>Test users</b> 加你的 Gmail。<br />
              首次配置会先跳到 <b>Branding</b>，填个应用名 + 联系邮箱即可。
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() => open("https://console.cloud.google.com/auth/audience")}
            >
              打开 Audience <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>3</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>创建 OAuth 客户端</div>
            <div className={styles.stepDesc}>
              左侧 <b>Clients</b> → Create client → Application type 选{" "}
              <b>Desktop app</b>，名字随意。建好后会显示 Client ID + Secret。
            </div>
            <button
              type="button"
              className={styles.stepBtn}
              onClick={() => open("https://console.cloud.google.com/auth/clients")}
            >
              打开 Clients <ChevronRight size={13} strokeWidth={2.25} />
            </button>
          </div>
        </li>
        <li>
          <span className={styles.stepNum}>4</span>
          <div className={styles.stepBody}>
            <div className={styles.stepTitle}>把 Client ID / Secret 粘到下面</div>
            <label className={styles.credField}>
              <span className={styles.credLabel}>Client ID</span>
              <input
                type="text"
                className={styles.credInput}
                value={clientId}
                onChange={(e) => onChangeId(e.target.value)}
                placeholder="xxxxxx.apps.googleusercontent.com"
                spellCheck={false}
                autoComplete="off"
              />
            </label>
            <label className={styles.credField}>
              <span className={styles.credLabel}>Client Secret</span>
              <div className={styles.credInputWrap}>
                <input
                  type={secretVisible ? "text" : "password"}
                  className={`${styles.credInput} ${styles.credInputWithBtn}`}
                  value={clientSecret}
                  onChange={(e) => onChangeSecret(e.target.value)}
                  placeholder="GOCSPX-..."
                  spellCheck={false}
                  autoComplete="off"
                />
                <button
                  type="button"
                  className={styles.credEyeBtn}
                  onClick={() => setSecretVisible((v) => !v)}
                  aria-label={secretVisible ? "隐藏 Client Secret" : "显示 Client Secret"}
                  title={secretVisible ? "隐藏" : "显示"}
                  tabIndex={-1}
                >
                  {secretVisible ? (
                    <EyeOff size={14} strokeWidth={1.85} />
                  ) : (
                    <Eye size={14} strokeWidth={1.85} />
                  )}
                </button>
              </div>
            </label>
            <div className={styles.credHint}>
              凭证只存本地。两项都填好后，"用 Google 登录"按钮会自动变可点。
            </div>
          </div>
        </li>
      </ol>
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
