import { api, esc, fmtUsd } from "../api";
import { t, t2 } from "../i18n";
import { eventTable } from "./projects";
import type { Page } from "../main";

// 会话明细：跨项目、按时间倒序的会话流水，每条可展开逐请求事件表。
export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const sessions = await api.sessionsRecent(100);
      root.innerHTML = `<h2 style="margin-bottom:10px">${t("s.title")} <span class="dim">${t2("s.recent", { n: sessions.length })}</span></h2>
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
          <span class="dim">${s.events} ${t("p.requests")}</span>
          <span class="dim">${t("subagent")} ${fmtUsd(s.side_cost)}</span>
          <span class="badge badge-${esc(s.billing_mode)}">${s.billing_mode === "subscription" ? t("badge.sub") : esc(s.billing_mode)}</span>
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
