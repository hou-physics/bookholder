import { api, fmtUsd, fmtTok, ProjectRow, SessionRow, EventRow } from "../api";
import type { Page } from "../main";

function eventTable(evs: EventRow[]): string {
  const rows = evs.map((e) => `<tr>
    <td>${e.ts}</td><td>${e.model}</td><td>${e.is_sidechain ? "sub" : "主"}</td>
    <td>${fmtTok(e.input)}</td><td>${fmtTok(e.output)}</td><td>${fmtTok(e.thinking)}</td>
    <td>${fmtTok(e.cache_write_5m + e.cache_write_1h)}</td><td>${fmtTok(e.cache_read)}</td>
    <td>${e.cost_usd == null ? "未计价" : fmtUsd(e.cost_usd)}</td></tr>`).join("");
  return `<table><tr><th>时间</th><th>模型</th><th>类型</th><th>in</th><th>out</th>
    <th>think</th><th>cache写</th><th>cache读</th><th>成本</th></tr>${rows}</table>`;
}

async function showSessions(root: HTMLElement, p: ProjectRow): Promise<void> {
  const sessions = await api.sessions(p.id);
  root.innerHTML = `<button id="back">← 项目列表</button>
    <h2 style="margin:10px 0">${p.display_name} <span class="dim">${fmtUsd(p.cost_usd)}</span></h2>
    <div id="s-list"></div>`;
  root.querySelector("#back")!.addEventListener("click", () => void page.render(root));
  const list = root.querySelector("#s-list")!;
  for (const s of sessions) {
    const div = document.createElement("div");
    div.className = "panel";
    div.style.marginBottom = "8px";
    div.innerHTML = `<div class="clickable sess-head" style="display:flex;gap:12px;cursor:pointer">
      <b>${s.session_id.slice(0, 8)}</b><span class="dim">${s.started_at} → ${s.ended_at}</span>
      <span>${fmtUsd(s.cost_usd)}</span><span class="dim">${s.events} 次请求</span>
      <span class="dim">subagent ${fmtUsd(s.side_cost)}</span>
      <span class="badge badge-${s.billing_mode}">${s.billing_mode === "subscription" ? "订阅" : s.billing_mode}</span>
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
      const projects = await api.projects();
      root.innerHTML = `<h2 style="margin-bottom:10px">项目</h2>
        <table><tr><th>项目</th><th>成本</th><th>tokens</th><th>会话</th><th>活跃天数</th><th>最近活动</th></tr>
        ${projects.map((p, i) => `<tr class="clickable" data-i="${i}">
          <td><b>${p.display_name}</b> <span class="dim">${p.cwd}</span></td>
          <td>${fmtUsd(p.cost_usd)}</td><td>${fmtTok(p.tokens)}</td>
          <td>${p.sessions}</td><td>${p.active_days}</td><td>${p.last_seen}</td></tr>`).join("")}
        </table>`;
      root.querySelectorAll("tr.clickable").forEach((tr) =>
        tr.addEventListener("click", () =>
          void showSessions(root, projects[Number((tr as HTMLElement).dataset.i)])));
    })();
  },
};
