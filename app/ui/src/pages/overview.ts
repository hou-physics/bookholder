import { api, fmtUsd, fmtTok, Totals } from "../api";
import { t as i18nt } from "../i18n";
import { mountChart, palette } from "../charts";
import type { Page } from "../main";

function card(label: string, t: Totals): string {
  const tokens = t.input + t.output + t.cache_read + t.cache_write;
  return `<div class="card"><label>${label}</label><b>${fmtTok(tokens)} tok</b>
    <span class="usd">${fmtUsd(t.cost_usd)}</span>
    <span class="dim">in ${fmtTok(t.input)} · out ${fmtTok(t.output)} · ${i18nt("o.cacheR")} ${fmtTok(t.cache_read)} · ${i18nt("o.cacheW")} ${fmtTok(t.cache_write)}</span>
    ${t.unpriced > 0 ? `<span class="warn">${t.unpriced} ${i18nt("unpriced")}</span>` : ""}</div>`;
}

export const page: Page = {
  render(root: HTMLElement): void {
    root.innerHTML = `<div id="o-limits"></div><div id="billing-note"></div><div id="sub-compare"></div><div id="cards" class="cards"></div>
      <div class="chart-grid">
        <div class="panel"><h3>${i18nt("o.chartDaily")}</h3><div id="c-daily" class="chart"></div></div>
        <div class="panel"><h3>${i18nt("o.chartModels")}</h3><div id="c-models" class="chart"></div></div>
        <div class="panel"><h3>${i18nt("o.chartSide")}</h3><div id="c-side" class="chart chart-slim"></div></div>
      </div>`;
    void (async () => {
      const [o, s, sc] = await Promise.all([api.overview(), api.settings(), api.subscriptionComparison()]);
      void api.usageLimits().then((u) => {
        const label = (w: { kind: string; scope: string | null }): string =>
          w.kind === "session" ? i18nt("f.win5h") : w.kind === "weekly_all" ? i18nt("f.winWeekAll") : `${i18nt("f.winWeekPrefix")}${w.scope ?? w.kind}`;
        const html = u.windows.map((w) => {
          const pct = Math.max(0, Math.min(100, w.utilization));
          const cls = pct >= 95 ? " crit" : pct >= 80 ? " hot" : "";
          const reset = w.resets_at ? new Date(w.resets_at) : null;
          const resetTxt = reset ? ` · ${i18nt("f.reset")} ${reset.toLocaleString(undefined, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}` : "";
          const etaTxt = w.eta_days != null ? ` · ${i18nt("f.est")} ${w.eta_days.toFixed(1)}${i18nt("f.workDays")}` : w.eta_h != null ? ` · ${i18nt("f.est")} ${w.eta_h < 48 ? Math.round(w.eta_h) + "h" : (w.eta_h / 24).toFixed(1) + "d"}` : "";
          return `<div class="limit-row" style="margin-bottom:6px"><span class="limit-label" style="width:70px">${label(w)}</span>
            <div class="limit-track"><div class="limit-fill${cls}" style="width:${pct}%"></div></div>
            <span class="limit-txt dim" style="min-width:190px">${Math.round(pct)}%${resetTxt}${etaTxt}</span></div>`;
        }).join("");
        if (html) {
          document.getElementById("o-limits")!.innerHTML =
            `<div class="panel" style="margin-bottom:12px"><h3>${i18nt("o.limits")}</h3>${html}</div>`;
        }
      }).catch(() => {});
      if (sc.equiv_usd > 0) {
        const subEl = document.getElementById("sub-compare")!;
        if (sc.fees.length === 0) {
          subEl.innerHTML = `<p class="billing-note">${i18nt("o.fillFeeHint")}</p>`;
        } else {
          const lev = sc.leverage ? `${sc.leverage.toFixed(1)}×` : "—";
          const monthFee = sc.month_fee_usd != null ? fmtUsd(sc.month_fee_usd) : "—";
          subEl.innerHTML = `<div class="panel sub-compare">
            <h3>${i18nt("o.subTitle")} (${i18nt("o.from")} ${sc.window_start ?? ""}, ${Math.round(sc.window_days)} ${i18nt("o.days")})</h3>
            <div class="sub-grid">
              <div><label>${i18nt("o.actual")}</label><b>${fmtUsd(sc.actual_usd)}</b></div>
              <div><label>${i18nt("o.equiv")}</label><b>${fmtUsd(sc.equiv_usd)}</b></div>
              <div><label>${i18nt("o.saved")}</label><b class="good">${fmtUsd(sc.savings_usd)}</b></div>
              <div><label>${i18nt("o.leverage")}</label><b class="good">${lev}</b></div>
            </div>
            <p class="dim" style="margin-top:6px">${i18nt("o.thisMonth")}: ${i18nt("o.equivShort")} ${fmtUsd(sc.month_equiv_usd)} ｜ ${i18nt("o.monthFee")} ${monthFee}${sc.api_usd > 0 ? ` ｜ ${i18nt("o.apiExtra")} ${fmtUsd(sc.api_usd)} (${i18nt("o.notAllocated")})` : ""}</p>
          </div>`;
        }
      }
      if (s.billing_mode === "subscription") {
        document.getElementById("billing-note")!.innerHTML = `<p class="billing-note">${i18nt("o.noteSub")}</p>`;
      } else if (s.billing_mode === "api") {
        document.getElementById("billing-note")!.innerHTML = `<p class="billing-note">${i18nt("o.noteApi")}</p>`;
      }
      document.getElementById("cards")!.innerHTML =
        card(i18nt("o.today"), o.today) + card(i18nt("o.week"), o.week) + card(i18nt("o.month"), o.month) + card(i18nt("o.all"), o.all);

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
          label: { color: document.body.classList.contains("theme-dark") ? "#e8eaf0" : "#33291f" },
          data: o.models.map((m) => ({ name: m.model, value: +m.cost_usd.toFixed(4) })) }],
      });

      mountChart(document.getElementById("c-side")!, {
        tooltip: {},
        xAxis: { type: "value", axisLabel: { formatter: (v: number) => `$${v}` } },
        yAxis: { type: "category", data: [i18nt("o.cost")] },
        series: [
          { name: i18nt("main.dialog"), type: "bar", stack: "s", data: [+o.main_cost.toFixed(4)], itemStyle: { color: palette()[0] } },
          { name: i18nt("subagent"), type: "bar", stack: "s", data: [+o.side_cost.toFixed(4)], itemStyle: { color: palette()[1] } },
        ],
        legend: { textStyle: { color: "#8b90a0" } },
      });
    })();
  },
};
