/** The `retentionDays` value that keeps screenshots forever. The backend uses the same value. */
export const KEEP_FOREVER_DAYS = 36500;

/** The longest retention in days (2 years). The backend uses the same value. */
export const RETENTION_MAX_DAYS = 730;

/** Where the retention slider snaps: days apart at the start, months apart at the end, then forever. */
export const RETENTION_STOPS: readonly number[] = [
  1,
  3,
  7,
  14,
  30,
  60,
  90,
  180,
  365,
  RETENTION_MAX_DAYS,
  KEEP_FOREVER_DAYS,
];
