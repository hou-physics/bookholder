import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Totals {
  cost_usd: number; input: number; output: number; thinking: number;
  cache_read: number; cache_write: number; events: number; unpriced: number;
}
export interface DailyModelRow { date: string; model: string; cost_usd: number }
export interface HourRow { hour: string; main_cost: number; side_cost: number }
export interface ModelSplitRow { model: string; cost_usd: number; input: number; output: number; events: number }
export interface ProjectRow {
  id: number; display_name: string; cwd: string; cost_usd: number;
  tokens: number; sessions: number; active_days: number; last_seen: string;
}
export interface SessionRow {
  id: number; session_id: string; started_at: string; ended_at: string;
  billing_mode: string; cost_usd: number; events: number; side_cost: number;
}
export interface EventRow {
  ts: string; model: string; is_sidechain: boolean; input: number; output: number;
  thinking: number; cache_write_5m: number; cache_write_1h: number; cache_read: number;
  cost_usd: number | null;
}
export interface ActiveProjectRow {
  project_id: number; project_name: string; recent_cost: number;
  total_cost: number; last_model: string;
}
export interface FloatData {
  today_cost: number; project_cost: number; project_name: string; model: string;
  burn_rate: number; billing_mode: string; hourly: HourRow[]; active: ActiveProjectRow[];
}
export interface Overview {
  today: Totals; week: Totals; month: Totals; all: Totals;
  daily: DailyModelRow[]; models: ModelSplitRow[]; main_cost: number; side_cost: number;
}
export interface RecentSessionRow {
  id: number; session_id: string; project_name: string; started_at: string;
  ended_at: string; billing_mode: string; cost_usd: number; events: number; side_cost: number;
}
export interface FeePeriod { from: string; usd: number }
export interface SubComparison {
  fees: FeePeriod[]; window_start: string | null; window_days: number;
  actual_usd: number; equiv_usd: number; api_usd: number; savings_usd: number;
  leverage: number | null; month_equiv_usd: number; month_fee_usd: number | null;
  detected_tier: string | null;
}
export interface SettingsStatus {
  prices_last_fetch: string | null; prices_last_status: string | null; price_count: number;
  billing_mode: string; billing_override: string | null;
  skip_lines: string | null; bad_lines: string | null; db_path: string;
}

export const api = {
  floatData: () => invoke<FloatData>("float_data"),
  overview: () => invoke<Overview>("overview"),
  projects: () => invoke<ProjectRow[]>("projects_list"),
  sessions: (projectId: number) => invoke<SessionRow[]>("project_sessions", { projectId }),
  projectOverview: (projectId: number) => invoke<{ daily: DailyModelRow[]; models: ModelSplitRow[] }>("project_overview", { projectId }),
  events: (sessionPk: number) => invoke<EventRow[]>("session_events", { sessionPk }),
  settings: () => invoke<SettingsStatus>("settings_status"),
  refreshPrices: () => invoke<string>("refresh_prices"),
  backfill: () => invoke<{ added: number; skipped: number; bad: number }>("run_backfill"),
  exportReport: (kind: string, dest: string) => invoke<void>("export_report", { kind, dest }),
  setBillingOverride: (mode: string) => invoke<void>("set_billing_override", { mode }),
  sessionsRecent: (limit: number) => invoke<RecentSessionRow[]>("sessions_recent", { limit }),
  subscriptionComparison: () => invoke<SubComparison>("subscription_comparison"),
  setSubscriptionFees: (feesJson: string) => invoke<void>("set_subscription_fees", { feesJson }),
  openDashboard: () => invoke<void>("open_dashboard"),
};

export function onUsageUpdated(cb: () => void): void {
  void listen("usage-updated", cb);
}

export function fmtUsd(n: number): string {
  return n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`;
}

export function esc(s: string): string {
  return s.replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]!));
}

export function fmtTok(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return String(n);
}
