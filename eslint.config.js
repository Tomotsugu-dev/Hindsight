// ESLint flat config (eslint v9) for Hindsight 前端
// 策略：现有代码量较大，所有非致命规则先 warn，build 不阻塞；后续 phase 修代码时渐进升级到 error。

import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import jsxA11y from "eslint-plugin-jsx-a11y";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/target/**",
      "src-tauri/gen/**",
      ".venv/**",
      "scripts/**",
      "public/**",
      "*.config.js",
    ],
  },
  // 基础配置（仅 src/）
  js.configs.recommended,
  ...tseslint.configs.recommended,
  // —— 类型感知层（仅 src）——
  // projectService 让 typescript-eslint 拿到类型信息，开启只有类型才能查的规则
  // （no-floating-promises / no-misused-promises 等）。这是重异步 Tauri 应用最高价值的
  // lint 升级——能抓住一整类漏 await 的 bug，AST-only 规则看不到。
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // 高价值：漏 await / promise 误用，保 error（这是开类型感知层的主要目的）
      "@typescript-eslint/no-floating-promises": "error",
      // async 事件处理器（onClick={() => x.minimize()} 等）是 React 惯用法，
      // 不视为 misuse；其余真正危险的 void-return 误用仍保 error
      "@typescript-eslint/no-misused-promises": [
        "error",
        { checksVoidReturn: { attributes: false } },
      ],
      // 以下两条偏噪声/风格，各仅 1 处，关掉避免污染 ratchet（与"先放后渐进"策略一致）：
      // - restrict-template-expressions：模板里插非 string/number 的风格约束
      // - no-redundant-type-constituents：DeviceFilterValue = string | "all" 的语义化冗余
      "@typescript-eslint/restrict-template-expressions": "off",
      "@typescript-eslint/no-redundant-type-constituents": "off",
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2020,
      globals: { ...globals.browser },
      parserOptions: {
        ecmaFeatures: { jsx: true },
      },
    },
    settings: {
      react: { version: "detect" },
    },
    plugins: {
      react,
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
      "jsx-a11y": jsxA11y,
    },
    rules: {
      // React 推荐
      ...react.configs.flat.recommended.rules,
      ...react.configs.flat["jsx-runtime"].rules,
      ...reactHooks.configs.recommended.rules,
      // a11y：放宽到 warn，配合 P3 a11y 改造
      ...jsxA11y.flatConfigs.recommended.rules,

      // React 19 + TS 项目惯例
      "react/prop-types": "off",
      "react/react-in-jsx-scope": "off",
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],

      // 现存代码大量使用 (e: unknown) catch + 下划线参数；先 warn
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/no-empty-object-type": "warn",
      "@typescript-eslint/ban-ts-comment": "warn",

      // a11y 系列全降级到 warn，作为 P3 增量改造的基线
      "jsx-a11y/no-autofocus": "warn",
      "jsx-a11y/click-events-have-key-events": "warn",
      "jsx-a11y/no-static-element-interactions": "warn",
      "jsx-a11y/label-has-associated-control": "warn",
      "jsx-a11y/no-noninteractive-element-interactions": "warn",
      "jsx-a11y/no-noninteractive-tabindex": "warn",
      "jsx-a11y/no-noninteractive-element-to-interactive-role": "warn",
      "jsx-a11y/anchor-is-valid": "warn",

      // rules-of-hooks 是 React 正确性的根本，必须保 error
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
    },
  },
  // Vite / Vitest 配置文件：Node 环境，且关掉类型感知规则
  // （它们不在 src 工程内，开 type-checked 会报 "not in project"）
  {
    files: ["vite.config.ts", "vitest.config.ts"],
    extends: [tseslint.configs.disableTypeChecked],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  // Prettier 关闭格式相关规则（必须最后）
  prettier
);
