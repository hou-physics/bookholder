import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, fmtUsd, onUsageUpdated, FloatData } from "./api";

const win = getCurrentWindow();

function el(id: string): HTMLElement {
  return document.getElementById(id)!;
}

function shortModel(m: string): string {
  return m.replace(/^claude-/, "").replace(/-\d{8}$/, "");
}

function renderSpark(hourly: FloatData["hourly"]): void {
  const spark = el("f-spark");
  spark.innerHTML = "";
  const max = Math.max(...hourly.map((h) => h.main_cost + h.side_cost), 1e-9);
  for (const h of hourly) {
    const col = document.createElement("div");
    col.className = "spark-col";
    const side = document.createElement("div");
    side.className = "spark-side";
    side.style.height = `${((h.side_cost / max) * 100).toFixed(1)}%`;
    const main = document.createElement("div");
    main.className = "spark-main";
    main.style.height = `${((h.main_cost / max) * 100).toFixed(1)}%`;
    col.append(side, main);
    col.title = `${h.hour}  主 ${fmtUsd(h.main_cost)} / 子 ${fmtUsd(h.side_cost)}`;
    spark.appendChild(col);
  }
}

async function refresh(): Promise<void> {
  const d = await api.floatData();
  el("f-project").textContent = d.project_name;
  el("f-model").textContent = shortModel(d.model);
  const badge = el("f-badge");
  badge.textContent = d.billing_mode === "subscription" ? "订阅" : d.billing_mode === "api" ? "API" : "?";
  badge.className = `badge badge-${d.billing_mode}`;
  badge.title =
    d.billing_mode === "subscription"
      ? "订阅模式：所有金额为等值 API 成本（这些 token 若按 API 计费的价格），不是你的实际账单"
      : d.billing_mode === "api"
        ? "API 模式：金额为实际计费成本"
        : "";
  el("f-today").textContent = fmtUsd(d.today_cost);
  el("f-proj").textContent = fmtUsd(d.project_cost);
  el("f-burn").textContent = fmtUsd(d.burn_rate);
  // 并发任务：近 30 分钟内有消耗的项目 ≥2 个时列出（否则隐藏该行）
  const activeEl = el("f-active");
  if (d.active.length >= 2) {
    activeEl.style.display = "block";
    activeEl.textContent =
      `▶ ${d.active.length} 个任务: ` +
      d.active.map((a) => `${a.project_name} ${fmtUsd(a.recent_cost)}`).join(" · ");
    activeEl.title = `最近 30 分钟内活跃的项目及其窗口内消耗\n${d.active
      .map((a) => `${a.project_name}: ${fmtUsd(a.recent_cost)}`)
      .join("\n")}`;
  } else {
    activeEl.style.display = "none";
  }
  renderSpark(d.hourly);
}

// Tauri 的 data-tauri-drag-region 只匹配 mousedown 的精确目标元素，
// 悬浮窗表面全被子元素覆盖，属性永远不命中 —— 因此改为程序化拖拽。
document.body.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("#f-hide")) return;
  if (e.detail >= 2) {
    void api.openDashboard(); // 双击的第二次 mousedown：开面板
  } else {
    void win.startDragging();
  }
});
el("f-hide").addEventListener("click", () => void win.hide());
onUsageUpdated(() => void refresh());
void refresh();
setInterval(() => void refresh(), 60_000); // 兜底：burn rate 随时间衰减也要更新
