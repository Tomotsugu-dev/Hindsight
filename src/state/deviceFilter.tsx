import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { api, type DeviceRow } from "../api/hindsight";
import { logError } from "../lib/logger";

export interface Device {
  id: string;
  name: string;
  color: string;
  icon: string;
  current: boolean;
  /** 最后一次见到该设备活动；远端设备靠 sync 推过来的 last_seen_at，未同步过为 null */
  lastSeenAt: string | null;
}

export type DeviceFilterValue = string | "all";

interface DeviceFilterState {
  devices: Device[];
  selected: DeviceFilterValue;
  /** 给 API hook 用：selected==="all" 返回 undefined（聚合所有设备） */
  selectedDeviceId: string | undefined;
  selfId: string | null;
  setSelected: (id: DeviceFilterValue) => void;
  renameSelf: (newName: string) => void;
  recolorSelf: (color: string) => void;
  reiconSelf: (icon: string) => void;
  /** 当前 self 设备最新元信息（来自 devices 表） */
  self: Device | null;
  reload: () => Promise<void>;
}

const DeviceFilterContext = createContext<DeviceFilterState | null>(null);

const SELECTED_KEY = "hindsight.device.selected";

function rowToDevice(row: DeviceRow): Device {
  return {
    id: row.deviceId,
    name: row.displayName,
    color: row.color,
    icon: row.icon,
    current: row.isSelf,
    lastSeenAt: row.lastSeenAt,
  };
}

export function DeviceFilterProvider({ children }: { children: ReactNode }) {
  const [devices, setDevices] = useState<Device[]>([]);
  const [selected, setSelectedState] = useState<DeviceFilterValue>(
    () => (localStorage.getItem(SELECTED_KEY) as DeviceFilterValue) ?? "self",
  );

  const reload = useCallback(async () => {
    try {
      const rows = await api.listDevices();
      setDevices(rows.map(rowToDevice));
    } catch (e) {
      logError("devices.list", e);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const self = useMemo<Device | null>(
    () => devices.find((d) => d.current) ?? null,
    [devices],
  );
  const selfId = self?.id ?? null;

  const setSelected = useCallback((id: DeviceFilterValue) => {
    setSelectedState(id);
    localStorage.setItem(SELECTED_KEY, id);
  }, []);

  // 兼容老的 "self" 字面值：第一次加载到真实 selfId 后把 "self" 替换为真实 UUID
  useEffect(() => {
    if (selected === "self" && selfId) {
      setSelected(selfId);
    }
  }, [selected, selfId, setSelected]);

  // selected 指向已不存在的设备 → 回落到 self
  useEffect(() => {
    if (devices.length === 0) return;
    if (selected === "all") return;
    if (selected === "self") return; // 等下一轮 useEffect 替换
    if (!devices.some((d) => d.id === selected) && selfId) {
      setSelected(selfId);
    }
  }, [devices, selected, selfId, setSelected]);

  const selectedDeviceId = useMemo(() => {
    if (selected === "all") return undefined;
    if (selected === "self") return selfId ?? undefined;
    return selected;
  }, [selected, selfId]);

  const renameSelf = useCallback(
    (newName: string) => {
      const trimmed = newName.trim();
      if (!trimmed) return;
      void api
        .updateSelfDevice(trimmed, undefined, undefined)
        .then((row) => {
          setDevices((prev) =>
            prev.map((d) => (d.id === row.deviceId ? rowToDevice(row) : d)),
          );
        })
        .catch((e) => logError("devices.renameSelf", e));
    },
    [],
  );

  const recolorSelf = useCallback((color: string) => {
    void api
      .updateSelfDevice(undefined, color, undefined)
      .then((row) => {
        setDevices((prev) =>
          prev.map((d) => (d.id === row.deviceId ? rowToDevice(row) : d)),
        );
      })
      .catch((e) => logError("devices.recolorSelf", e));
  }, []);

  const reiconSelf = useCallback((icon: string) => {
    void api
      .updateSelfDevice(undefined, undefined, icon)
      .then((row) => {
        setDevices((prev) =>
          prev.map((d) => (d.id === row.deviceId ? rowToDevice(row) : d)),
        );
      })
      .catch((e) => logError("devices.reiconSelf", e));
  }, []);

  // memo 化与其它四个 provider 对齐：所有回调已 useCallback 稳定，self/selectedDeviceId
  // 已 useMemo，避免每次 render 造新 value 把全部消费方（Today/Week/Month/DevicePicker…）带着重渲染
  const value = useMemo<DeviceFilterState>(
    () => ({
      devices,
      selected,
      selectedDeviceId,
      selfId,
      setSelected,
      renameSelf,
      recolorSelf,
      reiconSelf,
      self,
      reload,
    }),
    [
      devices,
      selected,
      selectedDeviceId,
      selfId,
      setSelected,
      renameSelf,
      recolorSelf,
      reiconSelf,
      self,
      reload,
    ],
  );

  return (
    <DeviceFilterContext.Provider value={value}>
      {children}
    </DeviceFilterContext.Provider>
  );
}

// hook 跟 Provider 同文件是有意为之——消费方一次 import 解决，dev 期 fast refresh
// 在改动 Provider 时退化为整页刷新（state 文件极少改动，影响可接受）。
// eslint-disable-next-line react-refresh/only-export-components
export function useDeviceFilter(): DeviceFilterState {
  const ctx = useContext(DeviceFilterContext);
  if (!ctx) throw new Error("useDeviceFilter must be used within DeviceFilterProvider");
  return ctx;
}
