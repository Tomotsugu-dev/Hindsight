import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

/** 单台设备 */
export interface Device {
  id: string;
  name: string;
  /** 是否本机 */
  current: boolean;
}

/** 选中目标：设备 id 或 "all" 表示所有设备 */
export type DeviceFilterValue = string | "all";

interface DeviceFilterState {
  /** 所有已知设备（含本机） */
  devices: Device[];
  /** 当前筛选选中 */
  selected: DeviceFilterValue;
  /** 切换选中 */
  setSelected: (id: DeviceFilterValue) => void;
  /** 给本机改名 */
  renameSelf: (newName: string) => void;
}

const DeviceFilterContext = createContext<DeviceFilterState | null>(null);

/** 占位数据 — Phase C 后端上线后从 invoke("list_devices") 拿 */
const MOCK_DEVICES: Device[] = [
  { id: "self", name: "我的电脑", current: true },
  // { id: "laptop", name: "工作笔记本", current: false },  // 取消注释可演示多设备
];

const SELF_NAME_KEY = "hindsight.device.selfName";
const SELECTED_KEY = "hindsight.device.selected";

export function DeviceFilterProvider({ children }: { children: ReactNode }) {
  // 本机名 — 持久化到 localStorage（Phase C 改成 tauri-plugin-store）
  const [selfName, setSelfName] = useState<string>(() => {
    return localStorage.getItem(SELF_NAME_KEY) ?? "我的电脑";
  });

  // 当前筛选 — 持久化
  const [selected, setSelectedState] = useState<DeviceFilterValue>(() => {
    return (localStorage.getItem(SELECTED_KEY) as DeviceFilterValue) ?? "self";
  });

  // 把当前 selfName 注入 mock 列表
  const devices = useMemo<Device[]>(() => {
    return MOCK_DEVICES.map((d) =>
      d.current ? { ...d, name: selfName } : d,
    );
  }, [selfName]);

  const setSelected = useCallback((id: DeviceFilterValue) => {
    setSelectedState(id);
    localStorage.setItem(SELECTED_KEY, id);
  }, []);

  const renameSelf = useCallback((newName: string) => {
    const trimmed = newName.trim();
    if (!trimmed) return;
    setSelfName(trimmed);
    localStorage.setItem(SELF_NAME_KEY, trimmed);
  }, []);

  // selected 失效时（设备被移除）兜底回退到 self
  useEffect(() => {
    if (selected === "all") return;
    if (!devices.some((d) => d.id === selected)) {
      setSelected("self");
    }
  }, [devices, selected, setSelected]);

  const value: DeviceFilterState = {
    devices,
    selected,
    setSelected,
    renameSelf,
  };

  return (
    <DeviceFilterContext.Provider value={value}>
      {children}
    </DeviceFilterContext.Provider>
  );
}

export function useDeviceFilter(): DeviceFilterState {
  const ctx = useContext(DeviceFilterContext);
  if (!ctx) throw new Error("useDeviceFilter must be used within DeviceFilterProvider");
  return ctx;
}
