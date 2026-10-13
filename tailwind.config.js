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
    extend: {
      // 设计 token：极简留白（OpenAI 风格）。颜色值以 src/app.css 的 CSS 变量为准，
      // 此处仅暴露给 Tailwind 工具类使用，禁止在组件里写死其他颜色。
      colors: {
        ink: "var(--ink)", // #0d0d0d 正文主色
        secondary: "var(--ink-2)", // #1a1a1a 次级文字
        subtle: "var(--muted)", // #6e6e6e 弱化文字
        accent: "var(--accent)", // #10a37f 唯一强调色
        surface: "var(--surface)", // #f5f5f5 卡片/表面
        line: "var(--line)", // #e5e5e5 边框/分隔线
      },
      fontFamily: {
        heading: ['"Signifier"', '"Source Serif Pro"', "Georgia", "ui-serif", "serif"],
        body: ['"Söhne"', '"Inter"', "system-ui", "-apple-system", '"Segoe UI"', "sans-serif"],
      },
      borderRadius: {
        card: "16px",
        btn: "16px",
      },
    },
  },
  plugins: [require("daisyui")],
  daisyui: {
    // 自定义 light 主题：把 daisyUI 语义变量映射到上面的设计 token，
    // 使既有 btn/card/badge/alert 类自动落在极简配色上。
    themes: [
      {
        light: {
          primary: "#10a37f",
          "primary-content": "#ffffff",
          secondary: "#f5f5f5",
          "secondary-content": "#0d0d0d",
          accent: "#10a37f",
          "accent-content": "#ffffff",
          neutral: "#0d0d0d",
          "neutral-content": "#ffffff",
          "base-100": "#ffffff",
          "base-200": "#f5f5f5",
          "base-300": "#e5e5e5",
          "base-content": "#0d0d0d",
          info: "#6e6e6e",
          "info-content": "#ffffff",
          success: "#10a37f",
          "success-content": "#ffffff",
          warning: "#b45309",
          "warning-content": "#ffffff",
          error: "#b42318",
          "error-content": "#ffffff",
        },
      },
    ],
    logs: false,
  },
};
