import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, applyTheme, fmtUsd, onUiPrefsChanged, onUsageUpdated, FloatData } from "./api";

const win = getCurrentWindow();

async function applyPrefs(): Promise<void> {
  const s = await api.settings();
  applyTheme(s.theme);
  document.body.style.opacity = String(s.float_opacity); // 整窗透明度（窗口本体 transparent）
}

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

// 活跃任务切换：selectedId 记住用户选择；项目掉出活跃列表后回落到消耗最高者。
let selectedId: number | null = null;
let viewList: { project_id: number; project_name: string; recent_cost: number; total_cost: number; last_model: string }[] = [];

function renderSelection(d: FloatData): void {
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const cur = viewList[idx];
  const multi = viewList.length > 1;
  el("f-project").textContent = cur ? cur.project_name : d.project_name;
  el("f-project").title = cur && multi ? `活跃任务 ${idx + 1}/${viewList.length}（近30分 ${fmtUsd(cur.recent_cost)}）` : "";
  el("f-count").textContent = multi ? `${idx + 1}/${viewList.length}` : "";
  el("f-model").textContent = shortModel(cur ? cur.last_model : d.model);
  el("f-proj").textContent = fmtUsd(cur ? cur.total_cost : d.project_cost);
  el("f-proj-label").textContent = multi ? "该任务累计" : "当前项目";
  (el("f-prev") as HTMLButtonElement).style.display = multi ? "" : "none";
  (el("f-next") as HTMLButtonElement).style.display = multi ? "" : "none";
}

function cycle(step: number): void {
  if (viewList.length < 2) return;
  const idx = Math.max(0, viewList.findIndex((a) => a.project_id === selectedId));
  const next = (idx + step + viewList.length) % viewList.length;
  selectedId = viewList[next].project_id;
  void refresh();
}

async function refresh(): Promise<void> {
  const d = await api.floatData();
  viewList = d.active;
  if (selectedId != null && !viewList.some((a) => a.project_id === selectedId)) {
    selectedId = null; // 选中的任务已不活跃，回落
  }
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
  el("f-burn").textContent = fmtUsd(d.burn_rate);
  renderSelection(d);
  renderSpark(d.hourly);
}

// Tauri 的 data-tauri-drag-region 只匹配 mousedown 的精确目标元素，
// 悬浮窗表面全被子元素覆盖，属性永远不命中 —— 因此改为程序化拖拽。
document.body.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button")) return; // 按钮不触发拖拽/双击
  if (e.detail >= 2) {
    void api.openDashboard(); // 双击的第二次 mousedown：开面板
  } else {
    void win.startDragging();
  }
});
el("f-hide").addEventListener("click", () => void win.hide());
el("f-prev").addEventListener("click", () => cycle(-1));
el("f-next").addEventListener("click", () => cycle(1));
onUsageUpdated(() => void refresh());
onUiPrefsChanged(() => void applyPrefs());
void applyPrefs();
void refresh();
setInterval(() => void refresh(), 60_000); // 兜底：burn rate 随时间衰减也要更新
