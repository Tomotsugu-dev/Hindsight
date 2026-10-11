import styles from "./TrafficLights.module.css";

/** macOS 窗口左上角的红绿灯，画在原生窗口放它的位置。网页里没有这三个按钮，
 *  文档截图又要看起来像 macOS 应用，所以由 demo 自己画。 */
export function TrafficLights() {
  return (
    <div className={styles.lights} aria-hidden>
      <span className={styles.close} />
      <span className={styles.minimize} />
      <span className={styles.zoom} />
    </div>
  );
}
