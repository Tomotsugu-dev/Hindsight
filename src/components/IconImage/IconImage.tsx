import { useState } from "react";
import { cachedVerdict, judgeIcon, readIconPixels } from "./darkIcons";
import styles from "./IconImage.module.css";

interface IconImageProps {
  /** Image address, ready for `<img src>`. */
  src: string;
  size: number;
  alt: string;
  className?: string;
  onError?: () => void;
}

/**
 * The image of an app or website icon. An icon that is hard to see on the dark card gets a
 * light backing in dark mode.
 *
 * The image loads with CORS, or the canvas refuses to give its pixels. The asset protocol and
 * the demo's icon CDN both allow it.
 */
export function IconImage({ src, size, alt, className, onError }: IconImageProps) {
  const [judged, setJudged] = useState<{ src: string; dark: boolean } | null>(null);
  const dark = judged?.src === src ? judged.dark : (cachedVerdict(src) ?? false);

  return (
    // onLoad and onError are not user actions: the rule takes them for clicks
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <img
      className={[className, dark && styles.dark].filter(Boolean).join(" ")}
      crossOrigin="anonymous"
      src={src}
      width={size}
      height={size}
      alt={alt}
      // A draggable image takes the drag for itself, so a draggable parent would not get it
      draggable={false}
      onLoad={(e) => {
        const img = e.currentTarget;
        setJudged({ src, dark: judgeIcon(src, () => readIconPixels(img)) });
      }}
      onError={onError}
    />
  );
}
