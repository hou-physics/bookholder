import { api } from "../api";
import { save } from "@tauri-apps/plugin-dialog";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import type { Page } from "../main";

async function exportAs(kind: "md" | "csv" | "json", status: HTMLElement): Promise<void> {
  const ext = kind;
  const dest = await save({
    defaultPath: `bookholder-report.${ext}`,
    filters: [{ name: kind.toUpperCase(), extensions: [ext] }],
  });
  if (!dest) return;
  await api.exportReport(kind, dest);
  status.textContent = `已导出 ${dest}`;
}

export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const s = await api.settings();
      const auto = await isEnabled().catch(() => false);
      root.innerHTML = `<h2 style="margin-bottom:10px">设置</h2>
        <div class="panel" style="margin-bottom:10px">
          <h3>价格数据</h3>
          <p>已知 ${s.price_count} 个模型 ｜ 最后更新：${s.prices_last_fetch ?? "从未（使用内置快照）"}
             ｜ 状态：${s.prices_last_status ?? "—"}</p>
          <button id="btn-prices">立即刷新价格</button>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>计费口径（当前：${s.billing_mode === "subscription" ? "订阅（显示等值 API 成本）" : s.billing_mode === "api" ? "API（实际计费成本）" : "未知"}）</h3>
          <select id="sel-billing">
            <option value="" ${!s.billing_override ? "selected" : ""}>自动检测</option>
            <option value="subscription" ${s.billing_override === "subscription" ? "selected" : ""}>强制：订阅</option>
            <option value="api" ${s.billing_override === "api" ? "selected" : ""}>强制：API</option>
          </select>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>数据</h3>
          <p>数据库：${s.db_path} ｜ 跳过行：${s.skip_lines ?? 0} ｜ 坏行：${s.bad_lines ?? 0}</p>
          <button id="btn-backfill">全量重扫</button>
          <button id="btn-md">导出 Markdown</button>
          <button id="btn-csv">导出 CSV</button>
          <button id="btn-json">导出 JSON</button>
        </div>
        <div class="panel">
          <h3>启动</h3>
          <label><input type="checkbox" id="chk-auto" ${auto ? "checked" : ""}/> 开机自动启动</label>
        </div>
        <p id="status" class="dim" style="margin-top:10px"></p>`;

      const status = root.querySelector("#status") as HTMLElement;
      const busy = async (btn: HTMLButtonElement, fn: () => Promise<string | void>): Promise<void> => {
        btn.disabled = true;
        try { status.textContent = (await fn()) ?? "完成"; }
        catch (e) { status.textContent = `失败：${e}`; }
        finally { btn.disabled = false; }
      };
      const q = (id: string): HTMLButtonElement => root.querySelector(id) as HTMLButtonElement;
      q("#btn-prices").addEventListener("click", () => void busy(q("#btn-prices"), () => api.refreshPrices()));
      q("#btn-backfill").addEventListener("click", () => void busy(q("#btn-backfill"), async () => {
        const st = await api.backfill();
        return `重扫完成：新增 ${st.added}，跳过 ${st.skipped}，坏行 ${st.bad}`;
      }));
      q("#btn-md").addEventListener("click", () => void busy(q("#btn-md"), () => exportAs("md", status)));
      q("#btn-csv").addEventListener("click", () => void busy(q("#btn-csv"), () => exportAs("csv", status)));
      q("#btn-json").addEventListener("click", () => void busy(q("#btn-json"), () => exportAs("json", status)));
      (root.querySelector("#sel-billing") as HTMLSelectElement).addEventListener("change", (e) => {
        void api.setBillingOverride((e.target as HTMLSelectElement).value).then(() => page.render(root));
      });
      (root.querySelector("#chk-auto") as HTMLInputElement).addEventListener("change", (e) => {
        const on = (e.target as HTMLInputElement).checked;
        void (on ? enable() : disable()).then(() => (status.textContent = on ? "已开启自启" : "已关闭自启"));
      });
    })();
  },
};
