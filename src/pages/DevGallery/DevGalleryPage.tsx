// Development only: the route exists only when import.meta.env.DEV, so this page is not in a
// release build. Its text is for developers and is not translated.

import { useState, type CSSProperties } from "react";
import { Download, ListFilter, MoreHorizontal, Search, Trash2, X } from "lucide-react";
import {
  Button,
  type ButtonSize,
  type ButtonTone,
  type ButtonVariant,
} from "../../components/Button/Button";
import { IconButton } from "../../components/Button/IconButton";
import { TextInput } from "../../components/TextInput/TextInput";
import { Checkbox } from "../../components/Checkbox/Checkbox";
import { SimplePicker } from "../../components/SimplePicker/SimplePicker";
import styles from "./DevGalleryPage.module.css";

const VARIANTS: ButtonVariant[] = ["solid", "soft", "outline", "ghost"];
const TONES: ButtonTone[] = ["accent", "neutral", "danger"];
const SIZES: ButtonSize[] = ["xs", "sm", "md", "lg"];
const LEVELS = [
  "display",
  "headline",
  "title",
  "body-lg",
  "body",
  "body-sm",
  "caption",
  "micro",
] as const;
const PERIODS = [
  { value: "day", label: "Day" },
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
];

/**
 * Every shared control in each look, size and state, to check a change or a theme at a glance.
 * Open it with Cmd/Ctrl+Shift+G.
 *
 * The sliders set --font-scale and --density on this page only, the way a theme can set them
 * on one page.
 */
export default function DevGalleryPage() {
  const [fontScale, setFontScale] = useState(1);
  const [density, setDensity] = useState(1);
  const [period, setPeriod] = useState("week");
  const scale = { "--font-scale": fontScale, "--density": density } as CSSProperties;

  return (
    <div className={styles.page} style={scale} data-page="dev-gallery">
      <header className={styles.header}>
        <h1 className={styles.title}>Components</h1>
        <label className={styles.knob}>
          --font-scale {fontScale.toFixed(2)}
          <input
            type="range"
            min={0.8}
            max={1.4}
            step={0.05}
            value={fontScale}
            onChange={(e) => setFontScale(Number(e.target.value))}
          />
        </label>
        <label className={styles.knob}>
          --density {density.toFixed(2)}
          <input
            type="range"
            min={0.7}
            max={1.3}
            step={0.05}
            value={density}
            onChange={(e) => setDensity(Number(e.target.value))}
          />
        </label>
      </header>

      <section className={styles.section}>
        <h2 className={styles.heading}>Text levels</h2>
        {LEVELS.map((level) => (
          <p
            key={level}
            className={styles.level}
            // Each row shows its own level, so the size is computed here
            style={{ fontSize: `calc(var(--font-size-${level}) * var(--font-scale))` }}
          >
            <code>{level}</code> The quick brown fox · 敏捷的狐狸 · 素早い狐
          </p>
        ))}
      </section>

      <section className={styles.section}>
        <h2 className={styles.heading}>Button: variant × tone</h2>
        <div className={styles.grid}>
          {TONES.map((tone) =>
            VARIANTS.map((variant) => (
              <Button key={`${tone}-${variant}`} variant={variant} tone={tone}>
                {variant} {tone}
              </Button>
            )),
          )}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.heading}>Button: sizes and states</h2>
        {SIZES.map((size) => (
          <div key={size} className={styles.row}>
            <Button size={size} variant="solid" tone="accent">
              Save
            </Button>
            <Button size={size} icon={Download}>
              Export
            </Button>
            <Button size={size} variant="outline" tone="danger" icon={Trash2}>
              Uninstall
            </Button>
            <Button size={size} loading>
              Working
            </Button>
            <Button size={size} disabled>
              Disabled
            </Button>
            <IconButton size={size} icon={X} label="Close" />
            <IconButton size={size} icon={MoreHorizontal} label="More" variant="soft" />
            {/* Pickers and inputs have no xs */}
            {size !== "xs" ? (
              <>
                <SimplePicker size={size} value={period} options={PERIODS} onChange={setPeriod} />
                <TextInput size={size} icon={Search} placeholder="Search" aria-label="Search" />
              </>
            ) : null}
          </div>
        ))}
      </section>

      <section className={styles.section}>
        <h2 className={styles.heading}>Button: quiet (gray until hovered)</h2>
        <div className={styles.grid}>
          {TONES.map((tone) =>
            VARIANTS.filter((variant) => variant !== "solid").map((variant) => (
              <Button key={`${tone}-${variant}`} variant={variant} tone={tone} quiet>
                {variant} {tone}
              </Button>
            )),
          )}
        </div>
      </section>

      <section className={styles.section}>
        <h2 className={styles.heading}>Inputs</h2>
        <div className={styles.row}>
          <TextInput placeholder="No icon" aria-label="No icon" />
          <TextInput icon={Search} placeholder="Disabled" aria-label="Disabled" disabled />
          <SimplePicker value={period} options={PERIODS} onChange={setPeriod} icon={ListFilter} />
        </div>
        <div className={styles.column}>
          <Checkbox defaultChecked>Checked</Checkbox>
          <Checkbox>Not checked</Checkbox>
          <Checkbox disabled defaultChecked>
            Disabled
          </Checkbox>
          <Checkbox defaultChecked>
            A long label that wraps onto a second line, to check that the box stays next to the
            first line and the text wraps under itself
          </Checkbox>
        </div>
      </section>
    </div>
  );
}
