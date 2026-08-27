import { api, esc, fmtUsd } from "../api";
import { eventTable } from "./projects";
import type { Page } from "../main";

// 会话明细：跨项目、按时间倒序的会话流水，每条可展开逐请求事件表。
export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const sessions = await api.sessionsRecent(100);
      root.innerHTML = `<h2 style="margin-bottom:10px">会话明细 <span class="dim">最近 ${sessions.length} 个会话（全部项目，点击展开逐请求）</span></h2>
        <div id="sess-list"></div>`;
      const list = root.querySelector("#sess-list")!;
      for (const s of sessions) {
        const div = document.createElement("div");
        div.className = "panel";
        div.style.marginBottom = "8px";
        div.innerHTML = `<div class="clickable sess-head" style="display:flex;gap:12px;align-items:center;cursor:pointer">
          <b>${esc(s.project_name)}</b>
          <span class="dim">${esc(s.session_id.slice(0, 8))}</span>
          <span class="dim">${esc(s.started_at)} → ${esc(s.ended_at)}</span>
          <span>${fmtUsd(s.cost_usd)}</span>
          <span class="dim">${s.events} 次请求</span>
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
    })();
  },
};
