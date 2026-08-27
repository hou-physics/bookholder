import * as echarts from "echarts";

const DARK_PALETTE = ["#4f6df5", "#b46ff5", "#6fd49a", "#d4b16f", "#f56d6d", "#6fc4d4"];
const LIGHT_PALETTE = ["#e8722a", "#c9541f", "#2f9e63", "#b97f1d", "#d4453b", "#2a8fb8"];

function isDark(): boolean {
  return document.body.classList.contains("theme-dark");
}

/** 当前主题的图表调色板（[0]=主色 [1]=次色，overview 的主/子对比依赖此约定）。 */
export function palette(): string[] {
  return isDark() ? DARK_PALETTE : LIGHT_PALETTE;
}

export function mountChart(el: HTMLElement, option: echarts.EChartsOption): echarts.ECharts {
  const chart = echarts.init(el, undefined, { renderer: "canvas" });
  chart.setOption({
    color: palette(),
    textStyle: { color: isDark() ? "#e8eaf0" : "#33291f" },
    backgroundColor: "transparent",
    ...option,
  });
  new ResizeObserver(() => chart.resize()).observe(el);
  return chart;
}
