import { api, fmtUsd, onUsageUpdated, FloatData } from "./api";

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
  el("f-today").textContent = fmtUsd(d.today_cost);
  el("f-proj").textContent = fmtUsd(d.project_cost);
  el("f-burn").textContent = fmtUsd(d.burn_rate);
  renderSpark(d.hourly);
}

document.body.addEventListener("dblclick", () => void api.openDashboard());
onUsageUpdated(() => void refresh());
void refresh();
setInterval(() => void refresh(), 60_000); // 兜底：burn rate 随时间衰减也要更新
