"use strict";

(() => {
  const STORAGE_KEY = "color-scheme";
  const colorSchemeMql = window.matchMedia("(prefers-color-scheme: dark)");
  const EXAMPLE_URL_PLACEHOLDER = ["https:", "//api.example.com/v1/ping"].join("");
  const SENSITIVE_HEADER_NAMES = new Set([
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
  ]);

  const HTTP_REASONS = {
    100: "Continue",
    101: "Switching Protocols",
    200: "OK",
    201: "Created",
    202: "Accepted",
    204: "No Content",
    301: "Moved Permanently",
    302: "Found",
    304: "Not Modified",
    307: "Temporary Redirect",
    308: "Permanent Redirect",
    400: "Bad Request",
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    405: "Method Not Allowed",
    408: "Request Timeout",
    409: "Conflict",
    413: "Payload Too Large",
    422: "Unprocessable Entity",
    429: "Too Many Requests",
    500: "Internal Server Error",
    502: "Bad Gateway",
    503: "Service Unavailable",
    504: "Gateway Timeout",
  };

  const state = {
    mode: "cli",
    system: null,
    meta: null,
    about: null,
    changelog: null,
    changelogExpanded: false,
    snapshots: [],
    liveSnapshots: [],
    renderedSnapshots: [],
    finalStats: null,
    liveFinalStats: null,
    liveMeta: null,
    activeRunId: null,
    viewingRunId: null,
    paused: false,
    completed: false,
    stopRequested: false,
    hadDisconnect: false,
    eventSource: null,
    presets: [],
    recent: [],
    selectedPresetId: null,
    presetSaveMode: "create",
    formDirty: false,
    bodyMode: "none",
    headers: [],
    runs: [],
    compareIds: [],
    hoverIndex: {
      throughput: null,
      latency: null,
      errors: null,
    },
  };

  const els = {
    metaColorScheme: document.querySelector('meta[name="color-scheme"]'),
    formLiveAnnouncer: document.getElementById("form-live-announcer"),
    toggleSidebarBtn: document.getElementById("toggle-sidebar-btn"),
    workspaceLayout: document.getElementById("workspace-layout"),
    configSidebar: document.getElementById("config-sidebar"),
    appVersionPill: document.getElementById("app-version-pill"),
    phaseBadge: document.getElementById("phase-badge"),
    metaMethod: document.getElementById("meta-method"),
    metaUrl: document.getElementById("meta-url"),
    timerElapsed: document.getElementById("timer-elapsed"),
    timerDuration: document.getElementById("timer-duration"),
    runProgress: document.getElementById("run-progress"),
    runProgressFill: document.getElementById("run-progress-fill"),
    returnLiveBtn: document.getElementById("return-live-btn"),
    pauseChartsBtn: document.getElementById("pause-charts-btn"),
    exportMenuTrigger: document.getElementById("export-menu-trigger"),
    exportMenu: document.getElementById("export-menu"),
    exportMenuList: document.querySelector("#export-menu [focusgroup]"),
    aboutBtn: document.getElementById("about-btn"),
    aboutDialog: document.getElementById("about-dialog"),
    aboutVersionBadge: document.getElementById("about-version-badge"),
    aboutDescription: document.getElementById("about-description"),
    aboutLatestMeta: document.getElementById("about-latest-meta"),
    aboutLatestCard: document.getElementById("about-latest-card"),
    toggleChangelogBtn: document.getElementById("toggle-changelog-btn"),
    aboutChangelogContainer: document.getElementById("about-changelog-container"),
    themeToggleBtn: document.getElementById("theme-toggle"),
    stopRunBtn: document.getElementById("stop-run-btn"),
    connectionBanner: document.getElementById("connection-banner"),
    kpiWindowRps: document.getElementById("kpi-window-rps"),
    kpiAvgRps: document.getElementById("kpi-avg-rps"),
    kpiActiveVus: document.getElementById("kpi-active-vus"),
    kpiRampUp: document.getElementById("kpi-ramp-up"),
    kpiTotalReqs: document.getElementById("kpi-total-reqs"),
    kpiTotalBytes: document.getElementById("kpi-total-bytes"),
    kpiSuccessRate: document.getElementById("kpi-success-rate"),
    kpiErrorRate: document.getElementById("kpi-error-rate"),
    kpiLatencySummary: document.getElementById("kpi-latency-summary"),
    kpiLatencyMax: document.getElementById("kpi-latency-max"),
    kpiTransferRate: document.getElementById("kpi-transfer-rate"),
    kpiTimeout: document.getElementById("kpi-timeout"),
    finalSummaryPanel: document.getElementById("final-summary-panel"),
    finalDurationMeta: document.getElementById("final-duration-meta"),
    finalSummaryGrid: document.getElementById("final-summary-grid"),
    statusTotalMeta: document.getElementById("status-total-meta"),
    statusTableBody: document.getElementById("status-table-body"),
    errorTotalMeta: document.getElementById("error-total-meta"),
    errorTableBody: document.getElementById("error-table-body"),
    metaBodyInfo: document.getElementById("meta-body-info"),
    metaHeadersBody: document.getElementById("meta-headers-body"),
    chartThroughput: document.getElementById("chart-throughput"),
    chartLatency: document.getElementById("chart-latency"),
    chartErrors: document.getElementById("chart-errors"),
    tooltipThroughput: document.getElementById("tooltip-throughput"),
    tooltipLatency: document.getElementById("tooltip-latency"),
    tooltipErrors: document.getElementById("tooltip-errors"),
    presetDirtyPill: document.getElementById("preset-dirty-pill"),
    presetSelect: document.getElementById("preset-select"),
    presetNewBtn: document.getElementById("preset-new-btn"),
    presetSaveBtn: document.getElementById("preset-save-btn"),
    presetSaveAsBtn: document.getElementById("preset-save-as-btn"),
    presetDeleteBtn: document.getElementById("preset-delete-btn"),
    presetDeleteConfirm: document.getElementById("preset-delete-confirm"),
    presetDeleteConfirmBtn: document.getElementById("preset-delete-confirm-btn"),
    presetDeleteCancelBtn: document.getElementById("preset-delete-cancel-btn"),
    presetSavePanel: document.getElementById("preset-save-panel"),
    presetNameInput: document.getElementById("preset-name-input"),
    presetDescInput: document.getElementById("preset-desc-input"),
    presetCommitBtn: document.getElementById("preset-commit-btn"),
    presetCancelBtn: document.getElementById("preset-cancel-btn"),
    configForm: document.getElementById("config-form"),
    cfgMethod: document.getElementById("cfg-method"),
    cfgUrl: document.getElementById("cfg-url"),
    cfgConcurrency: document.getElementById("cfg-concurrency"),
    cfgDuration: document.getElementById("cfg-duration"),
    cfgRampUp: document.getElementById("cfg-ramp-up"),
    cfgTimeout: document.getElementById("cfg-timeout"),
    addHeaderBtn: document.getElementById("add-header-btn"),
    headersList: document.getElementById("headers-list"),
    bodyTabs: Array.from(document.querySelectorAll('[role="tab"][data-body-mode]')),
    bodyPanels: {
      none: document.getElementById("body-panel-none"),
      raw: document.getElementById("body-panel-raw"),
      file: document.getElementById("body-panel-file"),
    },
    bodyRawBytes: document.getElementById("body-raw-bytes"),
    formatJsonBtn: document.getElementById("format-json-btn"),
    cfgBodyRaw: document.getElementById("cfg-body-raw"),
    cfgBodyFile: document.getElementById("cfg-body-file"),
    chooseBodyFileBtn: document.getElementById("choose-body-file-btn"),
    bodyFileMeta: document.getElementById("body-file-meta"),
    autoExportDetails: document.getElementById("auto-export-details"),
    cfgOutputJson: document.getElementById("cfg-output-json"),
    cfgOutputCsv: document.getElementById("cfg-output-csv"),
    cfgOutputPdf: document.getElementById("cfg-output-pdf"),
    configOsWarning: document.getElementById("config-os-warning"),
    configFormError: document.getElementById("config-form-error"),
    startRunBtn: document.getElementById("start-run-btn"),
    copyCliBtn: document.getElementById("copy-cli-btn"),
    cliPreviewBox: document.getElementById("cli-preview-box"),
    runHistoryPanel: document.getElementById("run-history-panel"),
    runHistoryCount: document.getElementById("run-history-count"),
    runComparePanel: document.getElementById("run-compare-panel"),
    runHistoryBody: document.getElementById("run-history-body"),
  };

  function sessionToken() {
    return typeof window.__RL_SESSION_TOKEN__ === "string" && window.__RL_SESSION_TOKEN__.length > 0
      ? window.__RL_SESSION_TOKEN__
      : null;
  }

  async function apiFetch(url, options = {}) {
    const headers = new Headers(options.headers || {});
    const token = sessionToken();
    if (token) {
      headers.set("X-RL-Session", token);
    }
    return fetch(url, {
      cache: "no-store",
      credentials: "same-origin",
      ...options,
      headers,
    });
  }

  function isSensitiveHeaderName(name) {
    const lower = String(name || "").trim().toLowerCase();
    return (
      SENSITIVE_HEADER_NAMES.has(lower) ||
      lower.includes("token") ||
      lower.includes("secret") ||
      lower.includes("password") ||
      lower.includes("credential") ||
      lower.includes("key")
    );
  }

  function readStoredScheme() {
    try {
      const value = localStorage.getItem(STORAGE_KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch (_) {
      return null;
    }
  }

  function writeStoredScheme(value) {
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch (_) {}
  }

  function removeStoredScheme() {
    try {
      localStorage.removeItem(STORAGE_KEY);
    } catch (_) {}
  }

  function systemScheme() {
    return colorSchemeMql.matches ? "dark" : "light";
  }

  function renderedScheme() {
    return readStoredScheme() ?? systemScheme();
  }

  function applyMetaScheme(value) {
    if (els.metaColorScheme) {
      els.metaColorScheme.content = value;
    }
  }

  function initThemeControls() {
    if (els.themeToggleBtn) {
      els.themeToggleBtn.addEventListener("click", () => {
        const target = renderedScheme() === "dark" ? "light" : "dark";
        if (target !== systemScheme()) {
          writeStoredScheme(target);
          applyMetaScheme(target);
        } else {
          removeStoredScheme();
          applyMetaScheme("light dark");
        }
        drawAllCharts();
      });
    }

    colorSchemeMql.addEventListener("change", () => {
      drawAllCharts();
    });

    window.addEventListener("storage", (event) => {
      if (event.key === STORAGE_KEY || event.key === null) {
        applyMetaScheme(readStoredScheme() ?? "light dark");
        drawAllCharts();
      }
    });
  }

  function formatNumber(n) {
    return Number(n || 0).toLocaleString();
  }

  function formatBytes(bytes) {
    const b = Number(bytes || 0);
    const kb = 1024;
    const mb = kb * 1024;
    const gb = mb * 1024;
    if (b < kb) return `${Math.round(b)} B`;
    if (b < mb) return `${(b / kb).toFixed(2)} KB`;
    if (b < gb) return `${(b / mb).toFixed(2)} MB`;
    return `${(b / gb).toFixed(2)} GB`;
  }

  function formatRate(bytesPerSec) {
    const bps = Number(bytesPerSec || 0);
    const kb = 1024;
    const mb = kb * 1024;
    const gb = mb * 1024;
    if (bps < kb) return `${bps.toFixed(2)} B/s`;
    if (bps < mb) return `${(bps / kb).toFixed(2)} KB/s`;
    if (bps < gb) return `${(bps / mb).toFixed(2)} MB/s`;
    return `${(bps / gb).toFixed(2)} GB/s`;
  }

  function formatMs(ms) {
    return `${Number(ms || 0).toFixed(2)} ms`;
  }

  function formatSeconds(secs) {
    return `${Number(secs || 0).toFixed(1)}s`;
  }

  function showBanner(message, isError = false) {
    if (!els.connectionBanner) return;
    els.connectionBanner.textContent = message;
    els.connectionBanner.classList.toggle("notice-error", isError);
    els.connectionBanner.hidden = false;
  }

  function hideBanner() {
    if (!els.connectionBanner) return;
    els.connectionBanner.hidden = true;
    els.connectionBanner.classList.remove("notice-error");
    els.connectionBanner.textContent = "";
  }

  function setPhaseBadge(phase) {
    if (!els.phaseBadge) return;
    els.phaseBadge.className = "badge";
    switch (phase) {
      case "idle":
        els.phaseBadge.textContent = "Idle";
        els.phaseBadge.classList.add("badge-idle");
        break;
      case "ramping_up":
        els.phaseBadge.textContent = "Ramping Up";
        els.phaseBadge.classList.add("badge-ramping");
        break;
      case "draining":
        els.phaseBadge.textContent = "Draining";
        els.phaseBadge.classList.add("badge-draining");
        break;
      case "completed":
        els.phaseBadge.textContent = "Completed";
        els.phaseBadge.classList.add("badge-completed");
        break;
      case "failed":
        els.phaseBadge.textContent = "Failed";
        els.phaseBadge.classList.add("badge-failed");
        break;
      case "error":
        els.phaseBadge.textContent = "Disconnected";
        els.phaseBadge.classList.add("badge-error");
        break;
      default:
        els.phaseBadge.textContent = "Running";
        els.phaseBadge.classList.add("badge-running");
        break;
    }
  }

  function isActivePhase(phase) {
    return phase === "ramping_up" || phase === "running" || phase === "draining";
  }

  function syncRunButtonsForPhase(phase) {
    if (state.mode !== "interactive") return;
    const running = isActivePhase(phase);
    if (els.startRunBtn) {
      els.startRunBtn.disabled = running;
      els.startRunBtn.textContent = running ? "Benchmark Running..." : "Start Load Test";
    }
    if (els.stopRunBtn) {
      els.stopRunBtn.disabled = !running || phase === "draining";
      els.stopRunBtn.textContent =
        phase === "draining"
          ? "Stopping..."
          : running
            ? "Stop Run"
            : phase === "completed"
              ? "Completed"
              : phase === "failed"
                ? "Failed"
                : "Stop Run";
    }
  }

  function updateMetaDisplay(meta) {
    state.meta = meta;
    els.metaMethod.textContent = meta.method || "GET";
    els.metaUrl.textContent = meta.url || (meta.phase === "idle" ? "Configure a target to run" : "--");
    els.metaUrl.title = meta.url || "";
    els.timerDuration.textContent = formatSeconds(meta.duration_secs);
    els.kpiRampUp.textContent = `Ramp-up: ${Number(meta.ramp_up_secs || 0).toFixed(1)}s`;
    els.kpiTimeout.textContent = `Timeout: ${meta.timeout_ms} ms`;

    if (meta.has_body) {
      els.metaBodyInfo.textContent = `Request body: ${formatBytes(meta.body_size_bytes)}`;
    } else {
      els.metaBodyInfo.textContent = "No request body";
    }

    if (!state.completed || state.mode === "interactive") {
      setPhaseBadge(meta.phase);
    }

    syncRunButtonsForPhase(meta.phase);
    renderMetaTable(meta);

    if (state.snapshots.length === 0) {
      els.kpiActiveVus.textContent = `0 / ${meta.concurrency}`;
    }
  }

  function renderMetaTable(meta) {
    const tbody = els.metaHeadersBody;
    tbody.replaceChildren();

    const rows = [
      ["Target URL", meta.url || "--"],
      ["HTTP Method", meta.method || "GET"],
      ["Target Concurrency", `${meta.concurrency} VUs`],
      ["Configured Duration", `${Number(meta.duration_secs).toFixed(2)} s`],
      ["Ramp-Up Duration", `${Number(meta.ramp_up_secs).toFixed(2)} s`],
      ["Request Timeout", `${meta.timeout_ms} ms`],
    ];
    if (meta.preset_name) {
      rows.unshift(["Preset", meta.preset_name]);
    }

    const headerEntries = Object.entries(meta.headers || {}).sort(([a], [b]) => a.localeCompare(b));
    for (const [key, val] of headerEntries) {
      rows.push([`Header: ${key}`, val]);
    }

    for (const [label, value] of rows) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = label;
      const td = document.createElement("td");
      td.className = "mono";
      td.textContent = String(value);
      tr.append(th, td);
      tbody.appendChild(tr);
    }
  }

  function resetTelemetryDisplay() {
    state.snapshots = [];
    state.renderedSnapshots = [];
    state.finalStats = null;
    state.completed = false;
    state.stopRequested = false;

    els.kpiWindowRps.textContent = "0.0 req/s";
    els.kpiAvgRps.textContent = "Avg: 0.0 req/s";
    els.kpiActiveVus.textContent = `0 / ${state.meta ? state.meta.concurrency : 0}`;
    els.kpiTotalReqs.textContent = "0";
    els.kpiTotalBytes.textContent = "Transferred: 0 B";
    els.kpiSuccessRate.textContent = "100.00%";
    els.kpiErrorRate.textContent = "Errors: 0 (0.00%)";
    els.kpiLatencySummary.textContent = "0.00 / 0.00 / 0.00 ms";
    els.kpiLatencyMax.textContent = "Window Max: 0.00 ms";
    els.kpiTransferRate.textContent = "0.00 B/s";
    els.timerElapsed.textContent = "0.0s";
    els.runProgress.setAttribute("aria-valuenow", "0");
    els.runProgressFill.style.inlineSize = "0%";
    els.finalSummaryPanel.hidden = true;
    els.finalSummaryGrid.replaceChildren();
    renderStatusTable({}, 0);
    renderErrorTable({}, 0);
    drawAllCharts();
  }

  function updateKpisFromSnapshot(snap) {
    const targetConcurrency = state.meta ? state.meta.concurrency : snap.active_workers;
    const durationSecs = snap.duration_secs || (state.meta ? state.meta.duration_secs : 0);

    els.kpiWindowRps.textContent = `${snap.window_rps.toFixed(1)} req/s`;
    els.kpiAvgRps.textContent = `Avg: ${snap.avg_rps.toFixed(1)} req/s`;
    els.kpiActiveVus.textContent = `${snap.active_workers} / ${targetConcurrency}`;
    els.kpiTotalReqs.textContent = formatNumber(snap.total_requests);
    els.kpiTotalBytes.textContent = `Transferred: ${formatBytes(snap.total_bytes)}`;

    const totalErrors = snap.client_errors + snap.server_errors + snap.transport_errors;
    const total = snap.total_requests || 0;
    const successPct = total > 0 ? (snap.successful_requests / total) * 100 : 100;
    const errorPct = total > 0 ? (totalErrors / total) * 100 : 0;

    els.kpiSuccessRate.textContent = `${successPct.toFixed(2)}%`;
    els.kpiErrorRate.textContent = `Errors: ${formatNumber(totalErrors)} (${errorPct.toFixed(2)}%)`;

    const w = snap.window_latency_ms;
    els.kpiLatencySummary.textContent = `${w.p50.toFixed(2)} / ${w.p95.toFixed(2)} / ${w.p99.toFixed(2)} ms`;
    els.kpiLatencyMax.textContent = `Window Max: ${w.max.toFixed(2)} ms`;
    els.kpiTransferRate.textContent = formatRate(snap.bytes_per_sec);

    els.timerElapsed.textContent = formatSeconds(snap.elapsed_secs);
    if (durationSecs > 0) {
      els.timerDuration.textContent = formatSeconds(durationSecs);
      const pct = Math.min(100, Math.max(0, (snap.elapsed_secs / durationSecs) * 100));
      els.runProgress.setAttribute("aria-valuenow", Math.round(pct).toString());
      els.runProgressFill.style.inlineSize = `${pct.toFixed(1)}%`;
    }

    if (!state.completed) {
      setPhaseBadge(snap.phase);
      syncRunButtonsForPhase(snap.phase);
    }

    renderStatusTable(snap.status_distribution, snap.total_requests);
    renderErrorTable(snap.error_breakdown, snap.total_requests);
  }

  function renderStatusTable(statusDistribution, totalRequests) {
    const tbody = els.statusTableBody;
    tbody.replaceChildren();

    const entries = Object.entries(statusDistribution || {}).sort(
      ([a], [b]) => Number(a) - Number(b)
    );
    els.statusTotalMeta.textContent = `${entries.length} code${entries.length === 1 ? "" : "s"}`;

    if (entries.length === 0) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = "--";
      const tdReason = document.createElement("td");
      tdReason.textContent = "No responses recorded yet";
      const tdCount = document.createElement("td");
      tdCount.textContent = "0";
      const tdShare = document.createElement("td");
      tdShare.textContent = "0.00%";
      tr.append(th, tdReason, tdCount, tdShare);
      tbody.appendChild(tr);
      return;
    }

    for (const [codeStr, count] of entries) {
      const code = Number(codeStr);
      const reason = HTTP_REASONS[code] || "HTTP Status";
      const share = totalRequests > 0 ? (count / totalRequests) * 100 : 0;

      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = codeStr;
      const tdReason = document.createElement("td");
      tdReason.textContent = reason;
      const tdCount = document.createElement("td");
      tdCount.textContent = formatNumber(count);
      const tdShare = document.createElement("td");
      tdShare.textContent = `${share.toFixed(2)}%`;
      tr.append(th, tdReason, tdCount, tdShare);
      tbody.appendChild(tr);
    }
  }

  function renderErrorTable(errorBreakdown, totalRequests) {
    const tbody = els.errorTableBody;
    tbody.replaceChildren();

    const entries = Object.entries(errorBreakdown || {}).sort((a, b) => b[1] - a[1]);
    const totalErrors = entries.reduce((acc, [, c]) => acc + c, 0);
    els.errorTotalMeta.textContent = `${formatNumber(totalErrors)} error${totalErrors === 1 ? "" : "s"}`;

    if (entries.length === 0) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = "None";
      const tdCount = document.createElement("td");
      tdCount.textContent = "0";
      const tdShare = document.createElement("td");
      tdShare.textContent = "0.00%";
      tr.append(th, tdCount, tdShare);
      tbody.appendChild(tr);
      return;
    }

    for (const [category, count] of entries) {
      const share = totalRequests > 0 ? (count / totalRequests) * 100 : 0;
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = category;
      const tdCount = document.createElement("td");
      tdCount.textContent = formatNumber(count);
      const tdShare = document.createElement("td");
      tdShare.textContent = `${share.toFixed(2)}%`;
      tr.append(th, tdCount, tdShare);
      tbody.appendChild(tr);
    }
  }

  function applyFinalStats(stats) {
    state.completed = true;
    state.finalStats = stats;
    setPhaseBadge("completed");
    syncRunButtonsForPhase("completed");

    if (!state.viewingRunId) {
      hideBanner();
    }

    if (state.mode === "cli" && els.stopRunBtn) {
      els.stopRunBtn.disabled = true;
      els.stopRunBtn.textContent = "Completed";
    }

    els.kpiAvgRps.textContent = `Final Avg: ${stats.requests_per_sec.toFixed(1)} req/s`;
    els.kpiTotalReqs.textContent = formatNumber(stats.total_requests);
    els.kpiTotalBytes.textContent = `Transferred: ${formatBytes(stats.total_bytes)}`;
    els.kpiTransferRate.textContent = formatRate(stats.bytes_per_sec);
    els.kpiActiveVus.textContent = `0 / ${state.meta ? state.meta.concurrency : 0}`;

    const totalErrors = stats.client_errors + stats.server_errors + stats.transport_errors;
    const total = stats.total_requests || 0;
    const successPct = total > 0 ? (stats.successful_requests / total) * 100 : 100;
    const errorPct = total > 0 ? (totalErrors / total) * 100 : 0;
    els.kpiSuccessRate.textContent = `${successPct.toFixed(2)}%`;
    els.kpiErrorRate.textContent = `Errors: ${formatNumber(totalErrors)} (${errorPct.toFixed(2)}%)`;

    els.timerElapsed.textContent = formatSeconds(stats.total_duration_secs);
    els.runProgress.setAttribute("aria-valuenow", "100");
    els.runProgressFill.style.inlineSize = "100%";

    renderStatusTable(stats.status_distribution, stats.total_requests);
    renderErrorTable(stats.error_breakdown, stats.total_requests);

    const lat = stats.latencies;
    const summaryItems = [
      ["Total Requests", formatNumber(stats.total_requests)],
      ["Avg RPS", `${stats.requests_per_sec.toFixed(1)} req/s`],
      ["Min", formatMs(lat.min_ms)],
      ["Mean", formatMs(lat.mean_ms)],
      ["Median (p50)", formatMs(lat.p50_ms)],
      ["p90", formatMs(lat.p90_ms)],
      ["p95", formatMs(lat.p95_ms)],
      ["p99", formatMs(lat.p99_ms)],
      ["p99.9", formatMs(lat.p99_9_ms)],
      ["Max", formatMs(lat.max_ms)],
      ["Std Dev", formatMs(lat.std_dev_ms)],
      ["Transfer Rate", formatRate(stats.bytes_per_sec)],
    ];

    els.finalDurationMeta.textContent = `Completed in ${stats.total_duration_secs.toFixed(2)}s`;
    els.finalSummaryGrid.replaceChildren();

    for (const [label, value] of summaryItems) {
      const item = document.createElement("div");
      item.className = "summary-stat-item";
      const k = document.createElement("div");
      k.className = "stat-k";
      k.textContent = label;
      const v = document.createElement("div");
      v.className = "stat-v";
      v.textContent = value;
      item.append(k, v);
      els.finalSummaryGrid.appendChild(item);
    }

    els.finalSummaryPanel.hidden = false;
  }

  function mergeSnapshots(incoming) {
    if (!Array.isArray(incoming) || incoming.length === 0) return;

    const byElapsed = new Map();
    for (const snap of state.snapshots) {
      byElapsed.set(`${snap.timestamp_ms}_${snap.elapsed_secs.toFixed(4)}`, snap);
    }
    for (const snap of incoming) {
      byElapsed.set(`${snap.timestamp_ms}_${snap.elapsed_secs.toFixed(4)}`, snap);
    }
    state.snapshots = Array.from(byElapsed.values()).sort(
      (a, b) => a.elapsed_secs - b.elapsed_secs
    );
    if (!state.viewingRunId) {
      state.liveSnapshots = state.snapshots.slice();
    }
    if (!state.paused) {
      state.renderedSnapshots = state.snapshots.slice();
    }
    const latest = state.snapshots[state.snapshots.length - 1];
    if (latest) {
      updateKpisFromSnapshot(latest);
    }
    if (!state.paused) {
      drawAllCharts();
    }
  }

  function appendSnapshot(snap) {
    state.liveSnapshots.push(snap);
    if (state.liveSnapshots.length > 3600) {
      state.liveSnapshots.shift();
    }
    if (state.viewingRunId) {
      return;
    }

    state.snapshots.push(snap);
    if (state.snapshots.length > 3600) {
      state.snapshots.shift();
    }
    updateKpisFromSnapshot(snap);
    if (!state.paused) {
      state.renderedSnapshots = state.snapshots.slice();
      drawAllCharts();
    }
  }

  function computePerTickSeries(snaps) {
    const rows = [];
    for (let i = 0; i < snaps.length; i += 1) {
      const cur = snaps[i];
      const prev = i > 0 ? snaps[i - 1] : null;
      rows.push({
        elapsed_secs: cur.elapsed_secs,
        ok2xx: Math.max(0, cur.successful_requests - (prev ? prev.successful_requests : 0)),
        err4xx: Math.max(0, cur.client_errors - (prev ? prev.client_errors : 0)),
        err5xx: Math.max(0, cur.server_errors - (prev ? prev.server_errors : 0)),
        errNet: Math.max(0, cur.transport_errors - (prev ? prev.transport_errors : 0)),
      });
    }
    return rows;
  }

  function getCssToken(name, fallback) {
    const val = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return val || fallback;
  }

  function drawChartCanvas(canvas, hoverIdx, spec) {
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 10 || rect.height <= 10) return;

    const dpr = window.devicePixelRatio || 1;
    const width = Math.round(rect.width);
    const height = Math.round(rect.height);
    if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
    }

    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const isDark = renderedScheme() === "dark";
    const textMuted = getCssToken(
      isDark ? "--text-muted-dark" : "--text-muted-light",
      isDark ? "#94a3b8" : "#64748b"
    );
    const gridColor = getCssToken(
      isDark ? "--grid-line-dark" : "--grid-line-light",
      isDark ? "rgba(241, 245, 249, 0.09)" : "rgba(15, 23, 42, 0.08)"
    );
    const pad = { top: 14, right: spec.rightAxis ? 48 : 14, bottom: 26, left: 56 };
    const plotW = Math.max(20, width - pad.left - pad.right);
    const plotH = Math.max(20, height - pad.top - pad.bottom);

    const data = spec.data;
    if (!data || data.length === 0) {
      ctx.fillStyle = textMuted;
      ctx.font = "12px sans-serif";
      ctx.textAlign = "center";
      ctx.fillText(
        state.mode === "interactive" && state.meta && state.meta.phase === "idle"
          ? "Configure a target and start a benchmark run"
          : "Waiting for telemetry snapshots...",
        width / 2,
        height / 2
      );
      return;
    }

    let maxLeft = 1;
    let maxRight = 1;
    for (const row of data) {
      for (const s of spec.series) {
        const val = s.getValue(row);
        if (s.axis === "right") {
          if (val > maxRight) maxRight = val;
        } else if (val > maxLeft) {
          maxLeft = val;
        }
      }
    }
    maxLeft = maxLeft * 1.12 || 1;
    maxRight = Math.max(maxRight * 1.12, 1);

    const xMin = data[0].elapsed_secs;
    const xMax = Math.max(data[data.length - 1].elapsed_secs, xMin + 0.25);
    const xSpan = xMax - xMin || 1;

    const toX = (elapsed) => pad.left + ((elapsed - xMin) / xSpan) * plotW;
    const toYLeft = (v) => pad.top + plotH - (Math.min(v, maxLeft) / maxLeft) * plotH;
    const toYRight = (v) => pad.top + plotH - (Math.min(v, maxRight) / maxRight) * plotH;

    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 1;
    ctx.fillStyle = textMuted;
    ctx.font = "11px ui-monospace, SFMono-Regular, Menlo, monospace";

    const yTicks = 4;
    for (let i = 0; i <= yTicks; i += 1) {
      const frac = i / yTicks;
      const y = pad.top + plotH - frac * plotH;
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(pad.left + plotW, y);
      ctx.stroke();

      const leftLabelVal = maxLeft * frac;
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      ctx.fillText(spec.formatLeft(leftLabelVal), pad.left - 8, y);

      if (spec.rightAxis) {
        const rightLabelVal = maxRight * frac;
        ctx.textAlign = "left";
        ctx.fillText(spec.formatRight(rightLabelVal), pad.left + plotW + 8, y);
      }
    }

    const xTicks = Math.min(5, Math.max(2, Math.floor(plotW / 110)));
    ctx.textBaseline = "top";
    for (let i = 0; i <= xTicks; i += 1) {
      const frac = i / xTicks;
      const elapsed = xMin + xSpan * frac;
      const x = pad.left + plotW * frac;
      ctx.textAlign = i === 0 ? "left" : i === xTicks ? "right" : "center";
      ctx.fillText(`${elapsed.toFixed(1)}s`, x, pad.top + plotH + 8);
    }

    for (const s of spec.series) {
      const color = getCssToken(s.colorVar, s.fallbackColor);
      const getY = s.axis === "right" ? toYRight : toYLeft;

      ctx.save();
      ctx.strokeStyle = color;
      ctx.lineWidth = s.lineWidth || 2;
      if (s.dashed) {
        ctx.setLineDash([5, 4]);
      }

      ctx.beginPath();
      for (let i = 0; i < data.length; i += 1) {
        const x = data.length === 1 ? pad.left + plotW / 2 : toX(data[i].elapsed_secs);
        const y = getY(s.getValue(data[i]));
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();

      if (s.fillAlpha && data.length > 1) {
        ctx.setLineDash([]);
        ctx.lineTo(toX(data[data.length - 1].elapsed_secs), pad.top + plotH);
        ctx.lineTo(toX(data[0].elapsed_secs), pad.top + plotH);
        ctx.closePath();
        ctx.globalAlpha = s.fillAlpha;
        ctx.fillStyle = color;
        ctx.fill();
      }
      ctx.restore();
    }

    if (hoverIdx !== null && hoverIdx >= 0 && hoverIdx < data.length) {
      const row = data[hoverIdx];
      const hx = data.length === 1 ? pad.left + plotW / 2 : toX(row.elapsed_secs);

      ctx.save();
      ctx.strokeStyle = textMuted;
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(hx, pad.top);
      ctx.lineTo(hx, pad.top + plotH);
      ctx.stroke();
      ctx.restore();

      for (const s of spec.series) {
        const color = getCssToken(s.colorVar, s.fallbackColor);
        const getY = s.axis === "right" ? toYRight : toYLeft;
        const hy = getY(s.getValue(row));
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(hx, hy, 3.5, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }

  function buildChartSpecs() {
    const snaps = state.renderedSnapshots;
    const perTick = computePerTickSeries(snaps);

    return {
      throughput: {
        canvas: els.chartThroughput,
        tooltip: els.tooltipThroughput,
        data: snaps,
        rightAxis: true,
        formatLeft: (v) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : v.toFixed(0)),
        formatRight: (v) => Math.round(v).toString(),
        series: [
          {
            label: "Window RPS",
            colorVar: "--series-rps",
            fallbackColor: "#0ea5e9",
            axis: "left",
            fillAlpha: 0.12,
            getValue: (r) => r.window_rps,
            formatValue: (v) => `${v.toFixed(1)} req/s`,
          },
          {
            label: "Avg RPS",
            colorVar: "--series-avg",
            fallbackColor: "#6366f1",
            axis: "left",
            dashed: true,
            getValue: (r) => r.avg_rps,
            formatValue: (v) => `${v.toFixed(1)} req/s`,
          },
          {
            label: "Active VUs",
            colorVar: "--series-vus",
            fallbackColor: "#f59e0b",
            axis: "right",
            getValue: (r) => r.active_workers,
            formatValue: (v) => `${v} VUs`,
          },
        ],
      },
      latency: {
        canvas: els.chartLatency,
        tooltip: els.tooltipLatency,
        data: snaps,
        rightAxis: false,
        formatLeft: (v) => (v >= 100 ? v.toFixed(0) : v.toFixed(1)),
        series: [
          {
            label: "p50",
            colorVar: "--series-p50",
            fallbackColor: "#10b981",
            getValue: (r) => r.window_latency_ms.p50,
            formatValue: (v) => `${v.toFixed(2)} ms`,
          },
          {
            label: "p90",
            colorVar: "--series-p90",
            fallbackColor: "#38bdf8",
            getValue: (r) => r.window_latency_ms.p90,
            formatValue: (v) => `${v.toFixed(2)} ms`,
          },
          {
            label: "p95",
            colorVar: "--series-p95",
            fallbackColor: "#f59e0b",
            getValue: (r) => r.window_latency_ms.p95,
            formatValue: (v) => `${v.toFixed(2)} ms`,
          },
          {
            label: "p99",
            colorVar: "--series-p99",
            fallbackColor: "#f43f5e",
            getValue: (r) => r.window_latency_ms.p99,
            formatValue: (v) => `${v.toFixed(2)} ms`,
          },
          {
            label: "max",
            colorVar: "--series-max",
            fallbackColor: "#a855f7",
            dashed: true,
            getValue: (r) => r.window_latency_ms.max,
            formatValue: (v) => `${v.toFixed(2)} ms`,
          },
        ],
      },
      errors: {
        canvas: els.chartErrors,
        tooltip: els.tooltipErrors,
        data: perTick,
        rightAxis: false,
        formatLeft: (v) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : Math.round(v).toString()),
        series: [
          {
            label: "2xx",
            colorVar: "--series-2xx",
            fallbackColor: "#10b981",
            fillAlpha: 0.1,
            getValue: (r) => r.ok2xx,
            formatValue: (v) => `${formatNumber(v)} reqs`,
          },
          {
            label: "4xx",
            colorVar: "--series-4xx",
            fallbackColor: "#f59e0b",
            getValue: (r) => r.err4xx,
            formatValue: (v) => `${formatNumber(v)} reqs`,
          },
          {
            label: "5xx",
            colorVar: "--series-5xx",
            fallbackColor: "#ef4444",
            getValue: (r) => r.err5xx,
            formatValue: (v) => `${formatNumber(v)} reqs`,
          },
          {
            label: "Transport",
            colorVar: "--series-net",
            fallbackColor: "#ec4899",
            getValue: (r) => r.errNet,
            formatValue: (v) => `${formatNumber(v)} errs`,
          },
        ],
      },
    };
  }

  function drawAllCharts() {
    const specs = buildChartSpecs();
    for (const [key, spec] of Object.entries(specs)) {
      drawChartCanvas(spec.canvas, state.hoverIndex[key], spec);
    }
  }

  function bindChartHover(key, canvas, tooltipEl) {
    if (!canvas || !tooltipEl) return;

    canvas.addEventListener("mousemove", (event) => {
      const specs = buildChartSpecs();
      const spec = specs[key];
      const data = spec.data;
      if (!data || data.length === 0) {
        tooltipEl.hidden = true;
        return;
      }

      const rect = canvas.getBoundingClientRect();
      const padLeft = 56;
      const padRight = spec.rightAxis ? 48 : 14;
      const plotW = Math.max(20, rect.width - padLeft - padRight);
      const relX = Math.min(plotW, Math.max(0, event.clientX - rect.left - padLeft));
      const idx =
        data.length === 1
          ? 0
          : Math.min(data.length - 1, Math.round((relX / plotW) * (data.length - 1)));

      state.hoverIndex[key] = idx;
      drawChartCanvas(canvas, idx, spec);

      const row = data[idx];
      const lines = [`t = ${row.elapsed_secs.toFixed(2)}s`];
      for (const s of spec.series) {
        lines.push(`${s.label}: ${s.formatValue(s.getValue(row))}`);
      }
      tooltipEl.textContent = lines.join("\n");
      tooltipEl.style.whiteSpace = "pre";
      tooltipEl.hidden = false;

      const tipLeft = Math.min(rect.width - 170, Math.max(8, event.clientX - rect.left + 12));
      tooltipEl.style.left = `${tipLeft}px`;
      tooltipEl.style.top = "10px";
    });

    canvas.addEventListener("mouseleave", () => {
      state.hoverIndex[key] = null;
      tooltipEl.hidden = true;
      drawAllCharts();
    });
  }

  // Accessible validation & live announcement synchronized with :user-invalid
  const FormValidationController = (() => {
    const dirtyState = new WeakMap();
    const flagged = new WeakSet();
    const announced = new WeakSet();
    const supportsUserInvalid = typeof CSS !== "undefined" && CSS.supports("selector(:user-invalid)");
    let announceTimer = null;
    let announcedField = null;

    function linkError(el, shown) {
      const errorId = el.dataset.error;
      if (!errorId) return;
      const ids = (el.getAttribute("aria-describedby") || "")
        .split(/\s+/)
        .filter((id) => id && id !== errorId);
      if (shown) ids.push(errorId);
      if (ids.length) el.setAttribute("aria-describedby", ids.join(" "));
      else el.removeAttribute("aria-describedby");
    }

    function spokenText(node) {
      const copy = node.cloneNode(true);
      copy.querySelectorAll('[aria-hidden="true"]').forEach((n) => n.remove());
      copy
        .querySelectorAll('[role="img"][aria-label]')
        .forEach((n) => n.replaceWith(`${n.getAttribute("aria-label")}: `));
      return copy.textContent.replace(/\s+/g, " ").trim();
    }

    function announceError(el) {
      if (!els.formLiveAnnouncer) return;
      const errorEl = document.getElementById(el.dataset.error);
      if (!errorEl) return;
      const labelEl = el.labels && el.labels[0];
      const label = labelEl ? `${spokenText(labelEl)}: ` : "";
      clearTimeout(announceTimer);
      els.formLiveAnnouncer.textContent = "";
      announcedField = el;
      announceTimer = setTimeout(() => {
        els.formLiveAnnouncer.textContent = label + spokenText(errorEl);
      }, 500);
    }

    function isElInvalid(el) {
      if (supportsUserInvalid) {
        return el.matches(":user-invalid");
      }
      return el.classList.contains("user-invalid-fallback");
    }

    function syncAria(el, announce = false) {
      if (!el.matches || !el.matches("input, select, textarea")) return;
      const invalid = isElInvalid(el);
      if (invalid) {
        if (announce && !announced.has(el)) {
          announceError(el);
          announced.add(el);
        }
        el.setAttribute("aria-invalid", "true");
        linkError(el, true);
        flagged.add(el);
      } else if (flagged.has(el) || el.getAttribute("aria-invalid") !== "true") {
        el.setAttribute("aria-invalid", "false");
        linkError(el, false);
        flagged.delete(el);
        announced.delete(el);
        if (announcedField === el) {
          clearTimeout(announceTimer);
          if (els.formLiveAnnouncer) els.formLiveAnnouncer.textContent = "";
        }
      }
    }

    function updateFallbackState(input) {
      if (supportsUserInvalid || !input.validity) return;
      const isValid = input.validity.valid;
      input.classList.toggle("user-invalid-fallback", !isValid);
      input.classList.toggle("user-valid-fallback", isValid);
    }

    function setFieldError(el, message) {
      if (!el) return;
      const errId = el.dataset.error;
      const errEl = errId ? document.getElementById(errId) : null;
      if (message) {
        el.setCustomValidity(message);
        if (errEl) {
          errEl.replaceChildren();
          const icon = document.createElement("span");
          icon.setAttribute("role", "img");
          icon.setAttribute("aria-label", "Error");
          icon.textContent = "! ";
          errEl.append(icon, document.createTextNode(message));
        }
        if (!supportsUserInvalid) {
          dirtyState.set(el, { hasInteracted: true, hasBlurred: true });
          updateFallbackState(el);
        }
      } else {
        el.setCustomValidity("");
        if (!supportsUserInvalid) {
          updateFallbackState(el);
        }
      }
      syncAria(el, false);
    }

    function init(root) {
      if (!root) return;
      root.querySelectorAll("input, select, textarea").forEach((el) => {
        if (!el.hasAttribute("aria-invalid")) el.setAttribute("aria-invalid", "false");
      });

      root.addEventListener(
        "blur",
        (e) => {
          const el = e.target;
          if (!el.checkValidity) return;
          if (!supportsUserInvalid) {
            const st = dirtyState.get(el) || { hasInteracted: false, hasBlurred: false };
            st.hasBlurred = true;
            dirtyState.set(el, st);
            if (st.hasInteracted) updateFallbackState(el);
          }
          syncAria(el, true);
        },
        true
      );

      root.addEventListener("input", (e) => {
        const el = e.target;
        if (!el.checkValidity) return;
        el.setCustomValidity("");
        if (!supportsUserInvalid) {
          const st = dirtyState.get(el) || { hasInteracted: false, hasBlurred: false };
          st.hasInteracted = true;
          dirtyState.set(el, st);
          if (st.hasBlurred) updateFallbackState(el);
        }
        if (el.hasAttribute("aria-invalid")) syncAria(el, false);
      });

      root.addEventListener(
        "change",
        (e) => {
          const el = e.target;
          if (!el.checkValidity) return;
          if (!supportsUserInvalid) {
            const st = dirtyState.get(el) || { hasInteracted: false, hasBlurred: false };
            st.hasInteracted = true;
            dirtyState.set(el, st);
            if (st.hasBlurred) updateFallbackState(el);
          }
          syncAria(el, true);
        },
        true
      );

      root.addEventListener(
        "invalid",
        (e) => {
          const el = e.target;
          if (!supportsUserInvalid) {
            dirtyState.set(el, { hasInteracted: true, hasBlurred: true });
            updateFallbackState(el);
          }
          announced.add(el);
          syncAria(el, false);
        },
        true
      );

      root.addEventListener(
        "reset",
        (e) => {
          setTimeout(() => {
            for (const el of e.target.elements || []) {
              dirtyState.delete(el);
              el.classList.remove("user-invalid-fallback", "user-valid-fallback");
              if (el.setCustomValidity) el.setCustomValidity("");
              syncAria(el, false);
            }
          });
        },
        true
      );
    }

    return { init, setFieldError, syncAria };
  })();

  function parseDurationClientMs(raw, defaultUnit) {
    const trimmed = String(raw || "").trim();
    if (!trimmed) return null;
    let mult = defaultUnit === "ms" ? 1 : 1000;
    let numStr = trimmed;
    if (trimmed.endsWith("ms")) {
      mult = 1;
      numStr = trimmed.slice(0, -2).trim();
    } else if (trimmed.endsWith("s")) {
      mult = 1000;
      numStr = trimmed.slice(0, -1).trim();
    } else if (trimmed.endsWith("m")) {
      mult = 60_000;
      numStr = trimmed.slice(0, -1).trim();
    } else if (trimmed.endsWith("h")) {
      mult = 3_600_000;
      numStr = trimmed.slice(0, -1).trim();
    }
    if (!/^\d+$/.test(numStr)) return null;
    return Number(numStr) * mult;
  }

  function validateFormBoundsClient() {
    if (!els.configForm) return true;
    FormValidationController.setFieldError(els.cfgUrl, "");
    FormValidationController.setFieldError(els.cfgDuration, "");
    FormValidationController.setFieldError(els.cfgRampUp, "");
    FormValidationController.setFieldError(els.cfgTimeout, "");
    FormValidationController.setFieldError(els.cfgBodyFile, "");

    const rawUrl = els.cfgUrl.value.trim();
    if (!rawUrl) {
      FormValidationController.setFieldError(els.cfgUrl, "Enter a target URL.");
    } else {
      try {
        const parsed = new URL(rawUrl);
        if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
          FormValidationController.setFieldError(
            els.cfgUrl,
            "Only http and https URLs are supported."
          );
        }
      } catch (_) {
        FormValidationController.setFieldError(
          els.cfgUrl,
          "Enter a valid HTTP or HTTPS URL."
        );
      }
    }

    const durMs = parseDurationClientMs(els.cfgDuration.value, "s");
    if (durMs === null || durMs <= 0) {
      FormValidationController.setFieldError(
        els.cfgDuration,
        "Duration must be greater than 0ms (e.g. 30s)."
      );
    }

    const timeoutMs = parseDurationClientMs(els.cfgTimeout.value, "ms");
    if (timeoutMs === null || timeoutMs <= 0) {
      FormValidationController.setFieldError(
        els.cfgTimeout,
        "Timeout must be greater than 0ms (e.g. 5000ms)."
      );
    }

    const rampMs = parseDurationClientMs(els.cfgRampUp.value, "s");
    if (rampMs === null) {
      FormValidationController.setFieldError(
        els.cfgRampUp,
        "Invalid ramp-up duration (e.g. 0s or 10s)."
      );
    } else if (durMs !== null && rampMs > durMs) {
      FormValidationController.setFieldError(
        els.cfgRampUp,
        "Ramp-up cannot exceed total benchmark duration."
      );
    }

    if (state.bodyMode === "file" && !els.cfgBodyFile.value.trim()) {
      FormValidationController.setFieldError(
        els.cfgBodyFile,
        "Select or enter a payload file path."
      );
    }

    return els.configForm.checkValidity();
  }

  function updateOsWarningBanner() {
    if (!els.configOsWarning || !state.system || !state.system.limits) return;
    const limits = state.system.limits;
    const conc = Math.max(0, Number(els.cfgConcurrency?.value || 0));
    const estFds = conc * 2 + 128;
    if (limits.soft_limit < limits.recommended_min || estFds > limits.soft_limit) {
      els.configOsWarning.textContent = `OS open file limit is ${limits.soft_limit} (concurrency ${conc} needs ~${estFds} FDs). Lower concurrency or raise ulimit -n if socket errors occur.`;
      els.configOsWarning.hidden = false;
    } else {
      els.configOsWarning.hidden = true;
      els.configOsWarning.textContent = "";
    }
  }

  function markFormDirty(dirty = true) {
    state.formDirty = dirty;
    if (els.presetDirtyPill) {
      els.presetDirtyPill.hidden = !dirty;
    }
  }

  function setBodyMode(mode, markDirty = false) {
    const normalized = mode === "raw" || mode === "file" ? mode : "none";
    state.bodyMode = normalized;
    for (const tab of els.bodyTabs) {
      const selected = tab.dataset.bodyMode === normalized;
      tab.setAttribute("aria-selected", selected ? "true" : "false");
      tab.tabIndex = selected ? 0 : -1;
    }
    for (const [key, panel] of Object.entries(els.bodyPanels)) {
      if (!panel) continue;
      panel.hidden = key !== normalized;
    }
    if (markDirty) {
      markFormDirty(true);
    }
  }

  function initBodyTabs() {
    if (els.bodyTabs.length === 0) return;
    els.bodyTabs.forEach((tab, idx) => {
      tab.addEventListener("click", () => {
        setBodyMode(tab.dataset.bodyMode, true);
      });
      tab.addEventListener("keydown", (event) => {
        let nextIdx = null;
        if (event.key === "ArrowRight") {
          nextIdx = (idx + 1) % els.bodyTabs.length;
        } else if (event.key === "ArrowLeft") {
          nextIdx = (idx - 1 + els.bodyTabs.length) % els.bodyTabs.length;
        } else if (event.key === "Home") {
          nextIdx = 0;
        } else if (event.key === "End") {
          nextIdx = els.bodyTabs.length - 1;
        }
        if (nextIdx !== null) {
          event.preventDefault();
          const target = els.bodyTabs[nextIdx];
          target.focus();
          setBodyMode(target.dataset.bodyMode, true);
        }
      });
    });
  }

  function renderHeaderRows() {
    if (!els.headersList) return;
    els.headersList.replaceChildren();

    state.headers.forEach((entry, idx) => {
      const row = document.createElement("div");
      row.className = "header-row";

      const enabledCb = document.createElement("input");
      enabledCb.type = "checkbox";
      enabledCb.checked = entry.enabled !== false;
      enabledCb.setAttribute("aria-label", `Enable header ${idx + 1}`);
      enabledCb.addEventListener("change", () => {
        entry.enabled = enabledCb.checked;
        markFormDirty(true);
      });

      const nameInput = document.createElement("input");
      nameInput.type = "text";
      nameInput.className = "control-input mono";
      nameInput.placeholder = "Header-Name";
      nameInput.value = entry.name || "";
      nameInput.setAttribute("aria-label", `Header ${idx + 1} name`);

      const valInput = document.createElement("input");
      valInput.className = "control-input mono";
      valInput.placeholder = "Value";
      valInput.value = entry.value || "";
      valInput.setAttribute("aria-label", `Header ${idx + 1} value`);

      const syncSensitiveType = () => {
        const sensitive = isSensitiveHeaderName(nameInput.value);
        if (sensitive && !entry._revealed) {
          valInput.type = "password";
          revealBtn.hidden = false;
        } else if (sensitive) {
          valInput.type = "text";
          revealBtn.hidden = false;
        } else {
          valInput.type = "text";
          revealBtn.hidden = true;
        }
      };

      const revealBtn = document.createElement("button");
      revealBtn.type = "button";
      revealBtn.className = "btn btn-secondary btn-sm";
      revealBtn.textContent = entry._revealed ? "Hide" : "Show";
      revealBtn.addEventListener("click", () => {
        entry._revealed = !entry._revealed;
        revealBtn.textContent = entry._revealed ? "Hide" : "Show";
        syncSensitiveType();
      });

      nameInput.addEventListener("input", () => {
        entry.name = nameInput.value;
        syncSensitiveType();
        markFormDirty(true);
      });

      valInput.addEventListener("input", () => {
        entry.value = valInput.value;
        markFormDirty(true);
      });

      const removeBtn = document.createElement("button");
      removeBtn.type = "button";
      removeBtn.className = "btn btn-secondary btn-sm";
      removeBtn.textContent = "×";
      removeBtn.setAttribute("aria-label", `Remove header ${idx + 1}`);
      removeBtn.addEventListener("click", () => {
        state.headers.splice(idx, 1);
        renderHeaderRows();
        markFormDirty(true);
      });

      syncSensitiveType();
      row.append(enabledCb, nameInput, valInput, revealBtn, removeBtn);
      els.headersList.appendChild(row);
    });
  }

  function updateRawBodyByteCount() {
    if (!els.bodyRawBytes || !els.cfgBodyRaw) return;
    const bytes = new TextEncoder().encode(els.cfgBodyRaw.value || "").length;
    els.bodyRawBytes.textContent = formatBytes(bytes);
  }

  function readFormConfig() {
    const outJson = els.cfgOutputJson?.value.trim() || null;
    const outCsv = els.cfgOutputCsv?.value.trim() || null;
    const outPdf = els.cfgOutputPdf?.value.trim() || null;
    const bodyFile = els.cfgBodyFile?.value.trim() || null;
    const bodyRaw = els.cfgBodyRaw?.value || "";
    return {
      url: els.cfgUrl.value.trim(),
      method: els.cfgMethod.value,
      concurrency: Math.max(1, Number(els.cfgConcurrency.value || 700)),
      duration: els.cfgDuration.value.trim(),
      ramp_up: els.cfgRampUp.value.trim(),
      timeout: els.cfgTimeout.value.trim(),
      headers: state.headers
        .filter((h) => h.name.trim().length > 0 || h.value.trim().length > 0)
        .map((h) => ({
          name: h.name,
          value: h.value,
          enabled: h.enabled !== false,
        })),
      body_mode: state.bodyMode,
      body: state.bodyMode === "raw" ? bodyRaw : null,
      body_file: state.bodyMode === "file" ? bodyFile : null,
      output_json: outJson,
      output_csv: outCsv,
      output_pdf: outPdf,
    };
  }

  function populateFormConfig(cfg, presetId = null, dirty = false) {
    const c = cfg || {};
    els.cfgMethod.value = String(c.method || "GET").toUpperCase();
    els.cfgUrl.value = c.url || "";
    els.cfgConcurrency.value = String(c.concurrency || 700);
    els.cfgDuration.value = c.duration || "30s";
    els.cfgRampUp.value = c.ramp_up || "0s";
    els.cfgTimeout.value = c.timeout || "5000ms";
    if (els.cfgOutputJson) els.cfgOutputJson.value = c.output_json || "";
    if (els.cfgOutputCsv) els.cfgOutputCsv.value = c.output_csv || "";
    if (els.cfgOutputPdf) els.cfgOutputPdf.value = c.output_pdf || "";
    if (els.autoExportDetails && (c.output_json || c.output_csv || c.output_pdf)) {
      els.autoExportDetails.open = true;
    }
    els.cfgBodyRaw.value = c.body || "";
    els.cfgBodyFile.value = c.body_file || "";
    if (els.bodyFileMeta) els.bodyFileMeta.textContent = "";

    state.headers = Array.isArray(c.headers)
      ? c.headers.map((h) => ({
          name: h.name || "",
          value: h.value || "",
          enabled: h.enabled !== false,
          _revealed: false,
        }))
      : [];
    renderHeaderRows();
    setBodyMode(c.body_mode || "none", false);
    updateRawBodyByteCount();
    updateOsWarningBanner();

    state.selectedPresetId = presetId;
    if (els.presetDeleteBtn) {
      els.presetDeleteBtn.disabled = !presetId;
    }
    if (els.configFormError) {
      els.configFormError.hidden = true;
    }
    if (els.cliPreviewBox) {
      els.cliPreviewBox.hidden = true;
    }
    markFormDirty(dirty);
  }

  function renderPresetSelect() {
    if (!els.presetSelect) return;
    const prevValue = state.selectedPresetId ? `preset:${state.selectedPresetId}` : "";
    els.presetSelect.replaceChildren();

    const defaultOpt = document.createElement("option");
    defaultOpt.value = "";
    defaultOpt.textContent = "-- Custom / Unsaved Configuration --";
    els.presetSelect.appendChild(defaultOpt);

    if (state.presets.length > 0) {
      const grp = document.createElement("optgroup");
      grp.label = "Saved Presets";
      for (const preset of state.presets) {
        const opt = document.createElement("option");
        opt.value = `preset:${preset.id}`;
        opt.textContent = `${preset.name} (${preset.config.method} ${preset.config.concurrency} VUs)`;
        grp.appendChild(opt);
      }
      els.presetSelect.appendChild(grp);
    }

    if (state.recent.length > 0) {
      const grp = document.createElement("optgroup");
      grp.label = "Recent Runs";
      for (const rec of state.recent) {
        const opt = document.createElement("option");
        opt.value = `recent:${rec.id}`;
        const label = rec.preset_name ? `[${rec.preset_name}] ` : "";
        opt.textContent = `${label}${rec.config.method} ${rec.config.url} (${rec.config.concurrency} VUs)`;
        grp.appendChild(opt);
      }
      els.presetSelect.appendChild(grp);
    }

    els.presetSelect.value = prevValue;
    if (els.presetDeleteBtn) {
      els.presetDeleteBtn.disabled = !state.selectedPresetId;
    }
  }

  async function fetchSystemInfo() {
    const resp = await apiFetch("/api/system");
    if (!resp.ok) throw new Error(`Failed to fetch /api/system (${resp.status})`);
    state.system = await resp.json();

    if (els.chooseBodyFileBtn) {
      els.chooseBodyFileBtn.hidden = !state.system.native_dialogs;
    }

    if (state.system.initial_config) {
      populateFormConfig(state.system.initial_config, null, true);
    } else {
      updateOsWarningBanner();
    }
  }

  async function fetchPresets() {
    const resp = await apiFetch("/api/presets");
    if (!resp.ok) throw new Error(`Failed to fetch presets (${resp.status})`);
    const payload = await resp.json();
    state.presets = Array.isArray(payload.presets) ? payload.presets : [];
    state.recent = Array.isArray(payload.recent) ? payload.recent : [];
    renderPresetSelect();
  }

  async function fetchRunHistory() {
    const resp = await apiFetch("/api/runs");
    if (!resp.ok) return;
    state.runs = await resp.json();
    renderRunHistoryTable();
  }

  function renderRunHistoryTable() {
    if (!els.runHistoryBody || !els.runHistoryCount) return;
    els.runHistoryCount.textContent = `${state.runs.length} run${state.runs.length === 1 ? "" : "s"}`;
    els.runHistoryBody.replaceChildren();

    // Prune stale compare IDs
    const validIds = new Set(state.runs.map((r) => r.run_id));
    state.compareIds = state.compareIds.filter((id) => validIds.has(id));

    if (state.runs.length === 0) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = "--";
      const tdTarget = document.createElement("td");
      tdTarget.textContent = "No runs recorded yet";
      const tdVus = document.createElement("td");
      tdVus.textContent = "--";
      const tdRps = document.createElement("td");
      tdRps.textContent = "--";
      const tdLat = document.createElement("td");
      tdLat.textContent = "--";
      const tdErr = document.createElement("td");
      tdErr.textContent = "--";
      const tdAct = document.createElement("td");
      tdAct.textContent = "--";
      tr.append(th, tdTarget, tdVus, tdRps, tdLat, tdErr, tdAct);
      els.runHistoryBody.appendChild(tr);
      renderCompareStrip();
      return;
    }

    for (const run of state.runs) {
      const tr = document.createElement("tr");

      const thCmp = document.createElement("th");
      thCmp.scope = "row";
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = state.compareIds.includes(run.run_id);
      cb.setAttribute("aria-label", `Compare run ${run.url}`);
      cb.addEventListener("change", () => {
        if (cb.checked) {
          if (state.compareIds.length >= 2) {
            state.compareIds.shift();
          }
          state.compareIds.push(run.run_id);
        } else {
          state.compareIds = state.compareIds.filter((id) => id !== run.run_id);
        }
        renderRunHistoryTable();
      });
      thCmp.appendChild(cb);

      const tdTarget = document.createElement("td");
      tdTarget.className = "mono";
      const prefix = run.preset_name ? `[${run.preset_name}] ` : "";
      tdTarget.textContent = `${prefix}${run.method} ${run.url}`;

      const tdVus = document.createElement("td");
      tdVus.textContent = String(run.concurrency);

      const tdRps = document.createElement("td");
      tdRps.textContent =
        run.phase === "failed" ? "Failed" : `${run.requests_per_sec.toFixed(1)} req/s`;

      const tdLat = document.createElement("td");
      tdLat.textContent =
        run.phase === "failed"
          ? run.error_message || "--"
          : `${run.p50_ms.toFixed(1)} / ${run.p95_ms.toFixed(1)} / ${run.p99_ms.toFixed(1)} ms`;

      const tdErr = document.createElement("td");
      tdErr.textContent = `${run.error_rate_pct.toFixed(2)}%`;

      const tdActions = document.createElement("td");
      const wrap = document.createElement("div");
      wrap.className = "history-actions";

      const inspectBtn = document.createElement("button");
      inspectBtn.type = "button";
      inspectBtn.className = "btn btn-secondary btn-sm";
      inspectBtn.textContent = state.viewingRunId === run.run_id ? "Viewing" : "Inspect";
      inspectBtn.addEventListener("click", () => inspectHistoricalRun(run.run_id));

      const loadBtn = document.createElement("button");
      loadBtn.type = "button";
      loadBtn.className = "btn btn-secondary btn-sm";
      loadBtn.textContent = "Load";
      loadBtn.addEventListener("click", () => loadRunConfigIntoForm(run.run_id));

      wrap.append(inspectBtn, loadBtn);

      const canExport = run.phase !== "failed";
      for (const fmt of ["json", "csv", "pdf"]) {
        const expBtn = document.createElement("button");
        expBtn.type = "button";
        expBtn.className = "btn btn-secondary btn-sm";
        expBtn.textContent = fmt.toUpperCase();
        expBtn.disabled = !canExport;
        expBtn.title = canExport
          ? `Export ${fmt.toUpperCase()} report for this run`
          : "No benchmark summary available for failed run";
        expBtn.addEventListener("click", () => triggerReportExport(fmt, run.run_id));
        wrap.appendChild(expBtn);
      }

      tdActions.appendChild(wrap);

      tr.append(thCmp, tdTarget, tdVus, tdRps, tdLat, tdErr, tdActions);
      els.runHistoryBody.appendChild(tr);
    }

    renderCompareStrip();
  }

  function renderCompareStrip() {
    if (!els.runComparePanel) return;
    if (state.compareIds.length !== 2) {
      els.runComparePanel.hidden = true;
      els.runComparePanel.replaceChildren();
      return;
    }

    const runA = state.runs.find((r) => r.run_id === state.compareIds[0]);
    const runB = state.runs.find((r) => r.run_id === state.compareIds[1]);
    if (!runA || !runB) {
      els.runComparePanel.hidden = true;
      return;
    }
    // Compare chronologically: older run as Baseline (A), newer run as Candidate (B)
    const idxA = state.runs.indexOf(runA);
    const idxB = state.runs.indexOf(runB);
    const [a, b] = idxA > idxB ? [runA, runB] : [runB, runA];

    const metrics = [
      {
        label: "Avg RPS (B vs A)",
        val: `${b.requests_per_sec.toFixed(1)} vs ${a.requests_per_sec.toFixed(1)}`,
        delta: b.requests_per_sec - a.requests_per_sec,
        pct: a.requests_per_sec > 0 ? ((b.requests_per_sec - a.requests_per_sec) / a.requests_per_sec) * 100 : 0,
        higherIsBetter: true,
        unit: "req/s",
      },
      {
        label: "p50 Latency",
        val: `${b.p50_ms.toFixed(2)} vs ${a.p50_ms.toFixed(2)} ms`,
        delta: b.p50_ms - a.p50_ms,
        pct: a.p50_ms > 0 ? ((b.p50_ms - a.p50_ms) / a.p50_ms) * 100 : 0,
        higherIsBetter: false,
        unit: "ms",
      },
      {
        label: "p95 Latency",
        val: `${b.p95_ms.toFixed(2)} vs ${a.p95_ms.toFixed(2)} ms`,
        delta: b.p95_ms - a.p95_ms,
        pct: a.p95_ms > 0 ? ((b.p95_ms - a.p95_ms) / a.p95_ms) * 100 : 0,
        higherIsBetter: false,
        unit: "ms",
      },
      {
        label: "p99 Latency",
        val: `${b.p99_ms.toFixed(2)} vs ${a.p99_ms.toFixed(2)} ms`,
        delta: b.p99_ms - a.p99_ms,
        pct: a.p99_ms > 0 ? ((b.p99_ms - a.p99_ms) / a.p99_ms) * 100 : 0,
        higherIsBetter: false,
        unit: "ms",
      },
      {
        label: "Error Rate",
        val: `${b.error_rate_pct.toFixed(2)}% vs ${a.error_rate_pct.toFixed(2)}%`,
        delta: b.error_rate_pct - a.error_rate_pct,
        pct: null,
        higherIsBetter: false,
        unit: "pp",
      },
    ];

    els.runComparePanel.replaceChildren();
    for (const m of metrics) {
      const card = document.createElement("div");
      card.className = "compare-card";
      const k = document.createElement("div");
      k.className = "stat-k";
      k.textContent = m.label;
      const v = document.createElement("div");
      v.className = "stat-v";
      v.textContent = m.val;
      const d = document.createElement("div");
      const sign = m.delta >= 0 ? "+" : "";
      const pctStr = m.pct !== null ? ` (${sign}${m.pct.toFixed(1)}%)` : "";
      d.textContent = `${sign}${m.delta.toFixed(2)} ${m.unit}${pctStr}`;
      const improved = m.higherIsBetter ? m.delta > 0 : m.delta < 0;
      const regressed = m.higherIsBetter ? m.delta < 0 : m.delta > 0;
      d.className = `stat-delta ${improved ? "delta-better" : regressed ? "delta-worse" : ""}`;
      card.append(k, v, d);
      els.runComparePanel.appendChild(card);
    }
    els.runComparePanel.hidden = false;
  }

  async function inspectHistoricalRun(runId) {
    try {
      const resp = await apiFetch(`/api/runs/${encodeURIComponent(runId)}`);
      if (!resp.ok) throw new Error(`Unable to fetch run (${resp.status})`);
      const record = await resp.json();

      if (!state.viewingRunId) {
        state.liveMeta = state.meta;
        state.liveFinalStats = state.finalStats;
        state.liveSnapshots = state.snapshots.slice();
      }

      state.viewingRunId = record.run_id;
      if (els.returnLiveBtn) els.returnLiveBtn.hidden = false;
      showBanner(`Inspecting archived run (${record.meta.method} ${record.meta.url}).`);

      resetTelemetryDisplay();
      updateMetaDisplay(record.meta);
      if (Array.isArray(record.snapshots) && record.snapshots.length > 0) {
        mergeSnapshots(record.snapshots);
      }
      if (record.stats) {
        applyFinalStats(record.stats);
      } else if (record.error_message) {
        setPhaseBadge("failed");
        showBanner(`Archived run failed: ${record.error_message}`, true);
      }
      renderRunHistoryTable();
    } catch (err) {
      showBanner(err.message, true);
    }
  }

  function returnToLiveView() {
    state.viewingRunId = null;
    if (els.returnLiveBtn) els.returnLiveBtn.hidden = true;
    hideBanner();

    resetTelemetryDisplay();
    if (state.liveMeta) {
      updateMetaDisplay(state.liveMeta);
    }
    if (state.liveSnapshots.length > 0) {
      mergeSnapshots(state.liveSnapshots);
    }
    if (state.liveFinalStats) {
      applyFinalStats(state.liveFinalStats);
    }
    renderRunHistoryTable();
  }

  async function loadRunConfigIntoForm(runId) {
    try {
      const resp = await apiFetch(`/api/runs/${encodeURIComponent(runId)}`);
      if (!resp.ok) throw new Error(`Unable to fetch run (${resp.status})`);
      const record = await resp.json();
      populateFormConfig(record.config_redacted, null, true);
      if (els.presetSelect) els.presetSelect.value = "";
    } catch (err) {
      showBanner(err.message, true);
    }
  }

  async function startInteractiveRun() {
    if (els.configFormError) els.configFormError.hidden = true;
    if (!validateFormBoundsClient()) {
      const firstInvalid = els.configForm.querySelector(":invalid");
      if (firstInvalid) firstInvalid.focus();
      return;
    }

    const cfg = readFormConfig();
    const presetObj = state.presets.find((p) => p.id === state.selectedPresetId);
    const presetName = presetObj ? presetObj.name : null;

    if (els.startRunBtn) {
      els.startRunBtn.disabled = true;
      els.startRunBtn.textContent = "Starting...";
    }

    try {
      const resp = await apiFetch("/api/runs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ config: cfg, preset_name: presetName }),
      });

      const data = await resp.json().catch(() => ({}));
      if (!resp.ok) {
        if (data.validation && data.validation.field_errors) {
          applyServerFieldErrors(data.validation.field_errors);
        }
        if (els.configFormError) {
          els.configFormError.textContent = data.error || `Failed to start run (${resp.status})`;
          els.configFormError.hidden = false;
        }
        syncRunButtonsForPhase(state.meta?.phase || "idle");
        return;
      }

      if (state.viewingRunId) {
        state.viewingRunId = null;
        if (els.returnLiveBtn) els.returnLiveBtn.hidden = true;
      }
      state.activeRunId = data.run_id;
      await fetchPresets().catch(() => {});
    } catch (err) {
      if (els.configFormError) {
        els.configFormError.textContent = `Request failed: ${err.message}`;
        els.configFormError.hidden = false;
      }
      syncRunButtonsForPhase(state.meta?.phase || "idle");
    }
  }

  function applyServerFieldErrors(fieldErrors) {
    const fieldMap = {
      url: els.cfgUrl,
      concurrency: els.cfgConcurrency,
      duration: els.cfgDuration,
      ramp_up: els.cfgRampUp,
      timeout: els.cfgTimeout,
      body_file: els.cfgBodyFile,
    };
    for (const [key, msg] of Object.entries(fieldErrors || {})) {
      if (fieldMap[key]) {
        FormValidationController.setFieldError(fieldMap[key], msg);
      }
    }
  }

  async function stopActiveRun() {
    if (state.stopRequested) return;
    if (state.mode === "cli" && state.completed) return;
    if (state.mode === "interactive" && !isActivePhase(state.meta?.phase)) return;

    state.stopRequested = true;
    if (els.stopRunBtn) {
      els.stopRunBtn.disabled = true;
      els.stopRunBtn.textContent = "Stopping...";
    }
    setPhaseBadge("draining");
    try {
      await apiFetch("/api/stop", { method: "POST" });
    } catch (_) {
      state.stopRequested = false;
      syncRunButtonsForPhase(state.meta?.phase || "running");
    }
  }

  function initInteractiveControls() {
    if (els.cfgUrl) {
      els.cfgUrl.placeholder = EXAMPLE_URL_PLACEHOLDER;
    }

    FormValidationController.init(els.configForm);
    initBodyTabs();

    if (els.toggleSidebarBtn && els.workspaceLayout) {
      els.toggleSidebarBtn.addEventListener("click", () => {
        const collapsed = els.workspaceLayout.classList.toggle("sidebar-collapsed");
        els.toggleSidebarBtn.setAttribute("aria-expanded", collapsed ? "false" : "true");
        drawAllCharts();
      });
    }

    if (els.returnLiveBtn) {
      els.returnLiveBtn.addEventListener("click", returnToLiveView);
    }

    if (els.configForm) {
      els.configForm.addEventListener("submit", (event) => {
        event.preventDefault();
        startInteractiveRun();
      });

      els.configForm.addEventListener("input", () => {
        markFormDirty(true);
        updateOsWarningBanner();
      });
    }

    if (els.cfgBodyRaw) {
      els.cfgBodyRaw.addEventListener("input", updateRawBodyByteCount);
    }

    if (els.formatJsonBtn && els.cfgBodyRaw) {
      els.formatJsonBtn.addEventListener("click", () => {
        try {
          const parsed = JSON.parse(els.cfgBodyRaw.value);
          els.cfgBodyRaw.value = JSON.stringify(parsed, null, 2);
          updateRawBodyByteCount();
          markFormDirty(true);
        } catch (err) {
          showBanner(`Body is not valid JSON: ${err.message}`, true);
        }
      });
    }

    if (els.addHeaderBtn) {
      els.addHeaderBtn.addEventListener("click", () => {
        state.headers.push({ name: "", value: "", enabled: true, _revealed: false });
        renderHeaderRows();
        markFormDirty(true);
      });
    }

    if (els.chooseBodyFileBtn) {
      els.chooseBodyFileBtn.addEventListener("click", async () => {
        try {
          const resp = await apiFetch("/api/dialog/open-body-file", { method: "POST" });
          const data = await resp.json();
          if (resp.ok && !data.cancelled && data.path) {
            els.cfgBodyFile.value = data.path;
            FormValidationController.setFieldError(els.cfgBodyFile, "");
            if (els.bodyFileMeta) {
              els.bodyFileMeta.textContent = `Selected file (${formatBytes(data.size_bytes)})`;
            }
            markFormDirty(true);
          }
        } catch (err) {
          showBanner(`Unable to open file picker: ${err.message}`, true);
        }
      });
    }

    if (els.copyCliBtn) {
      els.copyCliBtn.addEventListener("click", async () => {
        const cfg = readFormConfig();
        try {
          const resp = await apiFetch("/api/runs/validate", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(cfg),
          });
          const data = await resp.json();
          const cmd = data.cli_command || "";
          if (els.cliPreviewBox) {
            els.cliPreviewBox.textContent = cmd;
            els.cliPreviewBox.hidden = false;
          }
          if (navigator.clipboard && navigator.clipboard.writeText) {
            await navigator.clipboard.writeText(cmd);
            const orig = els.copyCliBtn.textContent;
            els.copyCliBtn.textContent = "Copied!";
            setTimeout(() => {
              els.copyCliBtn.textContent = orig;
            }, 1400);
          }
        } catch (err) {
          showBanner(`Failed to build CLI command: ${err.message}`, true);
        }
      });
    }

    if (els.presetSelect) {
      els.presetSelect.addEventListener("change", () => {
        const val = els.presetSelect.value;
        if (!val) {
          state.selectedPresetId = null;
          if (els.presetDeleteBtn) els.presetDeleteBtn.disabled = true;
          return;
        }
        if (val.startsWith("preset:")) {
          const id = val.slice(7);
          const preset = state.presets.find((p) => p.id === id);
          if (preset) {
            populateFormConfig(preset.config, preset.id, false);
          }
        } else if (val.startsWith("recent:")) {
          const id = val.slice(7);
          const rec = state.recent.find((r) => r.id === id);
          if (rec) {
            populateFormConfig(rec.config, null, true);
          }
        }
      });
    }

    if (els.presetNewBtn) {
      els.presetNewBtn.addEventListener("click", () => {
        if (els.presetSelect) els.presetSelect.value = "";
        populateFormConfig(null, null, false);
      });
    }

    if (els.presetSaveBtn) {
      els.presetSaveBtn.addEventListener("click", async () => {
        if (!state.selectedPresetId) {
          openSavePresetPanel("create");
          return;
        }
        const existing = state.presets.find((p) => p.id === state.selectedPresetId);
        if (!existing) {
          openSavePresetPanel("create");
          return;
        }
        try {
          const resp = await apiFetch(`/api/presets/${encodeURIComponent(existing.id)}`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              name: existing.name,
              description: existing.description,
              config: readFormConfig(),
            }),
          });
          if (!resp.ok) throw new Error(`Save failed (${resp.status})`);
          await fetchPresets();
          markFormDirty(false);
        } catch (err) {
          showBanner(err.message, true);
        }
      });
    }

    if (els.presetSaveAsBtn) {
      els.presetSaveAsBtn.addEventListener("click", () => openSavePresetPanel("create"));
    }

    if (els.presetCancelBtn && els.presetSavePanel) {
      els.presetCancelBtn.addEventListener("click", () => {
        els.presetSavePanel.hidden = true;
      });
    }

    if (els.presetCommitBtn) {
      els.presetCommitBtn.addEventListener("click", commitSavePreset);
    }

    if (els.presetDeleteBtn && els.presetDeleteConfirm) {
      els.presetDeleteBtn.addEventListener("click", () => {
        if (!state.selectedPresetId) return;
        els.presetDeleteConfirm.hidden = false;
      });
    }

    if (els.presetDeleteCancelBtn && els.presetDeleteConfirm) {
      els.presetDeleteCancelBtn.addEventListener("click", () => {
        els.presetDeleteConfirm.hidden = true;
      });
    }

    if (els.presetDeleteConfirmBtn && els.presetDeleteConfirm) {
      els.presetDeleteConfirmBtn.addEventListener("click", async () => {
        if (!state.selectedPresetId) return;
        try {
          const resp = await apiFetch(
            `/api/presets/${encodeURIComponent(state.selectedPresetId)}`,
            { method: "DELETE" }
          );
          if (!resp.ok) throw new Error(`Delete failed (${resp.status})`);
          els.presetDeleteConfirm.hidden = true;
          state.selectedPresetId = null;
          await fetchPresets();
          markFormDirty(true);
        } catch (err) {
          showBanner(err.message, true);
        }
      });
    }

    // Keyboard shortcuts: Cmd+Enter (Start) and Cmd+. (Stop)
    document.addEventListener("keydown", (event) => {
      if (state.mode !== "interactive") return;
      if (event.metaKey && event.key === "Enter") {
        if (!isActivePhase(state.meta?.phase)) {
          event.preventDefault();
          startInteractiveRun();
        }
      } else if (event.metaKey && event.key === ".") {
        if (isActivePhase(state.meta?.phase)) {
          event.preventDefault();
          stopActiveRun();
        }
      }
    });
  }

  function openSavePresetPanel(mode) {
    if (!els.presetSavePanel) return;
    state.presetSaveMode = mode;
    const current = state.presets.find((p) => p.id === state.selectedPresetId);
    els.presetNameInput.value = current ? `${current.name} (Copy)` : "";
    els.presetDescInput.value = current?.description || "";
    FormValidationController.setFieldError(els.presetNameInput, "");
    els.presetSavePanel.hidden = false;
    els.presetNameInput.focus();
  }

  async function commitSavePreset() {
    const name = els.presetNameInput.value.trim();
    if (!name) {
      FormValidationController.setFieldError(els.presetNameInput, "Enter a name for this preset.");
      els.presetNameInput.focus();
      return;
    }

    try {
      const resp = await apiFetch("/api/presets", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name,
          description: els.presetDescInput.value.trim() || null,
          config: readFormConfig(),
        }),
      });
      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.error || `Failed to save preset (${resp.status})`);
      }
      const saved = await resp.json();
      els.presetSavePanel.hidden = true;
      state.selectedPresetId = saved.id;
      await fetchPresets();
      markFormDirty(false);
    } catch (err) {
      showBanner(err.message, true);
    }
  }

  async function fetchMeta() {
    const resp = await apiFetch("/api/meta");
    if (!resp.ok) {
      throw new Error(`Failed to fetch metadata (${resp.status})`);
    }
    const meta = await resp.json();
    state.mode = meta.mode === "interactive" ? "interactive" : "cli";
    state.liveMeta = meta;
    updateMetaDisplay(meta);
    return meta;
  }

  async function fetchAbout() {
    const resp = await apiFetch("/api/about");
    if (!resp.ok) {
      throw new Error(`Failed to fetch about payload (${resp.status})`);
    }
    const about = await resp.json();
    updateAboutDisplay(about);
  }

  async function fetchChangelog() {
    const resp = await apiFetch("/api/changelog");
    if (!resp.ok) {
      throw new Error(`Failed to fetch changelog (${resp.status})`);
    }
    const changelog = await resp.json();
    state.changelog = changelog;
    renderFullChangelog(changelog);
  }

  function renderReleaseNotesInto(container, entry, includeHeader) {
    container.replaceChildren();
    if (!entry) {
      const empty = document.createElement("p");
      empty.className = "panel-subtitle";
      empty.textContent = "No release notes available.";
      container.appendChild(empty);
      return;
    }

    if (includeHeader) {
      const headerRow = document.createElement("div");
      headerRow.className = "release-card-header";
      const verTag = document.createElement("span");
      verTag.className = "release-version-tag";
      verTag.textContent = `v${entry.version}`;
      headerRow.appendChild(verTag);
      if (entry.date) {
        const dateTag = document.createElement("span");
        dateTag.className = "release-date-tag";
        dateTag.textContent = entry.date;
        headerRow.appendChild(dateTag);
      }
      container.appendChild(headerRow);
    }

    const notes = Array.isArray(entry.notes) ? entry.notes : [];
    let currentList = null;

    for (const line of notes) {
      if (line.startsWith("### ")) {
        const cat = document.createElement("h4");
        cat.className = "release-category-title";
        cat.textContent = line.slice(4).trim();
        container.appendChild(cat);
        currentList = null;
      } else if (line.startsWith("- ")) {
        if (!currentList) {
          currentList = document.createElement("ul");
          currentList.className = "release-notes-list";
          container.appendChild(currentList);
        }
        const li = document.createElement("li");
        li.textContent = line.slice(2).trim();
        currentList.appendChild(li);
      } else {
        const p = document.createElement("p");
        p.className = "panel-subtitle";
        p.textContent = line;
        container.appendChild(p);
        currentList = null;
      }
    }
  }

  function updateAboutDisplay(about) {
    state.about = about;
    const versionText = `v${about.version || "0.2.0"}`;
    if (els.appVersionPill) {
      els.appVersionPill.textContent = versionText;
    }
    if (els.aboutVersionBadge) {
      els.aboutVersionBadge.textContent = versionText;
    }
    if (els.aboutDescription && about.description) {
      els.aboutDescription.textContent = about.description;
    }
    if (els.aboutLatestMeta) {
      if (about.latest_change) {
        const datePart = about.latest_change.date ? ` (${about.latest_change.date})` : "";
        els.aboutLatestMeta.textContent = `v${about.latest_change.version}${datePart}`;
      } else {
        els.aboutLatestMeta.textContent = versionText;
      }
    }
    if (els.aboutLatestCard) {
      renderReleaseNotesInto(els.aboutLatestCard, about.latest_change, false);
    }
  }

  function renderFullChangelog(changelog) {
    if (!els.aboutChangelogContainer) return;
    els.aboutChangelogContainer.replaceChildren();

    const releases = (changelog && Array.isArray(changelog.releases)) ? changelog.releases : [];
    if (releases.length === 0) {
      const card = document.createElement("div");
      card.className = "release-card";
      const pre = document.createElement("pre");
      pre.className = "mono";
      pre.textContent = (changelog && changelog.raw_markdown) ? changelog.raw_markdown : "No changelog entries found.";
      card.appendChild(pre);
      els.aboutChangelogContainer.appendChild(card);
      return;
    }

    for (const entry of releases) {
      const card = document.createElement("article");
      card.className = "release-card";
      renderReleaseNotesInto(card, entry, true);
      els.aboutChangelogContainer.appendChild(card);
    }
  }

  function initAboutModal() {
    const dialog = els.aboutDialog;
    if (!dialog) return;

    if (els.aboutBtn) {
      els.aboutBtn.addEventListener("click", () => {
        if (!state.about) {
          fetchAbout().catch(() => {});
        }
        dialog.showModal();
      });
    }

    dialog.addEventListener("close", () => {
      dialog.returnValue = "";
    });

    if (!("closedBy" in HTMLDialogElement.prototype)) {
      dialog.addEventListener("click", (event) => {
        if (event.target !== dialog) return;
        const rect = dialog.getBoundingClientRect();
        const isDialogContent =
          rect.top <= event.clientY &&
          event.clientY <= rect.top + rect.height &&
          rect.left <= event.clientX &&
          event.clientX <= rect.left + rect.width;
        if (isDialogContent) return;
        dialog.close();
      });
    }

    if (els.toggleChangelogBtn && els.aboutChangelogContainer) {
      els.toggleChangelogBtn.addEventListener("click", async () => {
        state.changelogExpanded = !state.changelogExpanded;
        els.toggleChangelogBtn.setAttribute(
          "aria-expanded",
          state.changelogExpanded ? "true" : "false"
        );
        els.toggleChangelogBtn.textContent = state.changelogExpanded
          ? "Hide Full Changelog"
          : "View Full Changelog";
        els.aboutChangelogContainer.hidden = !state.changelogExpanded;

        if (state.changelogExpanded && !state.changelog) {
          try {
            await fetchChangelog();
          } catch (err) {
            els.aboutChangelogContainer.replaceChildren();
            const errMsg = document.createElement("p");
            errMsg.className = "panel-subtitle";
            errMsg.textContent = `Unable to load changelog: ${err.message}`;
            els.aboutChangelogContainer.appendChild(errMsg);
          }
        }
      });
    }
  }

  async function fetchHistory() {
    const resp = await apiFetch("/api/history");
    if (!resp.ok) {
      throw new Error(`Failed to fetch history (${resp.status})`);
    }
    const history = await resp.json();
    mergeSnapshots(history);
  }

  function connectStream() {
    if (state.eventSource) {
      state.eventSource.close();
    }

    const es = new EventSource("/api/stream");
    state.eventSource = es;

    es.onopen = () => {
      if (!state.viewingRunId) {
        hideBanner();
      }
      if (state.hadDisconnect) {
        state.hadDisconnect = false;
        fetchHistory().catch(() => {});
      }
    };

    es.addEventListener("state", (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.meta) {
          state.liveMeta = payload.meta;
          if (!state.viewingRunId) {
            updateMetaDisplay(payload.meta);
          }
        }
        if (Array.isArray(payload.history) && payload.history.length > 0) {
          if (!state.viewingRunId) {
            mergeSnapshots(payload.history);
          } else {
            state.liveSnapshots = payload.history.slice();
          }
        }
      } catch (_) {}
    });

    es.addEventListener("run_started", (event) => {
      try {
        const payload = JSON.parse(event.data);
        state.activeRunId = payload.run_id || null;
        state.liveMeta = payload.meta || null;
        state.liveSnapshots = [];
        state.liveFinalStats = null;
        state.viewingRunId = null;
        if (els.returnLiveBtn) els.returnLiveBtn.hidden = true;
        hideBanner();
        resetTelemetryDisplay();
        if (payload.meta) {
          updateMetaDisplay(payload.meta);
        }
      } catch (_) {}
    });

    es.addEventListener("snapshot", (event) => {
      try {
        const snap = JSON.parse(event.data);
        appendSnapshot(snap);
      } catch (_) {}
    });

    es.addEventListener("complete", (event) => {
      try {
        const payload = JSON.parse(event.data);
        const stats = payload.stats ? payload.stats : payload;
        state.liveFinalStats = stats;
        if (payload.meta) {
          state.liveMeta = payload.meta;
        }
        if (!state.viewingRunId) {
          if (payload.meta) {
            updateMetaDisplay(payload.meta);
          }
          applyFinalStats(stats);
        }
        if (state.mode === "interactive") {
          fetchRunHistory().catch(() => {});
        }
      } catch (_) {}

      if (state.mode === "cli") {
        es.close();
        state.eventSource = null;
      }
    });

    es.addEventListener("run_failed", (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.meta) {
          state.liveMeta = payload.meta;
        }
        if (!state.viewingRunId) {
          if (payload.meta) {
            updateMetaDisplay(payload.meta);
          }
          setPhaseBadge("failed");
          syncRunButtonsForPhase("failed");
          showBanner(`Benchmark run failed: ${payload.error || "unknown error"}`, true);
        }
        if (state.mode === "interactive") {
          fetchRunHistory().catch(() => {});
        }
      } catch (_) {}
    });

    es.onerror = () => {
      if (state.mode === "cli" && state.completed) {
        es.close();
        return;
      }
      state.hadDisconnect = true;
      showBanner("Telemetry stream interrupted. Reconnecting and catching up from /api/history...");
    };
  }

  async function triggerReportExport(format, overrideRunId) {
    const fmt = format || "json";
    const runId =
      overrideRunId !== undefined
        ? overrideRunId
        : state.viewingRunId || state.activeRunId || state.meta?.run_id || null;

    if (state.mode === "interactive" && state.system && state.system.native_dialogs) {
      try {
        const resp = await apiFetch("/api/dialog/save-report", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ format: fmt, run_id: runId }),
        });
        const data = await resp.json().catch(() => ({}));
        if (!resp.ok) {
          showBanner(data.error || `Export failed (${resp.status})`, true);
        } else if (!data.cancelled && data.path) {
          const msg = `Saved ${fmt.toUpperCase()} report to ${data.path}`;
          showBanner(msg);
          if (els.formLiveAnnouncer) els.formLiveAnnouncer.textContent = msg;
        }
      } catch (err) {
        showBanner(`Export failed: ${err.message}`, true);
      }
      return;
    }

    const url = runId
      ? `/api/export/${fmt}?run_id=${encodeURIComponent(runId)}`
      : `/api/export/${fmt}`;
    const link = document.createElement("a");
    link.href = url;
    link.download = `rusty-loader-report.${fmt}`;
    link.style.display = "none";
    document.body.appendChild(link);
    link.click();
    link.remove();
    if (els.formLiveAnnouncer) {
      els.formLiveAnnouncer.textContent = `Downloading ${fmt.toUpperCase()} report.`;
    }
  }

  function initExportMenu() {
    const trigger = els.exportMenuTrigger;
    const menu = els.exportMenu;
    const group = els.exportMenuList;
    if (!trigger || !menu || !group) return;

    const supportsPopover = "popover" in HTMLElement.prototype;
    const supportsCommand = "command" in HTMLButtonElement.prototype;
    const supportsAnchor =
      typeof CSS !== "undefined" &&
      typeof CSS.supports === "function" &&
      CSS.supports("position-area", "block-end");

    if (supportsPopover && !supportsCommand) {
      trigger.setAttribute("popovertarget", "export-menu");
      trigger.setAttribute("popovertargetaction", "toggle");
    }

    let enterOnOpen = false;
    let pointerActivated = false;

    const getItems = () =>
      Array.from(group.querySelectorAll("button")).filter(
        (b) => !b.disabled && b.getAttribute("aria-disabled") !== "true"
      );

    const isOpen = () =>
      supportsPopover
        ? menu.matches(":popover-open")
        : menu.classList.contains(":popover-open");

    const positionFallback = () => {
      if (supportsAnchor) return;
      const rect = trigger.getBoundingClientRect();
      const menuRect = menu.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const margin = 8;
      let top = rect.bottom + 6;
      if (top + menuRect.height > vh - margin && rect.top - menuRect.height - 6 >= margin) {
        top = rect.top - menuRect.height - 6;
      }
      let left = rect.right - menuRect.width;
      if (left < margin) left = Math.max(margin, rect.left);
      if (left + menuRect.width > vw - margin) {
        left = Math.max(margin, vw - menuRect.width - margin);
      }
      menu.style.left = `${Math.round(left)}px`;
      menu.style.top = `${Math.round(top)}px`;
    };

    const setManualOpen = (open) => {
      if (supportsPopover) {
        if (open && !isOpen()) menu.showPopover();
        else if (!open && isOpen()) menu.hidePopover();
        return;
      }
      menu.classList.toggle(":popover-open", open);
      trigger.setAttribute("aria-expanded", open ? "true" : "false");
      if (open) {
        positionFallback();
        if (enterOnOpen) {
          enterOnOpen = false;
          const first = getItems()[0];
          if (first) first.focus({ preventScroll: true });
        }
      } else if (menu.contains(document.activeElement)) {
        trigger.focus({ preventScroll: true });
      }
    };

    if (!("focusGroup" in HTMLElement.prototype)) {
      group.setAttribute("role", "menu");
      group.querySelectorAll("button:not([role])").forEach((item) => {
        item.setAttribute("role", "menuitem");
      });
      group.addEventListener(
        "keydown",
        (event) => {
          if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
          const items = getItems();
          if (items.length === 0) return;
          const current = items.indexOf(document.activeElement);
          if (current < 0) return;
          const next =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? items.length - 1
                : (current + (event.key === "ArrowUp" ? -1 : 1) + items.length) %
                  items.length;
          event.preventDefault();
          event.stopImmediatePropagation();
          items[next].focus({ preventScroll: true });
        },
        true
      );
    }

    trigger.addEventListener("pointerdown", () => {
      pointerActivated = true;
      queueMicrotask(() => {
        pointerActivated = false;
      });
    });

    if (!supportsPopover) {
      trigger.addEventListener("click", () => {
        enterOnOpen = !pointerActivated;
        setManualOpen(!isOpen());
      });
      document.addEventListener("pointerdown", (event) => {
        if (isOpen() && !menu.contains(event.target) && !trigger.contains(event.target)) {
          setManualOpen(false);
        }
      });
    } else if (!supportsCommand) {
      trigger.addEventListener("click", () => {
        if (!isOpen() && !pointerActivated) {
          enterOnOpen = true;
        }
      });
    }

    trigger.addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        if (!isOpen()) {
          enterOnOpen = true;
          trigger.click();
        } else {
          const first = getItems()[0];
          if (first) first.focus({ preventScroll: true });
        }
      } else if ((event.key === "Enter" || event.key === " ") && !isOpen()) {
        enterOnOpen = true;
      }
    });

    if (supportsPopover) {
      menu.addEventListener("toggle", (event) => {
        const open = event.newState === "open";
        trigger.setAttribute("aria-expanded", open ? "true" : "false");
        if (open) {
          positionFallback();
          if (enterOnOpen) {
            enterOnOpen = false;
            const first = getItems()[0];
            if (first) first.focus({ preventScroll: true });
          } else {
            trigger.focus({ preventScroll: true });
          }
        } else {
          enterOnOpen = false;
          if (menu.contains(document.activeElement)) {
            trigger.focus({ preventScroll: true });
          }
        }
      });
    }

    menu.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        trigger.focus({ preventScroll: true });
        setManualOpen(false);
      } else if (event.key === "Tab") {
        setManualOpen(false);
      }
    });

    group.addEventListener("pointermove", (event) => {
      const item = event.target.closest("button");
      if (!item || !group.contains(item) || item.disabled) return;
      const hit = document.elementFromPoint(event.clientX, event.clientY);
      if (hit && item.contains(hit)) {
        item.focus({ preventScroll: true });
      }
    });

    group.addEventListener("click", (event) => {
      const item = event.target.closest("button[data-export-format]");
      if (!item || item.disabled) return;
      const format = item.dataset.exportFormat;
      setManualOpen(false);
      trigger.focus({ preventScroll: true });
      triggerReportExport(format);
    });

    document.addEventListener("focusin", (event) => {
      if (isOpen() && !menu.contains(event.target) && !trigger.contains(event.target)) {
        setManualOpen(false);
      }
    });
  }

  function initButtons() {
    if (els.pauseChartsBtn) {
      els.pauseChartsBtn.addEventListener("click", () => {
        state.paused = !state.paused;
        els.pauseChartsBtn.setAttribute("aria-pressed", state.paused ? "true" : "false");
        els.pauseChartsBtn.textContent = state.paused ? "Resume Charts" : "Pause Charts";
        if (!state.paused) {
          state.renderedSnapshots = state.snapshots.slice();
          drawAllCharts();
        }
      });
    }

    if (els.stopRunBtn) {
      els.stopRunBtn.addEventListener("click", stopActiveRun);
    }

    initExportMenu();
  }

  function initCharts() {
    bindChartHover("throughput", els.chartThroughput, els.tooltipThroughput);
    bindChartHover("latency", els.chartLatency, els.tooltipLatency);
    bindChartHover("errors", els.chartErrors, els.tooltipErrors);

    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(() => drawAllCharts());
      observer.observe(els.chartThroughput);
      observer.observe(els.chartLatency);
      observer.observe(els.chartErrors);
    } else {
      window.addEventListener("resize", drawAllCharts);
    }
  }

  async function boot() {
    initThemeControls();
    initButtons();
    initAboutModal();
    initCharts();

    fetchAbout().catch(() => {});

    try {
      const meta = await fetchMeta();
      if (meta.mode === "interactive") {
        if (els.toggleSidebarBtn) els.toggleSidebarBtn.hidden = false;
        if (els.configSidebar) els.configSidebar.hidden = false;
        if (els.runHistoryPanel) els.runHistoryPanel.hidden = false;
        initInteractiveControls();
        await Promise.all([
          fetchSystemInfo(),
          fetchPresets(),
          fetchRunHistory(),
          fetchHistory(),
        ]);
        syncRunButtonsForPhase(meta.phase || "idle");
      } else {
        await fetchHistory();
      }
    } catch (err) {
      showBanner(`Unable to load initial telemetry: ${err.message}`, true);
    }

    connectStream();
    drawAllCharts();
  }

  boot();
})();
