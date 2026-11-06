// Tells whether an icon is hard to see on the dark card, from its pixels.
// Luminance and contrast follow the WCAG 2 definitions.

/** Pixels more opaque than this are part of the icon. The rest show the card behind it. */
const OPAQUE_ALPHA = 128;

/**
 * A pixel with at least this contrast against the card can be seen. WCAG asks 3 for graphics,
 * but that bar is for seeing clearly: red and blue icons fall under it and are still easy to see.
 */
const VISIBLE_CONTRAST = 2;

/** An icon is dark when fewer than this share of its pixels can be seen. */
const MIN_VISIBLE_SHARE = 0.35;

/** The light of one sRGB channel (0–255), from 0 to 1. */
function linear(channel: number): number {
  const v = channel / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

/** Relative luminance of an sRGB color: 0 for black, 1 for white. */
export function relativeLuminance(r: number, g: number, b: number): number {
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** Contrast between two luminances, from 1 (the same) to 21 (black and white), in either order. */
export function contrastRatio(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** The card color in dark mode, `#16161a`. */
const DARK_CARD = relativeLuminance(0x16, 0x16, 0x1a);

/**
 * Whether an icon is hard to see on the dark card. `pixels` is RGBA data, four bytes a pixel.
 *
 * A fully transparent icon is not dark: a backing would show as an empty light square.
 */
export function isDarkIcon(pixels: Uint8ClampedArray): boolean {
  let opaque = 0;
  let visible = 0;
  for (let i = 0; i < pixels.length; i += 4) {
    if (pixels[i + 3] <= OPAQUE_ALPHA) continue;
    opaque++;
    const luminance = relativeLuminance(pixels[i], pixels[i + 1], pixels[i + 2]);
    if (contrastRatio(luminance, DARK_CARD) >= VISIBLE_CONTRAST) visible++;
  }
  return opaque > 0 && visible / opaque < MIN_VISIBLE_SHARE;
}
