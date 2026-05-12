// 一次性迁移脚本：把 i18n JSON 中 `aiSummary.quick.analysis.*` 下
// `foo` + `foo.v2` + `foo.v3` 这种 flat 兄弟 key 重组成嵌套 `foo: { v1, v2, v3 }`。
// i18next 默认按 `.` 切嵌套，flat 形式根本拿不到 .v2，所以必须嵌套。
//
// 用法：node scripts/migrate-quick-variants.mjs
// 幂等：跑两次结果相同；只迁移 analysis 子树，其它 key 不动。

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

function migrateNode(obj) {
  if (obj == null || typeof obj !== "object" || Array.isArray(obj)) return;
  // 先递归子节点（确保嵌套对象内部也被处理）
  for (const v of Object.values(obj)) migrateNode(v);

  // 找到所有 `foo.v2` / `foo.v3` 形式的 key，合并到 `foo` 上
  const variantKeys = Object.keys(obj).filter((k) => /\.v\d+$/.test(k));
  if (variantKeys.length === 0) return;

  const grouped = new Map(); // base -> { v2: "...", v3: "..." }
  for (const k of variantKeys) {
    const m = k.match(/^(.+)\.v(\d+)$/);
    if (!m) continue;
    const base = m[1];
    const n = parseInt(m[2], 10);
    if (!grouped.has(base)) grouped.set(base, {});
    grouped.get(base)[`v${n}`] = obj[k];
  }

  for (const [base, variants] of grouped) {
    // 如果 baseKey 本身仍是 string，把它升格为 { v1: <原 string>, v2, v3 }
    const baseVal = obj[base];
    if (typeof baseVal === "string") {
      obj[base] = { v1: baseVal, ...variants };
    } else if (typeof baseVal === "object" && baseVal != null) {
      // 已经是 object（部分迁移过了），补 v2/v3 上去
      Object.assign(baseVal, variants);
    }
    // 删除 flat 的 .v2 / .v3 sibling
    for (const v of Object.keys(variants)) {
      delete obj[`${base}.${v}`];
    }
  }
}

for (const lang of ["zh-CN", "en", "ja"]) {
  const file = path.join(ROOT, "src", "i18n", "locales", `${lang}.json`);
  const json = JSON.parse(fs.readFileSync(file, "utf8"));
  // 只迁移 aiSummary.quick.analysis 子树（其它地方不该有 .v2 兄弟 key 模式）
  if (json.aiSummary?.quick?.analysis) {
    migrateNode(json.aiSummary.quick.analysis);
  }
  fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n", "utf8");
  console.log(`migrated ${lang}`);
}
