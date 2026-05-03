import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { X } from "lucide-react";
import { api, type AppGroup, type AppGroupMember } from "../../api/hindsight";
import { AppIcon } from "../../components/AppIcon/AppIcon";
import { useCategories } from "../../state/categories";
import { useDeviceFilter, type Device } from "../../state/deviceFilter";
import { displayAppName } from "../../utils/displayName";
import styles from "./Pairing.module.css";

/** 把 group + 设备列表换算成「每个 device 列对应哪个 member（如果有）」的 lookup */
function membersByDevice(group: AppGroup, devices: Device[]): (AppGroupMember | null)[] {
  return devices.map((d) => {
    // 优先匹配 last_device_id；如果某个 member 的 last_device_id 是这个设备就放进来
    return group.members.find((m) => m.lastDeviceId === d.id) ?? null;
  });
}

function fmtDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.round(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rem = m % 60;
  return rem === 0 ? `${h}h` : `${h}h${rem}m`;
}

export function PairingSection() {
  const { devices } = useDeviceFilter();
  const { getCategory } = useCategories();
  const [groups, setGroups] = useState<AppGroup[] | null>(null);
  const [draggingProcessName, setDraggingProcessName] = useState<string | null>(null);
  const [hoverGroupId, setHoverGroupId] = useState<string | null>(null);
  const [pendingNames, setPendingNames] = useState<Record<string, string>>({});

  const reload = async () => {
    try {
      const list = await api.listAppGroups();
      setGroups(list);
    } catch (e) {
      console.error("listAppGroups 失败:", e);
      setGroups([]);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  // self 设备先排前面，其他设备按名字稳定排序
  const sortedDevices = useMemo<Device[]>(() => {
    const arr = [...devices];
    arr.sort((a, b) => {
      if (a.current && !b.current) return -1;
      if (!a.current && b.current) return 1;
      return a.name.localeCompare(b.name);
    });
    return arr;
  }, [devices]);

  // 过滤掉永远没有任何成员（被软删 / 全成员被 unmerge 走）的空组，避免列表里出现一行全是「—」
  const visibleGroups = useMemo(() => {
    if (!groups) return null;
    return groups
      .filter((g) => g.members.length > 0)
      .filter((g) =>
        g.members.some((m) => sortedDevices.some((d) => d.id === m.lastDeviceId)),
      );
  }, [groups, sortedDevices]);

  if (visibleGroups === null) {
    return <div className={styles.toolbar}>加载中…</div>;
  }
  if (sortedDevices.length === 0) {
    return <div className={styles.toolbar}>还没有设备数据。启动一段时间后再来看。</div>;
  }

  // 所有列（每台设备 + 统一名）等宽 1fr；操作列 auto 自适应。
  const deviceColsTemplate = sortedDevices.map(() => "1fr").join(" ");
  const cssVars = { "--device-cols": deviceColsTemplate } as CSSProperties;

  const onDrop = async (targetGroupId: string) => {
    const src = draggingProcessName;
    setDraggingProcessName(null);
    setHoverGroupId(null);
    if (!src) return;
    try {
      await api.mergeAppGroup(src, targetGroupId);
      await reload();
    } catch (e) {
      console.error("merge 失败:", e);
    }
  };

  const onUnmerge = async (processName: string) => {
    try {
      await api.unmergeAppGroup(processName);
      await reload();
    } catch (e) {
      console.error("unmerge 失败:", e);
    }
  };

  const onCommitName = async (groupId: string) => {
    const next = pendingNames[groupId];
    if (next === undefined) return;
    const trimmed = next.trim();
    if (!trimmed) {
      setPendingNames((p) => {
        const cp = { ...p };
        delete cp[groupId];
        return cp;
      });
      return;
    }
    try {
      await api.renameAppGroup(groupId, trimmed);
      await reload();
    } catch (e) {
      console.error("rename 失败:", e);
    } finally {
      setPendingNames((p) => {
        const cp = { ...p };
        delete cp[groupId];
        return cp;
      });
    }
  };

  return (
    <div className={styles.pairing} style={cssVars}>
      <div className={styles.toolbar}>
        把左边的应用拖到右边一行里，就能合并成一组（跨平台 / 跨设备同名应用）。一组共享一个分类。
      </div>

      <div className={styles.devHeader}>
        {sortedDevices.map((d) => (
          <span key={d.id} className={styles.devHeaderName}>
            {d.name}
          </span>
        ))}
        <span className={styles.devHeaderName}>统一名</span>
        <span className={styles.devHeaderActionPad} />
      </div>

      {visibleGroups.map((group) => {
        const slots = membersByDevice(group, sortedDevices);
        const isPaired = group.members.length > 1;
        const isHover = hoverGroupId === group.id;
        const cat = group.categoryId ? getCategory(group.categoryId) : null;
        const nameDraft = pendingNames[group.id] ?? group.displayName;

        return (
          <div
            key={group.id}
            className={[
              styles.row,
              isPaired ? styles.paired : "",
              isHover ? styles.dropTarget : "",
            ]
              .filter(Boolean)
              .join(" ")}
            onDragOver={(e) => {
              if (!draggingProcessName) return;
              // 只在源 process 不属于本 group 时允许 drop（拖回原位无意义）
              const sourceInThisGroup = group.members.some(
                (m) => m.processName === draggingProcessName,
              );
              if (sourceInThisGroup) return;
              e.preventDefault();
              if (hoverGroupId !== group.id) setHoverGroupId(group.id);
            }}
            onDragLeave={() => {
              if (hoverGroupId === group.id) setHoverGroupId(null);
            }}
            onDrop={(e) => {
              e.preventDefault();
              void onDrop(group.id);
            }}
          >
            {slots.map((member, idx) => {
              const dev = sortedDevices[idx];
              if (!member) {
                return (
                  <div key={dev.id} className={`${styles.devCol} ${styles.empty}`}>
                    —
                  </div>
                );
              }
              const isDragging = draggingProcessName === member.processName;
              return (
                <div
                  key={dev.id}
                  className={styles.devCol}
                  draggable
                  onDragStart={(e) => {
                    setDraggingProcessName(member.processName);
                    e.dataTransfer.setData("text/plain", member.processName);
                    e.dataTransfer.effectAllowed = "move";
                  }}
                  onDragEnd={() => {
                    setDraggingProcessName(null);
                    setHoverGroupId(null);
                  }}
                >
                  <span
                    className={`${styles.chip} ${isDragging ? styles.dragging : ""}`}
                  >
                    <AppIcon
                      processName={member.processName}
                      fallbackColor="#94a3b8"
                      size={14}
                    />
                    <span className={styles.chipName} title={member.processName}>
                      {displayAppName(member.processName)}
                    </span>
                    <span className={styles.chipMeta}>{fmtDuration(member.recentSecs)}</span>
                    {isPaired && (
                      <button
                        type="button"
                        className={styles.chipUnmerge}
                        onClick={() => void onUnmerge(member.processName)}
                        title="从该组移出"
                      >
                        <X size={11} strokeWidth={2.25} />
                      </button>
                    )}
                  </span>
                </div>
              );
            })}

            <div className={styles.nameCol}>
              <input
                className={styles.nameInput}
                value={nameDraft}
                onChange={(e) =>
                  setPendingNames((p) => ({ ...p, [group.id]: e.target.value }))
                }
                onBlur={() => void onCommitName(group.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.currentTarget.blur();
                  } else if (e.key === "Escape") {
                    setPendingNames((p) => {
                      const cp = { ...p };
                      delete cp[group.id];
                      return cp;
                    });
                    e.currentTarget.blur();
                  }
                }}
              />
            </div>

            <div className={styles.actionCol}>
              {cat && (
                <span
                  className={styles.catTag}
                  style={{
                    background: `color-mix(in oklab, ${cat.color} 18%, white)`,
                    color: `color-mix(in oklab, ${cat.color} 60%, black)`,
                  }}
                >
                  {cat.name}
                </span>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
