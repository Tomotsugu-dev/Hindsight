/** The shortest screenshot interval in seconds. */
export const INTERVAL_MIN_SECS = 5;

/** The longest screenshot interval in seconds (10 minutes). The backend allows up to 600. */
export const INTERVAL_MAX_SECS = 600;

/** Where the screenshot interval slider snaps: seconds apart at the start, minutes apart at the end. */
export const INTERVAL_STOPS: readonly number[] = [
  INTERVAL_MIN_SECS,
  10,
  15,
  30,
  60,
  120,
  300,
  INTERVAL_MAX_SECS,
];
