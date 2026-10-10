// Svelte 5 + Vite 的最小运行时入口。
// 业务逻辑禁止写在前端：所有能力通过 invoke 调用 src-tauri 命令。
import App from "./App.svelte";
import "./app.css";

const app = new App({
  target: document.getElementById("app")!,
});

export default app;
