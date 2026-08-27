import { onUsageUpdated } from "./api";
import { page as overview } from "./pages/overview";
import { page as projects } from "./pages/projects";
import { page as sessions } from "./pages/sessions";
import { page as settings } from "./pages/settings";

export interface Page { render(root: HTMLElement): void }

const routes: Record<string, Page> = {
  "#overview": overview,
  "#projects": projects,
  "#sessions": sessions,
  "#settings": settings,
};

function route(): void {
  const hash = location.hash || "#overview";
  document.querySelectorAll("#nav a").forEach((a) =>
    a.classList.toggle("active", a.getAttribute("href") === hash));
  const root = document.getElementById("page")!;
  root.innerHTML = "";
  (routes[hash] ?? routes["#overview"]).render(root);
}

window.addEventListener("hashchange", route);
onUsageUpdated(() => {
  // 数据更新 → 当前页整页重渲染（数据量小，简单可靠）；
  // 但设置页含表单交互状态，流式 ingest 事件不应打断用户操作
  if ((location.hash || "#overview") !== "#settings") route();
});
route();
