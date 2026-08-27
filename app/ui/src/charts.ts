import * as echarts from "echarts";

export const PALETTE = ["#4f6df5", "#b46ff5", "#6fd49a", "#d4b16f", "#f56d6d", "#6fc4d4"];

export function mountChart(el: HTMLElement, option: echarts.EChartsOption): echarts.ECharts {
  const chart = echarts.init(el, undefined, { renderer: "canvas" });
  chart.setOption({
    color: PALETTE,
    textStyle: { color: "#e8eaf0" },
    backgroundColor: "transparent",
    ...option,
  });
  new ResizeObserver(() => chart.resize()).observe(el);
  return chart;
}
