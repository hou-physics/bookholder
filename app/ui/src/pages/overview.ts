import { api, fmtUsd, fmtTok, Totals } from "../api";
import { mountChart, PALETTE } from "../charts";
import type { Page } from "../main";

function card(label: string, t: Totals): string {
  return `<div class="card"><label>${label}</label><b>${fmtUsd(t.cost_usd)}</b>
    <span class="dim">in ${fmtTok(t.input)} · out ${fmtTok(t.output)} · cache读 ${fmtTok(t.cache_read)} · 写 ${fmtTok(t.cache_write)}</span>
    ${t.unpriced > 0 ? `<span class="warn">${t.unpriced} 条未计价</span>` : ""}</div>`;
}

export const page: Page = {
  render(root: HTMLElement): void {
    root.innerHTML = `<div id="billing-note"></div><div id="sub-compare"></div><div id="cards" class="cards"></div>
      <div class="chart-grid">
        <div class="panel"><h3>近 30 天成本（按模型）</h3><div id="c-daily" class="chart"></div></div>
        <div class="panel"><h3>模型占比</h3><div id="c-models" class="chart"></div></div>
        <div class="panel"><h3>主对话 vs Subagent</h3><div id="c-side" class="chart chart-slim"></div></div>
      </div>`;
    void (async () => {
      const [o, s, sc] = await Promise.all([api.overview(), api.settings(), api.subscriptionComparison()]);
      if (sc.equiv_usd > 0) {
        const subEl = document.getElementById("sub-compare")!;
        if (sc.fees.length === 0) {
          subEl.innerHTML = `<p class="billing-note">想看订阅实付 vs 等值成本的对比？到<a href="#settings">设置页</a>填一下订阅月费即可。</p>`;
        } else {
          const lev = sc.leverage ? `${sc.leverage.toFixed(1)}×` : "—";
          const monthFee = sc.month_fee_usd != null ? fmtUsd(sc.month_fee_usd) : "—";
          subEl.innerHTML = `<div class="panel sub-compare">
            <h3>订阅对比（${sc.window_start ?? ""} 起，${Math.round(sc.window_days)} 天）</h3>
            <div class="sub-grid">
              <div><label>订阅实付（折算）</label><b>${fmtUsd(sc.actual_usd)}</b></div>
              <div><label>等值 API 成本</label><b>${fmtUsd(sc.equiv_usd)}</b></div>
              <div><label>省下</label><b class="good">${fmtUsd(sc.savings_usd)}</b></div>
              <div><label>杠杆</label><b class="good">${lev}</b></div>
            </div>
            <p class="dim" style="margin-top:6px">本月：等值 ${fmtUsd(sc.month_equiv_usd)} ｜ 月费 ${monthFee}${sc.api_usd > 0 ? ` ｜ 另有 API 实付 ${fmtUsd(sc.api_usd)}（未参与分摊）` : ""}</p>
          </div>`;
        }
      }
      if (s.billing_mode === "subscription") {
        document.getElementById("billing-note")!.innerHTML =
          `<p class="billing-note">💡 订阅模式：以下所有金额是<b>等值 API 成本</b>——这些 token 若按 API 价格计费需要花多少钱。你的实际支出是订阅费本身；这个数字越高，说明订阅越划算。</p>`;
      } else if (s.billing_mode === "api") {
        document.getElementById("billing-note")!.innerHTML =
          `<p class="billing-note">API 模式：以下金额为实际计费成本。</p>`;
      }
      document.getElementById("cards")!.innerHTML =
        card("今日", o.today) + card("近 7 天", o.week) + card("近 30 天", o.month) + card("全部", o.all);

      const models = [...new Set(o.daily.map((d) => d.model))];
      const dates = [...new Set(o.daily.map((d) => d.date))].sort();
      mountChart(document.getElementById("c-daily")!, {
        tooltip: { trigger: "axis" },
        legend: { textStyle: { color: "#8b90a0" } },
        xAxis: { type: "category", data: dates },
        yAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
        series: models.map((m) => ({
          name: m, type: "line", stack: "cost", areaStyle: {}, showSymbol: false,
          data: dates.map((dt) => o.daily.find((r) => r.date === dt && r.model === m)?.cost_usd ?? 0),
        })),
      });

      mountChart(document.getElementById("c-models")!, {
        tooltip: { formatter: (p: { name: string; value: number }) => `${p.name}: ${fmtUsd(p.value)}` } as any,
        series: [{ type: "pie", radius: ["45%", "72%"],
          label: { color: "#e8eaf0" },
          data: o.models.map((m) => ({ name: m.model, value: +m.cost_usd.toFixed(4) })) }],
      });

      mountChart(document.getElementById("c-side")!, {
        tooltip: {},
        xAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
        yAxis: { type: "category", data: ["成本"] },
        series: [
          { name: "主对话", type: "bar", stack: "s", data: [+o.main_cost.toFixed(4)], itemStyle: { color: PALETTE[0] } },
          { name: "Subagent", type: "bar", stack: "s", data: [+o.side_cost.toFixed(4)], itemStyle: { color: PALETTE[1] } },
        ],
        legend: { textStyle: { color: "#8b90a0" } },
      });
    })();
  },
};
