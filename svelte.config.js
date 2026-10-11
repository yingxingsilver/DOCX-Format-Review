// Svelte 编译器配置：svelte-check / vite-plugin-svelte 共用。
// vite-plugin-svelte 会自动加载本文件；若缺失，插件初始化会被跳过，
// svelte-check 将报 "No Svelte configuration found in vite config"。
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
};
