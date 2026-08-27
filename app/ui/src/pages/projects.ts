import { api, esc, fmtUsd, fmtTok, ProjectRow, SessionRow, EventRow } from "../api";
import { mountChart } from "../charts";
import type { Page } from "../main";

export function eventTable(evs: EventRow[]): string {
  const rows = evs.map((e) => `<tr>
    <td>${esc(e.ts)}</td><td>${esc(e.model)}</td><td>${e.is_sidechain ? "sub" : "主"}</td>
    <td>${fmtTok(e.input)}</td><td>${fmtTok(e.output)}</td><td>${fmtTok(e.thinking)}</td>
    <td>${fmtTok(e.cache_write_5m + e.cache_write_1h)}</td><td>${fmtTok(e.cache_read)}</td>
    <td>${e.cost_usd == null ? "未计价" : fmtUsd(e.cost_usd)}</td></tr>`).join("");
  return `<table><tr><th>时间</th><th>模型</th><th>类型</th><th>in</th><th>out</th>
    <th>think</th><th>cache写</th><th>cache读</th><th>成本</th></tr>${rows}</table>`;
}

function mountProjectCharts(daily: Awaited<ReturnType<typeof api.projectOverview>>["daily"], models: Awaited<ReturnType<typeof api.projectOverview>>["models"]): void {
  const modelNames = [...new Set(daily.map((d) => d.model))];
  const dates = [...new Set(daily.map((d) => d.date))].sort();
  mountChart(document.getElementById("p-daily")!, {
    tooltip: { trigger: "axis" },
    legend: { textStyle: { color: "#8b90a0" } },
    xAxis: { type: "category", data: dates },
    yAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
    series: modelNames.map((m) => ({
      name: m, type: "line", stack: "cost", areaStyle: {}, showSymbol: false,
      data: dates.map((dt) => daily.find((r) => r.date === dt && r.model === m)?.cost_usd ?? 0),
    })),
  });

  mountChart(document.getElementById("p-models")!, {
    tooltip: { formatter: (it: { name: string; value: number }) => `${it.name}: ${fmtUsd(it.value)}` } as any,
    series: [{ type: "pie", radius: ["45%", "72%"],
      label: { color: document.body.classList.contains("theme-dark") ? "#e8eaf0" : "#33291f" },
      data: models.map((m) => ({ name: m.model, value: +m.cost_usd.toFixed(4) })) }],
  });
}

async function showSessions(root: HTMLElement, p: ProjectRow): Promise<void> {
  const sessions = await api.sessions(p.id);
  root.innerHTML = `<button id="back">← 项目列表</button>
    <h2 style="margin:10px 0">${esc(p.display_name)} <span class="dim">${fmtUsd(p.cost_usd)}</span></h2>
    <div class="chart-grid" style="margin-bottom:10px"><div class="panel"><h3>近 30 天成本（按模型）</h3><div id="p-daily" class="chart"></div></div><div class="panel"><h3>模型占比</h3><div id="p-models" class="chart"></div></div></div>
    <div id="s-list"></div>`;
  root.querySelector("#back")!.addEventListener("click", () => void page.render(root));

  void api.projectOverview(p.id).then((o) => mountProjectCharts(o.daily, o.models));

  const list = root.querySelector("#s-list")!;
  for (const s of sessions) {
    const div = document.createElement("div");
    div.className = "panel";
    div.style.marginBottom = "8px";
    div.innerHTML = `<div class="clickable sess-head" style="display:flex;gap:12px;cursor:pointer">
      <b>${esc(s.session_id.slice(0, 8))}</b><span class="dim">${esc(s.started_at)} → ${esc(s.ended_at)}</span>
      <span>${fmtUsd(s.cost_usd)}</span><span class="dim">${s.events} 次请求</span>
      <span class="dim">subagent ${fmtUsd(s.side_cost)}</span>
      <span class="badge badge-${esc(s.billing_mode)}">${s.billing_mode === "subscription" ? "订阅" : esc(s.billing_mode)}</span>
    </div><div class="sess-body" style="display:none;margin-top:8px"></div>`;
    div.querySelector(".sess-head")!.addEventListener("click", () => {
      const body = div.querySelector(".sess-body") as HTMLElement;
      if (body.style.display === "none") {
        body.style.display = "block";
        void api.events(s.id).then((evs) => (body.innerHTML = eventTable(evs)));
      } else {
        body.style.display = "none";
      }
    });
    list.appendChild(div);
  }
}

export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const [projects, sc] = await Promise.all([api.projects(), api.subscriptionComparison()]);
      // 分摊实付：订阅实付按各项目等值成本占比反推（仅在填写了月费时显示）
      const canAllocate = sc.actual_usd > 0 && sc.equiv_usd > 0;
      const alloc = (equiv: number): string =>
        canAllocate ? fmtUsd((sc.actual_usd * equiv) / sc.equiv_usd) : "—";
      root.innerHTML = `<h2 style="margin-bottom:10px">项目</h2>
        <table><tr><th>项目</th><th>等值成本</th>${canAllocate ? "<th>分摊实付</th>" : ""}<th>tokens</th><th>会话</th><th>活跃天数</th><th>最近活动</th></tr>
        ${projects.map((p, i) => `<tr class="clickable" data-i="${i}">
          <td><b>${esc(p.display_name)}</b> <span class="dim">${esc(p.cwd)}</span></td>
          <td>${fmtUsd(p.cost_usd)}</td>${canAllocate ? `<td>${alloc(p.cost_usd)}</td>` : ""}<td>${fmtTok(p.tokens)}</td>
          <td>${p.sessions}</td><td>${p.active_days}</td><td>${esc(p.last_seen)}</td></tr>`).join("")}
        </table>`;
      root.querySelectorAll("tr.clickable").forEach((tr) =>
        tr.addEventListener("click", () =>
          void showSessions(root, projects[Number((tr as HTMLElement).dataset.i)])));
    })();
  },
};
