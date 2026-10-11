/**
 * Tailwind CSS 3 + daisyUI 4 配置（P6 引入）。
 *
 * 说明：
 * - GUI（src/）通过 Vite + PostCSS 使用 daisyUI 组件类（btn/card/badge/alert/modal…）。
 * - HTML 审核报告由 Rust 端 `format_core::report` 生成，不经过本配置；
 *   其内联 CSS 是 daisyUI light 主题同名 class 的构建期裁剪子集，
 *   升级 daisyUI 版本时需同步刷新 report.rs 中的 CSS 常量。
 */
/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{svelte,ts}"],
  theme: {
    extend: {},
  },
  plugins: [require("daisyui")],
  daisyui: {
    // 只启用浅色主题：桌面应用面向校内用户，保持与报告配色一致。
    themes: ["light"],
    logs: false,
  },
};
