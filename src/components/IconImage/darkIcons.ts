import { isDarkIcon } from "../../lib/iconContrast";
import { lruInsert } from "../../utils/lru";

/** The icon is drawn into a square this big before its pixels are read. */
const SAMPLE_PX = 32;

/** Verdicts kept, by image address. Enough for the sites page with hundreds of icons. */
const MAX_VERDICTS = 512;

const verdicts = new Map<string, boolean>();

/** Whether the icon at `src` is dark, or `undefined` when it has not been judged. */
export function cachedVerdict(src: string): boolean | undefined {
  return verdicts.get(src);
}

/**
 * Whether the icon at `src` is dark. Calls `read` the first time only and keeps the result.
 * An icon whose pixels cannot be read (`read` returns null) is not dark.
 */
export function judgeIcon(src: string, read: () => Uint8ClampedArray | null): boolean {
  const known = verdicts.get(src);
  if (known !== undefined) return known;
  const pixels = read();
  const dark = pixels !== null && isDarkIcon(pixels);
  lruInsert(verdicts, src, dark, MAX_VERDICTS);
  return dark;
}

/**
 * The RGBA pixels of a loaded image, drawn into a SAMPLE_PX square with its shape kept.
 * Null when the canvas refuses to give them, as for an image from another origin without CORS.
 */
export function readIconPixels(img: HTMLImageElement): Uint8ClampedArray | null {
  // An SVG without its own size reports 0
  const w = img.naturalWidth || SAMPLE_PX;
  const h = img.naturalHeight || SAMPLE_PX;
  const scale = SAMPLE_PX / Math.max(w, h);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = SAMPLE_PX;
    canvas.height = SAMPLE_PX;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    const dw = w * scale;
    const dh = h * scale;
    ctx.drawImage(img, (SAMPLE_PX - dw) / 2, (SAMPLE_PX - dh) / 2, dw, dh);
    return ctx.getImageData(0, 0, SAMPLE_PX, SAMPLE_PX).data;
  } catch {
    return null;
  }
}
