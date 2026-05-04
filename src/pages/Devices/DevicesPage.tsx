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
  // sync 报告 token 解密失败 / 凭证失效时，把"退出"换成"重新登录"
  // —— 用户多半就是想刷新 token 而不是真的登出
  const authExpired =
    signedIn &&
    !!sync?.lastError &&
    (sync.lastError.includes("登录凭证失效") ||
      sync.lastError.includes("aes decrypt") ||
      sync.lastError.includes("crypto: aes"));
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
                className={`${styles.smallBtn} ${styles.smallBtnSwap} ${
                  authExpired ? styles.smallBtnAccent : styles.smallBtnDanger
                }`}
                onClick={authExpired ? onSignIn : onSignOut}
                disabled={busy}
                title={authExpired ? "凭证失效，重新登录刷新 token" : "退出当前账号"}
              >
                <span
                  className={styles.smallBtnFace}
                  data-active={!authExpired}
                  aria-hidden={authExpired}
                >
                  <LogOut size={13} strokeWidth={1.85} />
                  退出
                </span>
                <span
                  className={styles.smallBtnFace}
                  data-active={authExpired}
                  aria-hidden={!authExpired}
                >
                  <LogIn size={13} strokeWidth={1.85} />
                  重新登录
                </span>
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
      {!error && signedIn && sync?.lastError && (
        <div className={styles.syncError}>{sync.lastError}</div>
      )}
      {auth?.requiresRestart && (
        <div className={styles.syncError}>
          已登录到新的账号，需要重启 app 才能切换到该账号的本地数据库。
          <button
            type="button"
            className={styles.smallBtn}
            style={{ marginLeft: 8 }}
            onClick={() => api.restartApp().catch(() => {})}
          >
            <RefreshCw size={13} strokeWidth={1.85} />
            重启 app
          </button>
        </div>
      )}
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
              点击 <span className={styles.cueBtn}>启用</span> 按钮。
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
              首次进入可能要先点击 <span className={styles.cueBtn}>开始</span>{" "}
              按钮 → 随意填写应用名称与邮箱 → 受众群体选择 外部 →
              在联系信息里再次填写邮箱地址 → 点击{" "}
              <span className={styles.cueBtn}>创建</span> 按钮。
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
              左侧 客户端 → 点击上方{" "}
              <span className={styles.cueLink}>+创建新客户端</span> →
              应用类型选择 桌面应用 → 名称随意，然后会显示 客户端 ID 与 客户端密钥。
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
            <div className={styles.stepTitle}>把 客户端 ID 和 客户端密钥 粘贴到下面</div>
            <label className={styles.credField}>
              <span className={styles.credLabel}>客户端 ID</span>
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
              <span className={styles.credLabel}>客户端密钥</span>
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
                  aria-label={secretVisible ? "隐藏客户端密钥" : "显示客户端密钥"}
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
