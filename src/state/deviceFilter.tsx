import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

export interface Device {
  id: string;
  name: string;
  color: string;
  icon: string;
  current: boolean;
}

export type DeviceFilterValue = string | "all";

interface DeviceFilterState {
  devices: Device[];
  selected: DeviceFilterValue;
  setSelected: (id: DeviceFilterValue) => void;
  renameSelf: (newName: string) => void;
  recolorSelf: (color: string) => void;
  reiconSelf: (icon: string) => void;
}

const DeviceFilterContext = createContext<DeviceFilterState | null>(null);

const MOCK_DEVICES: Device[] = [
  { id: "self", name: "我的电脑", color: "#60a5fa", icon: "Monitor", current: true },
];

const SELF_NAME_KEY = "hindsight.device.selfName";
const SELF_COLOR_KEY = "hindsight.device.selfColor";
const SELF_ICON_KEY = "hindsight.device.selfIcon";
const SELECTED_KEY = "hindsight.device.selected";

export function DeviceFilterProvider({ children }: { children: ReactNode }) {
  const [selfName, setSelfName] = useState<string>(
    () => localStorage.getItem(SELF_NAME_KEY) ?? "我的电脑",
  );
  const [selfColor, setSelfColor] = useState<string>(
    () => localStorage.getItem(SELF_COLOR_KEY) ?? "#60a5fa",
  );
  const [selfIcon, setSelfIcon] = useState<string>(
    () => localStorage.getItem(SELF_ICON_KEY) ?? "Monitor",
  );

  const [selected, setSelectedState] = useState<DeviceFilterValue>(
    () => (localStorage.getItem(SELECTED_KEY) as DeviceFilterValue) ?? "self",
  );

  const devices = useMemo<Device[]>(() => {
    return MOCK_DEVICES.map((d) =>
      d.current ? { ...d, name: selfName, color: selfColor, icon: selfIcon } : d,
    );
  }, [selfName, selfColor, selfIcon]);

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

  const recolorSelf = useCallback((color: string) => {
    setSelfColor(color);
    localStorage.setItem(SELF_COLOR_KEY, color);
  }, []);

  const reiconSelf = useCallback((icon: string) => {
    setSelfIcon(icon);
    localStorage.setItem(SELF_ICON_KEY, icon);
  }, []);

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
    recolorSelf,
    reiconSelf,
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
