// Svelte 5 + Vite 的最小运行时入口。
// 业务逻辑禁止写在前端：所有能力通过 invoke 调用 src-tauri 命令。
import App from "./App.svelte";
import { mount } from "svelte";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  // 审查项 9/12：不 panic、不白屏静默，给出中文兜底提示。
  document.body.innerHTML =
    '<p style="padding:2rem;font-family:sans-serif">页面加载失败：未找到应用容器，请重启程序。</p>';
} else {
  // Svelte 5 runes 模式组件没有 new App() 构造签名，必须用 mount API。
  mount(App, { target });
}
