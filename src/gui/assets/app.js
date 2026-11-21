"use strict";

(() => {
  const STORAGE_KEY = "color-scheme";
  const colorSchemeMql = window.matchMedia("(prefers-color-scheme: dark)");

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
    meta: null,
    about: null,
    changelog: null,
    changelogExpanded: false,
    snapshots: [],
    renderedSnapshots: [],
    finalStats: null,
    paused: false,
    completed: false,
    stopRequested: false,
    hadDisconnect: false,
    eventSource: null,
    hoverIndex: {
      throughput: null,
      latency: null,
      errors: null,
    },
  };

  const els = {
    metaColorScheme: document.querySelector('meta[name="color-scheme"]'),
    appVersionPill: document.getElementById("app-version-pill"),
    phaseBadge: document.getElementById("phase-badge"),
    metaMethod: document.getElementById("meta-method"),
    metaUrl: document.getElementById("meta-url"),
    timerElapsed: document.getElementById("timer-elapsed"),
    timerDuration: document.getElementById("timer-duration"),
    runProgress: document.getElementById("run-progress"),
    runProgressFill: document.getElementById("run-progress-fill"),
    pauseChartsBtn: document.getElementById("pause-charts-btn"),
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
  };

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
      // Never delete localStorage on system preference changes; just re-render canvas colors.
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

  function showBanner(message) {
    if (!els.connectionBanner) return;
    els.connectionBanner.textContent = message;
    els.connectionBanner.hidden = false;
  }

  function hideBanner() {
    if (!els.connectionBanner) return;
    els.connectionBanner.hidden = true;
    els.connectionBanner.textContent = "";
  }

  function setPhaseBadge(phase) {
    if (!els.phaseBadge) return;
    els.phaseBadge.className = "badge";
    switch (phase) {
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

  function updateMetaDisplay(meta) {
    state.meta = meta;
    els.metaMethod.textContent = meta.method || "GET";
    els.metaUrl.textContent = meta.url || "--";
    els.metaUrl.title = meta.url || "";
    els.timerDuration.textContent = formatSeconds(meta.duration_secs);
    els.kpiRampUp.textContent = `Ramp-up: ${Number(meta.ramp_up_secs || 0).toFixed(1)}s`;
    els.kpiTimeout.textContent = `Timeout: ${meta.timeout_ms} ms`;

    if (meta.has_body) {
      els.metaBodyInfo.textContent = `Request body: ${formatBytes(meta.body_size_bytes)}`;
    } else {
      els.metaBodyInfo.textContent = "No request body";
    }

    if (!state.completed) {
      setPhaseBadge(meta.phase);
    }

    renderMetaTable(meta);

    if (state.snapshots.length === 0) {
      els.kpiActiveVus.textContent = `0 / ${meta.concurrency}`;
    }
  }

  function renderMetaTable(meta) {
    const tbody = els.metaHeadersBody;
    tbody.replaceChildren();

    const rows = [
      ["Target URL", meta.url],
      ["HTTP Method", meta.method],
      ["Target Concurrency", `${meta.concurrency} VUs`],
      ["Configured Duration", `${Number(meta.duration_secs).toFixed(2)} s`],
      ["Ramp-Up Duration", `${Number(meta.ramp_up_secs).toFixed(2)} s`],
      ["Request Timeout", `${meta.timeout_ms} ms`],
    ];

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
    hideBanner();

    if (els.stopRunBtn) {
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
      ctx.fillText("Waiting for telemetry snapshots...", width / 2, height / 2);
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

  async function fetchMeta() {
    const resp = await fetch("/api/meta", { cache: "no-store" });
    if (!resp.ok) {
      throw new Error(`Failed to fetch metadata (${resp.status})`);
    }
    const meta = await resp.json();
    updateMetaDisplay(meta);
  }

  async function fetchAbout() {
    const resp = await fetch("/api/about", { cache: "no-store" });
    if (!resp.ok) {
      throw new Error(`Failed to fetch about payload (${resp.status})`);
    }
    const about = await resp.json();
    updateAboutDisplay(about);
  }

  async function fetchChangelog() {
    const resp = await fetch("/api/changelog", { cache: "no-store" });
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
      verTag.className = "release-version-tag mono";
      verTag.textContent = `v${entry.version}`;
      headerRow.appendChild(verTag);

      if (entry.date) {
        const dateTag = document.createElement("span");
        dateTag.className = "release-date-tag mono";
        dateTag.textContent = entry.date;
        headerRow.appendChild(dateTag);
      }
      container.appendChild(headerRow);
    }

    const lines = String(entry.notes || "")
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);

    let currentList = null;
    for (const line of lines) {
      if (line.startsWith("### ")) {
        const cat = document.createElement("h4");
        cat.className = "release-category-title";
        cat.textContent = line.slice(4).trim();
        container.appendChild(cat);
        currentList = null;
      } else if (line.startsWith("- ") || line.startsWith("* ")) {
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
    const versionText = `v${about.version || "0.1.0"}`;
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

    // Fallback for browsers without native <dialog closedby="any"> support
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
    const resp = await fetch("/api/history", { cache: "no-store" });
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
      hideBanner();
      if (state.hadDisconnect) {
        state.hadDisconnect = false;
        fetchHistory().catch(() => {});
      }
    };

    es.addEventListener("snapshot", (event) => {
      try {
        const snap = JSON.parse(event.data);
        appendSnapshot(snap);
      } catch (_) {}
    });

    es.addEventListener("complete", (event) => {
      try {
        const stats = JSON.parse(event.data);
        applyFinalStats(stats);
      } catch (_) {}
      es.close();
      state.eventSource = null;
    });

    es.onerror = () => {
      if (state.completed) {
        es.close();
        return;
      }
      state.hadDisconnect = true;
      showBanner("Telemetry stream interrupted. Reconnecting and catching up from /api/history...");
    };
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
      els.stopRunBtn.addEventListener("click", async () => {
        if (state.stopRequested || state.completed) return;
        state.stopRequested = true;
        els.stopRunBtn.disabled = true;
        els.stopRunBtn.textContent = "Stopping...";
        setPhaseBadge("draining");
        try {
          await fetch("/api/stop", { method: "POST" });
        } catch (_) {
          if (!state.completed) {
            state.stopRequested = false;
            els.stopRunBtn.disabled = false;
            els.stopRunBtn.textContent = "Stop Run";
          }
        }
      });
    }
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
      await Promise.all([fetchMeta(), fetchHistory()]);
    } catch (err) {
      showBanner(`Unable to load initial telemetry: ${err.message}`);
    }

    connectStream();
    drawAllCharts();
  }

  boot();
})();
