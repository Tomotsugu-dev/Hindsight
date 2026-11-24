/**
 * Returns where a value sits on a track that spaces the stops evenly: the index of a stop, or a
 * fraction between two stops. Stops must be in increasing order.
 */
export function stopPosition(value: number, stops: readonly number[]): number {
  if (value <= stops[0]) return 0;
  const last = stops.length - 1;
  for (let i = 0; i < last; i++) {
    const lo = stops[i];
    const hi = stops[i + 1];
    if (value <= hi) return i + (value - lo) / (hi - lo);
  }
  return last;
}

/** Returns the stop nearest to a track position. */
export function nearestStop(position: number, stops: readonly number[]): number {
  const i = Math.min(stops.length - 1, Math.max(0, Math.round(position)));
  return stops[i];
}

/** Returns the next stop above or below a value, or the value itself at either end. */
export function adjacentStop(value: number, stops: readonly number[], direction: 1 | -1): number {
  if (direction > 0) return stops.find((s) => s > value) ?? value;
  for (let i = stops.length - 1; i >= 0; i--) {
    if (stops[i] < value) return stops[i];
  }
  return value;
}
