export type Lang = "zh" | "en" | "de";

let lang: Lang = "en";

/** 用户偏好（meta ui_lang，null=auto）→ 实际语言；auto 时按系统语言匹配。 */
export function resolveLang(pref: string | null): Lang {
  if (pref === "zh" || pref === "en" || pref === "de") return pref;
  const sys = (navigator.language || "en").toLowerCase();
  if (sys.startsWith("zh")) return "zh";
  if (sys.startsWith("de")) return "de";
  return "en";
}

export function setLang(l: Lang): void {
  lang = l;
}

export function currentLang(): Lang {
  return lang;
}

const D: Record<string, [string, string, string]> = {
  // —— 通用 / 导航 —— [zh, en, de]
  "nav.overview": ["总览", "Overview", "Übersicht"],
  "nav.projects": ["项目", "Projects", "Projekte"],
  "nav.sessions": ["会话明细", "Sessions", "Sitzungen"],
  "nav.settings": ["设置", "Settings", "Einstellungen"],
  "badge.sub": ["订阅", "Sub", "Abo"],
  "badge.api": ["API", "API", "API"],
  "unpriced": ["未计价", "unpriced", "ohne Preis"],
  "main.dialog": ["主对话", "Main", "Hauptdialog"],
  "subagent": ["Subagent", "Subagent", "Subagent"],

  // —— 悬浮窗 ——
  "f.today": ["今日", "Today", "Heute"],
  "f.taskTotal": ["该任务累计", "Task total", "Task gesamt"],
  "f.currentProject": ["当前项目", "Project", "Projekt"],
  "f.burn": ["燃烧率/时", "Burn rate/h", "Rate/Std"],
  "f.h24": ["24 小时前", "24 h ago", "vor 24 Std"],
  "f.perBar": ["每柱 = 1 小时", "1 bar = 1 hour", "1 Balken = 1 Std"],
  "f.now": ["现在", "now", "jetzt"],
  "f.currentTask": ["当前任务", "Current task", "Aktueller Task"],
  "f.prev": ["上一个活跃任务", "Previous active task", "Vorheriger aktiver Task"],
  "f.next": ["下一个活跃任务", "Next active task", "Nächster aktiver Task"],
  "f.expandTip": ["展开/收起当前任务 24 小时明细", "Expand/collapse current task's 24h detail", "24-Std-Detail des Tasks ein-/ausklappen"],
  "f.hideTip": ["隐藏悬浮窗（菜单栏图标可恢复，不是退出）", "Hide floating window (restore via menu bar icon; app keeps running)", "Fenster ausblenden (über Menüleiste wiederherstellbar; App läuft weiter)"],
  "f.activeTip": ["活跃任务", "Active task", "Aktiver Task"],
  "f.last30": ["近30分", "last 30 min", "letzte 30 Min"],
  "f.tipSub": ["订阅模式：所有金额为等值 API 成本（这些 token 若按 API 计费的价格），不是你的实际账单", "Subscription mode: amounts are API-equivalent cost (what these tokens would cost at API prices), not your actual bill", "Abo-Modus: Beträge sind API-Äquivalenzkosten (was diese Tokens zu API-Preisen kosten würden), nicht Ihre tatsächliche Rechnung"],
  "f.tipApi": ["API 模式：金额为实际计费成本", "API mode: amounts are actual billed cost", "API-Modus: Beträge sind tatsächliche Kosten"],

  "f.win5h": ["5 小时", "Session", "Session"],
  "f.winWeekAll": ["周·全部", "Wk · all", "Wo · alle"],
  "f.winWeekPrefix": ["周·", "Wk · ", "Wo · "],
  "f.allProjects": ["全部项目", "All projects", "Alle Projekte"],
  "f.est": ["est", "est", "ca."],
  "f.workDays": [" 工作日", " wd", " AT"],
  "f.reset": ["重置", "resets", "Reset"],
  "f.exhaust": ["耗尽", "empty", "leer"],
  "f.limitErr": ["用量接口不可用（钥匙串未授权？）", "usage API unavailable (keychain denied?)", "Usage-API nicht verfügbar (Schlüsselbund?)"],
  "o.limits": ["订阅限额", "Subscription limits", "Abo-Kontingente"],

  // —— 总览 ——
  "o.today": ["今日", "Today", "Heute"],
  "o.week": ["近 7 天", "Last 7 days", "Letzte 7 Tage"],
  "o.month": ["近 30 天", "Last 30 days", "Letzte 30 Tage"],
  "o.all": ["全部", "All time", "Gesamt"],
  "o.cacheR": ["cache读", "cache r", "Cache-L"],
  "o.cacheW": ["写", "w", "S"],
  "o.chartDaily": ["近 30 天成本（按模型）", "Cost, last 30 days (by model)", "Kosten, letzte 30 Tage (nach Modell)"],
  "o.chartModels": ["模型占比", "By model", "Nach Modell"],
  "o.chartSide": ["主对话 vs Subagent", "Main vs subagent", "Hauptdialog vs. Subagent"],
  "o.cost": ["成本", "Cost", "Kosten"],
  "o.noteSub": ["订阅模式：以下所有金额是<b>等值 API 成本</b>——这些 token 若按 API 价格计费需要花多少钱。你的实际支出是订阅费本身；这个数字越高，说明订阅越划算。", "Subscription mode: all amounts below are <b>API-equivalent cost</b> — what these tokens would cost at API prices. Your actual spend is the subscription fee; the higher this number, the better the deal.", "Abo-Modus: Alle Beträge sind <b>API-Äquivalenzkosten</b> — was diese Tokens zu API-Preisen kosten würden. Ihre tatsächlichen Ausgaben sind die Abo-Gebühr; je höher die Zahl, desto besser das Abo."],
  "o.noteApi": ["API 模式：以下金额为实际计费成本。", "API mode: amounts below are actual billed cost.", "API-Modus: Die Beträge sind tatsächliche Kosten."],
  "o.subTitle": ["订阅对比", "Subscription comparison", "Abo-Vergleich"],
  "o.days": ["天", "days", "Tage"],
  "o.from": ["起", "since", "seit"],
  "o.actual": ["订阅实付（折算）", "Actual paid (prorated)", "Tatsächlich gezahlt (anteilig)"],
  "o.equiv": ["等值 API 成本", "API-equivalent cost", "API-Äquivalenzkosten"],
  "o.saved": ["省下", "Saved", "Gespart"],
  "o.leverage": ["杠杆", "Leverage", "Hebel"],
  "o.thisMonth": ["本月", "This month", "Dieser Monat"],
  "o.equivShort": ["等值", "equiv.", "äquiv."],
  "o.monthFee": ["月费", "monthly fee", "Monatsgebühr"],
  "o.apiExtra": ["另有 API 实付", "plus actual API spend", "zzgl. API-Ausgaben"],
  "o.notAllocated": ["未参与分摊", "not allocated", "nicht umgelegt"],
  "o.fillFeeHint": ["想看订阅实付 vs 等值成本的对比？到<a href=\"#settings\">设置页</a>填一下订阅月费即可。", "Want the actual-vs-equivalent comparison? Enter your subscription fee on the <a href=\"#settings\">settings page</a>.", "Für den Vergleich tatsächlich vs. äquivalent: Abo-Gebühr auf der <a href=\"#settings\">Einstellungsseite</a> eintragen."],

  // —— 项目页 / 会话 ——
  "p.title": ["项目", "Projects", "Projekte"],
  "p.equiv": ["等值成本", "Equiv. cost", "Äquiv. Kosten"],
  "p.alloc": ["分摊实付", "Allocated paid", "Umgelegt bezahlt"],
  "p.sessions": ["会话", "Sessions", "Sitzungen"],
  "p.activeDays": ["活跃天数", "Active days", "Aktive Tage"],
  "p.last": ["最近活动", "Last active", "Zuletzt aktiv"],
  "p.back": ["← 项目列表", "← Projects", "← Projekte"],
  "p.requests": ["次请求", "requests", "Anfragen"],
  "e.time": ["时间", "Time", "Zeit"],
  "e.model": ["模型", "Model", "Modell"],
  "e.type": ["类型", "Type", "Typ"],
  "e.think": ["think", "think", "think"],
  "e.cacheW": ["cache写", "cache w", "Cache-S"],
  "e.cacheR": ["cache读", "cache r", "Cache-L"],
  "e.cost": ["成本", "Cost", "Kosten"],
  "e.main": ["主", "main", "Haupt"],
  "s.title": ["会话明细", "Sessions", "Sitzungen"],
  "s.recent": ["最近 {n} 个会话（全部项目，点击展开逐请求）", "Latest {n} sessions (all projects; click to expand per-request)", "Letzte {n} Sitzungen (alle Projekte; zum Aufklappen klicken)"],

  "m.files": ["代码文件", "code files", "Code-Dateien"],
  "m.code": ["代码量", "code size", "Code-Umfang"],
  "m.commits": ["提交", "commits", "Commits"],
  "m.days": ["快照天数", "snapshot days", "Snapshot-Tage"],

  // —— 设置 ——
  "st.title": ["设置", "Settings", "Einstellungen"],
  "st.prices": ["价格数据", "Pricing data", "Preisdaten"],
  "st.knownModels": ["已知 {n} 个模型", "{n} models known", "{n} Modelle bekannt"],
  "st.lastUpdate": ["最后更新", "Last update", "Letzte Aktualisierung"],
  "st.never": ["从未（使用内置快照）", "never (using bundled snapshot)", "nie (integrierter Snapshot)"],
  "st.status": ["状态", "Status", "Status"],
  "st.refresh": ["立即刷新价格", "Refresh prices now", "Preise jetzt aktualisieren"],
  "st.billing": ["计费口径（当前：{m}）", "Billing basis (current: {m})", "Abrechnungsbasis (aktuell: {m})"],
  "st.bSub": ["订阅（显示等值 API 成本）", "subscription (API-equivalent cost)", "Abo (API-Äquivalenzkosten)"],
  "st.bApi": ["API（实际计费成本）", "API (actual cost)", "API (tatsächliche Kosten)"],
  "st.bUnknown": ["未知", "unknown", "unbekannt"],
  "st.auto": ["自动检测", "Auto-detect", "Automatisch"],
  "st.forceSub": ["强制：订阅", "Force: subscription", "Erzwingen: Abo"],
  "st.forceApi": ["强制：API", "Force: API", "Erzwingen: API"],
  "st.data": ["数据", "Data", "Daten"],
  "st.db": ["数据库", "Database", "Datenbank"],
  "st.skipped": ["跳过行", "skipped lines", "übersprungene Zeilen"],
  "st.bad": ["坏行", "bad lines", "fehlerhafte Zeilen"],
  "st.backfill": ["全量重扫", "Full rescan", "Kompletter Rescan"],
  "st.exportMd": ["导出 Markdown", "Export Markdown", "Markdown exportieren"],
  "st.exportCsv": ["导出 CSV", "Export CSV", "CSV exportieren"],
  "st.exportJson": ["导出 JSON", "Export JSON", "JSON exportieren"],
  "st.fees": ["订阅月费（用于实付 vs 等值成本对比；检测到当前档位：{t}）", "Subscription fee (for actual-vs-equivalent comparison; detected tier: {t})", "Abo-Gebühr (für Vergleich tatsächlich vs. äquivalent; erkannte Stufe: {t})"],
  "st.noFees": ["尚未填写——填了才有订阅对比面板", "not set — required for the comparison panel", "nicht gesetzt — nötig für das Vergleichspanel"],
  "st.feeFrom": ["起", "from", "ab"],
  "st.perMonth": ["/月", "/mo", "/Monat"],
  "st.delete": ["删除", "Delete", "Löschen"],
  "st.feeUsd": ["美元/月", "USD/mo", "USD/Monat"],
  "st.addFee": ["添加/覆盖该日期起的月费", "Add/override fee from this date", "Gebühr ab Datum setzen"],
  "st.feeHint": ["换过档位就加多段（如：历史 $100，升级日起 $200），成本按天折算跨段累加。", "Changed tiers? Add segments (e.g. $100 before, $200 since upgrade); cost is prorated by day across segments.", "Stufe gewechselt? Segmente anlegen (z. B. $100 vorher, $200 ab Upgrade); Kosten werden tageweise anteilig berechnet."],
  "st.appearance": ["外观", "Appearance", "Darstellung"],
  "st.theme": ["主题", "Theme", "Design"],
  "st.light": ["浅色", "Light", "Hell"],
  "st.dark": ["深色", "Dark", "Dunkel"],
  "st.lang": ["语言", "Language", "Sprache"],
  "st.langAuto": ["跟随系统", "System default", "Systemsprache"],
  "st.opacity": ["悬浮窗透明度", "Float window opacity", "Fenster-Deckkraft"],
  "st.startup": ["启动", "Startup", "Start"],
  "st.autostart": ["开机自动启动", "Launch at login", "Beim Anmelden starten"],
  "st.quit": ["退出 Bookholder（停止采集）", "Quit Bookholder (stops tracking)", "Bookholder beenden (stoppt Erfassung)"],
  "st.done": ["完成", "Done", "Fertig"],
  "st.failed": ["失败：{e}", "Failed: {e}", "Fehlgeschlagen: {e}"],
  "st.exported": ["已导出 {p}", "Exported {p}", "Exportiert: {p}"],
  "st.saved": ["已保存", "Saved", "Gespeichert"],
  "st.fillFirst": ["请先选日期并填月费", "Pick a date and enter a fee first", "Bitte erst Datum und Gebühr angeben"],
  "st.backfillDone": ["重扫完成：新增 {a}，跳过 {s}，坏行 {b}", "Rescan done: {a} added, {s} skipped, {b} bad", "Rescan fertig: {a} neu, {s} übersprungen, {b} fehlerhaft"],
  "st.autoOn": ["已开启自启", "Launch at login enabled", "Autostart aktiviert"],
  "st.autoOff": ["已关闭自启", "Launch at login disabled", "Autostart deaktiviert"],
};

const IDX: Record<Lang, number> = { zh: 0, en: 1, de: 2 };

export function t(key: string): string {
  const row = D[key];
  return row ? row[IDX[lang]] : key;
}

/** 简单插值：t2("st.knownModels", { n: 51 }) */
export function t2(key: string, vars: Record<string, string | number>): string {
  let s = t(key);
  for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}

/** 静态 HTML：填充 [data-i18n] 文本与 [data-i18n-title] 提示。 */
export function applyStaticI18n(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n!);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle!);
  });
}
