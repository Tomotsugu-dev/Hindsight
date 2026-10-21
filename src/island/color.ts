// Colors on the island's black ground. A plugin can pass the colors users picked for their
// categories, so a color too dark to see on black is made lighter here instead of refused.

/** WCAG's minimum contrast for body text */
const MIN_CONTRAST = 4.5;

function linear(channel: number): number {
  const c = channel / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** The contrast of `rgb` against black */
function contrastOnBlack([r, g, b]: number[]): number {
  const luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
  return (luminance + 0.05) / 0.05;
}

const toHex = (rgb: number[]) => `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;

/** Returns `hex` (#rrggbb) mixed with just enough white to have a 4.5:1 contrast on black. */
export function readable(hex: string): string {
  const rgb = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  for (let step = 0; step <= 20; step++) {
    const mixed = rgb.map((c) => Math.round(c + ((255 - c) * step) / 20));
    if (contrastOnBlack(mixed) >= MIN_CONTRAST) return toHex(mixed);
  }
  return "#ffffff";
}

export const testing = { contrastOnBlack };
