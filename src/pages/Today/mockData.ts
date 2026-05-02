/**
 * 占位数据 — 后续接入采集结果后从数据库读
 */

export interface HourSegment {
  categoryId: string;
  minutes: number;
}

export interface HourSlot {
  hour: number; // 0-23
  segments: HourSegment[];
}

export interface AppUsage {
  process: string;
  categoryId: string;
  minutes: number;
}

export interface WorkRange {
  /** 0-24 小数允许（如 9.5 = 09:30） */
  startHour: number;
  endHour: number;
}

/** 24 小时活动数据 — 8 点开始，22 点收工 */
export const MOCK_HOURS: HourSlot[] = [
  ...Array.from({ length: 8 }, (_, i) => ({ hour: i, segments: [] })),
  { hour: 8, segments: [{ categoryId: "browse", minutes: 9 }] },
  { hour: 9, segments: [
    { categoryId: "code", minutes: 38 },
    { categoryId: "browse", minutes: 14 },
    { categoryId: "talk", minutes: 5 },
  ]},
  { hour: 10, segments: [
    { categoryId: "code", minutes: 48 },
    { categoryId: "browse", minutes: 8 },
  ]},
  { hour: 11, segments: [
    { categoryId: "code", minutes: 32 },
    { categoryId: "talk", minutes: 18 },
    { categoryId: "browse", minutes: 6 },
  ]},
  { hour: 12, segments: [
    { categoryId: "fun", minutes: 22 },
    { categoryId: "browse", minutes: 10 },
  ]},
  { hour: 13, segments: [{ categoryId: "browse", minutes: 8 }] },
  { hour: 14, segments: [
    { categoryId: "design", minutes: 36 },
    { categoryId: "browse", minutes: 12 },
  ]},
  { hour: 15, segments: [
    { categoryId: "code", minutes: 42 },
    { categoryId: "design", minutes: 14 },
  ]},
  { hour: 16, segments: [
    { categoryId: "code", minutes: 35 },
    { categoryId: "talk", minutes: 12 },
    { categoryId: "browse", minutes: 8 },
  ]},
  { hour: 17, segments: [
    { categoryId: "code", minutes: 26 },
    { categoryId: "browse", minutes: 18 },
  ]},
  { hour: 18, segments: [
    { categoryId: "talk", minutes: 14 },
    { categoryId: "browse", minutes: 10 },
  ]},
  { hour: 19, segments: [
    { categoryId: "fun", minutes: 32 },
    { categoryId: "browse", minutes: 12 },
  ]},
  { hour: 20, segments: [
    { categoryId: "fun", minutes: 38 },
    { categoryId: "browse", minutes: 6 },
  ]},
  { hour: 21, segments: [
    { categoryId: "fun", minutes: 24 },
    { categoryId: "browse", minutes: 12 },
  ]},
  { hour: 22, segments: [{ categoryId: "browse", minutes: 14 }] },
  { hour: 23, segments: [] },
];

export const MOCK_TOP_APPS: AppUsage[] = [
  { process: "code.exe", categoryId: "code", minutes: 221 },
  { process: "chrome.exe", categoryId: "browse", minutes: 138 },
  { process: "Spotify.exe", categoryId: "fun", minutes: 84 },
  { process: "Figma.exe", categoryId: "design", minutes: 50 },
  { process: "WeChat.exe", categoryId: "talk", minutes: 49 },
];

/** 设置为 null 即可演示「未配置工作时段」 */
export const MOCK_WORK_HOURS: WorkRange[] | null = [
  { startHour: 9, endHour: 12 },
  { startHour: 14, endHour: 18 },
];
