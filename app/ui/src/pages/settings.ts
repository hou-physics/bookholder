import { api, broadcastUiPrefsChanged } from "../api";
import { t, t2, currentLang } from "../i18n";
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
  status.textContent = t2("st.exported", { p: dest });
}

export const page: Page = {
  render(root: HTMLElement): void {
    void (async () => {
      const [s, sc] = await Promise.all([api.settings(), api.subscriptionComparison()]);
      const auto = await isEnabled().catch(() => false);
      const feeRows = sc.fees
        .map(
          (f, i) => `<tr><td>${f.from} ${t("st.feeFrom")}</td><td>$${f.usd}${t("st.perMonth")}</td>
            <td><button class="fee-del" data-i="${i}">${t("st.delete")}</button></td></tr>`,
        )
        .join("");
      const billingLabel =
        s.billing_mode === "subscription" ? t("st.bSub") : s.billing_mode === "api" ? t("st.bApi") : t("st.bUnknown");
      root.innerHTML = `<h2 style="margin-bottom:10px">${t("st.title")}</h2>
        <div class="panel" style="margin-bottom:10px">
          <h3>${t("st.prices")}</h3>
          <p>${t2("st.knownModels", { n: s.price_count })} ｜ ${t("st.lastUpdate")}: ${s.prices_last_fetch ?? t("st.never")}
             ｜ ${t("st.status")}: ${s.prices_last_status ?? "—"}</p>
          <button id="btn-prices">${t("st.refresh")}</button>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>${t2("st.billing", { m: billingLabel })}</h3>
          <select id="sel-billing">
            <option value="" ${!s.billing_override ? "selected" : ""}>${t("st.auto")}</option>
            <option value="subscription" ${s.billing_override === "subscription" ? "selected" : ""}>${t("st.forceSub")}</option>
            <option value="api" ${s.billing_override === "api" ? "selected" : ""}>${t("st.forceApi")}</option>
          </select>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>${t("st.data")}</h3>
          <p>${t("st.db")}: ${s.db_path} ｜ ${t("st.skipped")}: ${s.skip_lines ?? 0} ｜ ${t("st.bad")}: ${s.bad_lines ?? 0}</p>
          <button id="btn-backfill">${t("st.backfill")}</button>
          <button id="btn-md">${t("st.exportMd")}</button>
          <button id="btn-csv">${t("st.exportCsv")}</button>
          <button id="btn-json">${t("st.exportJson")}</button>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>${t2("st.fees", { t: sc.detected_tier ?? "—" })}</h3>
          <table style="max-width:420px">${feeRows || `<tr><td class="dim" colspan="3">${t("st.noFees")}</td></tr>`}</table>
          <p style="margin-top:8px">
            <input type="date" id="fee-from" />
            <input type="number" id="fee-usd" min="0" step="1" placeholder="${t("st.feeUsd")}" style="width:90px" />
            <button id="fee-add">${t("st.addFee")}</button>
          </p>
          <p class="dim">${t("st.feeHint")}</p>
        </div>
        <div class="panel" style="margin-bottom:10px">
          <h3>${t("st.appearance")}</h3>
          <label>${t("st.theme")}
            <select id="sel-theme">
              <option value="light" ${s.theme === "light" ? "selected" : ""}>${t("st.light")}</option>
              <option value="dark" ${s.theme === "dark" ? "selected" : ""}>${t("st.dark")}</option>
            </select>
          </label>
          <label style="margin-left:16px">${t("st.lang")}
            <select id="sel-lang">
              <option value="auto" ${!s.ui_lang ? "selected" : ""}>${t("st.langAuto")}</option>
              <option value="zh" ${s.ui_lang === "zh" ? "selected" : ""}>中文</option>
              <option value="en" ${s.ui_lang === "en" ? "selected" : ""}>English</option>
              <option value="de" ${s.ui_lang === "de" ? "selected" : ""}>Deutsch</option>
            </select>
          </label>
          <label style="margin-left:16px">${t("st.opacity")}
            <input type="range" id="rng-opacity" min="30" max="100" step="5" value="${Math.round(s.float_opacity * 100)}" />
            <span id="opacity-val">${Math.round(s.float_opacity * 100)}%</span>
          </label>
        </div>
        <div class="panel">
          <h3>${t("st.startup")}</h3>
          <label><input type="checkbox" id="chk-auto" ${auto ? "checked" : ""}/> ${t("st.autostart")}</label>
          <button id="btn-quit" style="margin-left:16px">${t("st.quit")}</button>
        </div>
        <p id="status" class="dim" style="margin-top:10px"></p>`;

      const status = root.querySelector("#status") as HTMLElement;
      const busy = async (btn: HTMLButtonElement, fn: () => Promise<string | void>): Promise<void> => {
        btn.disabled = true;
        try { status.textContent = (await fn()) ?? t("st.done"); }
        catch (e) { status.textContent = t2("st.failed", { e: e instanceof Error ? e.message : String(e) }); }
        finally { btn.disabled = false; }
      };
      const q = (id: string): HTMLButtonElement => root.querySelector(id) as HTMLButtonElement;
      q("#btn-prices").addEventListener("click", () => void busy(q("#btn-prices"), () => api.refreshPrices()));
      q("#btn-backfill").addEventListener("click", () => void busy(q("#btn-backfill"), async () => {
        const st = await api.backfill();
        return t2("st.backfillDone", { a: st.added, s: st.skipped, b: st.bad });
      }));
      q("#btn-md").addEventListener("click", () => void busy(q("#btn-md"), () => exportAs("md", status)));
      q("#btn-csv").addEventListener("click", () => void busy(q("#btn-csv"), () => exportAs("csv", status)));
      q("#btn-json").addEventListener("click", () => void busy(q("#btn-json"), () => exportAs("json", status)));
      (root.querySelector("#sel-billing") as HTMLSelectElement).addEventListener("change", (e) => {
        void api.setBillingOverride((e.target as HTMLSelectElement).value).then(() => page.render(root));
      });
      const saveFees = async (fees: { from: string; usd: number }[]): Promise<void> => {
        await api.setSubscriptionFees(JSON.stringify(fees));
        page.render(root);
      };
      q("#fee-add").addEventListener("click", () => void busy(q("#fee-add"), async () => {
        const from = (root.querySelector("#fee-from") as HTMLInputElement).value;
        const usd = Number((root.querySelector("#fee-usd") as HTMLInputElement).value);
        if (!from || !(usd >= 0)) return t("st.fillFirst");
        const fees = sc.fees.filter((f) => f.from !== from);
        fees.push({ from, usd });
        await saveFees(fees);
        return t("st.saved");
      }));
      root.querySelectorAll(".fee-del").forEach((btn) =>
        btn.addEventListener("click", () => {
          const i = Number((btn as HTMLElement).dataset.i);
          void saveFees(sc.fees.filter((_, idx) => idx !== i));
        }),
      );
      (root.querySelector("#sel-theme") as HTMLSelectElement).addEventListener("change", (e) => {
        void api
          .setUiPrefs((e.target as HTMLSelectElement).value, null)
          .then(() => broadcastUiPrefsChanged());
      });
      (root.querySelector("#sel-lang") as HTMLSelectElement).addEventListener("change", (e) => {
        void api
          .setUiPrefs(null, null, (e.target as HTMLSelectElement).value)
          .then(() => broadcastUiPrefsChanged());
      });
      const rng = root.querySelector("#rng-opacity") as HTMLInputElement;
      rng.addEventListener("input", () => {
        (root.querySelector("#opacity-val") as HTMLElement).textContent = `${rng.value}%`;
      });
      rng.addEventListener("change", () => {
        void api.setUiPrefs(null, Number(rng.value) / 100).then(() => broadcastUiPrefsChanged());
      });
      q("#btn-quit").addEventListener("click", () => void api.quitApp());
      (root.querySelector("#chk-auto") as HTMLInputElement).addEventListener("change", (e) => {
        const on = (e.target as HTMLInputElement).checked;
        void (on ? enable() : disable()).then(() => (status.textContent = on ? t("st.autoOn") : t("st.autoOff")));
      });
      void currentLang; // referenced to avoid unused import when tree-shaken
    })();
  },
};
