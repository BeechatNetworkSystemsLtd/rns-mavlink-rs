use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::Json;
use serde::Serialize;

pub const PLUGIN_TLS_CERT_FILE: &str = "plugin-tls.crt";
pub const PLUGIN_TLS_KEY_FILE: &str = "plugin-tls.key";

pub const DEFAULT_LOG_LINES: u32 = 100;
pub const MAX_LOG_LINES: u32 = 5000;

pub const STATS_LOOKBACK_SECS: u64 = 5;

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub text: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ServiceStats {
    pub fresh: bool,
    pub timestamp: Option<String>,
    pub link_in_bps: Option<u32>,
    pub link_out_bps: Option<u32>,
    pub packets_in_per_s: Option<u32>,
    pub packets_out_per_s: Option<u32>,
    pub total_packets_in: Option<u64>,
    pub total_packets_out: Option<u64>,
    pub total_bytes_in: Option<u64>,
    pub total_bytes_out: Option<u64>,
    pub extra: Option<u64>,
}

#[derive(Debug, Default)]
pub struct ParsedStatsLine {
    pub timestamp: Option<String>,
    pub link_in_bps: Option<u32>,
    pub link_out_bps: Option<u32>,
    pub packets_in_per_s: Option<u32>,
    pub packets_out_per_s: Option<u32>,
    pub total_packets_in: Option<u64>,
    pub total_packets_out: Option<u64>,
    pub total_bytes_in: Option<u64>,
    pub total_bytes_out: Option<u64>,
    pub total_ground_station_bytes: Option<u64>,
    pub total_serial_port_bytes: Option<u64>,
}

pub fn error_response(status: StatusCode, detail: impl Into<String>) -> impl IntoResponse {
    (status, Json(MessageResponse { detail: detail.into() }))
}

pub fn ok_response(detail: impl Into<String>) -> impl IntoResponse {
    (StatusCode::OK, Json(MessageResponse { detail: detail.into() }))
}

pub fn fetch_journalctl_logs(service_name: &str, lines: u32) -> Result<String, String> {
    let lines = lines.clamp(1, MAX_LOG_LINES);
    let output = std::process::Command::new("journalctl")
        .args([
            "-u",
            service_name,
            "-n",
            &lines.to_string(),
            "--no-pager",
        ])
        .output()
        .map_err(|err| format!("Failed to run journalctl: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "journalctl exited with status {}: {stderr}",
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn fetch_recent_stats_line(
    service_name: &str,
    lookback_secs: u64,
) -> Result<Option<String>, String> {
    let since_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("system time error: {e}"))?
        .as_secs()
        .saturating_sub(lookback_secs);
    let since_arg = format!("@{since_ts}");
    let output = std::process::Command::new("journalctl")
        .args([
            "-u",
            service_name,
            "--since",
            &since_arg,
            "--no-pager",
            "-o",
            "cat",
        ])
        .output()
        .map_err(|err| format!("Failed to run journalctl: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "journalctl exited with status {}: {stderr}",
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .rev()
        .find(|l| l.contains("link in B/s"))
        .map(|s| s.to_string()))
}

pub fn parse_stats_line(line: &str) -> ParsedStatsLine {
    ParsedStatsLine {
        timestamp: parse_log_timestamp(line),
        link_in_bps: extract_u32(line, "link in B/s"),
        link_out_bps: extract_u32(line, "link out B/s"),
        packets_in_per_s: extract_u32(line, "packets in / s"),
        packets_out_per_s: extract_u32(line, "packets out / s"),
        total_packets_in: extract_u64(line, "total packets in"),
        total_packets_out: extract_u64(line, "total packets out"),
        total_bytes_in: extract_u64(line, "total bytes in"),
        total_bytes_out: extract_u64(line, "total bytes out"),
        total_ground_station_bytes: extract_u64(line, "total ground station bytes"),
        total_serial_port_bytes: extract_u64(line, "total serial port bytes"),
    }
}

fn parse_log_timestamp(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let end = line[start + 1..].find(']')?;
    let inside = &line[start + 1..start + 1 + end];
    let ts_end = inside.find(' ').unwrap_or(inside.len());
    Some(inside[..ts_end].to_string())
}

fn extract_u64(line: &str, key: &str) -> Option<u64> {
    let idx = line.find(key)?;
    let after = &line[idx + key.len()..];
    let value_str = after.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
    let end = value_str
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value_str.len());
    if end == 0 {
        return None;
    }
    value_str[..end].parse().ok()
}

fn extract_u32(line: &str, key: &str) -> Option<u32> {
    extract_u64(line, key).and_then(|v| u32::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_GC: &str = "[2026-06-11T00:28:36Z INFO  rns_mavlink] link in B/s: 12, link out B/s: 34, packets in / s: 5, packets out / s: 6, total packets in: 700, total packets out: 800 total bytes in: 9000, total bytes out: 10000, total ground station bytes: 11000";
    const SAMPLE_FC: &str = "[2026-06-11T00:28:36Z INFO  rns_mavlink] link in B/s: 1, link out B/s: 2, packets in / s: 3, packets out / s: 4, total packets in: 5, total packets out: 6 total bytes in: 7, total bytes out: 8, total serial port bytes: 9";

    #[test]
    fn parses_gc_stats_line() {
        let p = parse_stats_line(SAMPLE_GC);
        assert_eq!(p.timestamp.as_deref(), Some("2026-06-11T00:28:36Z"));
        assert_eq!(p.link_in_bps, Some(12));
        assert_eq!(p.link_out_bps, Some(34));
        assert_eq!(p.packets_in_per_s, Some(5));
        assert_eq!(p.packets_out_per_s, Some(6));
        assert_eq!(p.total_packets_in, Some(700));
        assert_eq!(p.total_packets_out, Some(800));
        assert_eq!(p.total_bytes_in, Some(9000));
        assert_eq!(p.total_bytes_out, Some(10000));
        assert_eq!(p.total_ground_station_bytes, Some(11000));
        assert_eq!(p.total_serial_port_bytes, None);
    }

    #[test]
    fn parses_fc_stats_line() {
        let p = parse_stats_line(SAMPLE_FC);
        assert_eq!(p.link_in_bps, Some(1));
        assert_eq!(p.total_serial_port_bytes, Some(9));
        assert_eq!(p.total_ground_station_bytes, None);
    }
}

const BASE_STYLES: &str = r#"
:root {
  color-scheme: dark;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
body {
  margin: 0;
  min-height: 100vh;
  background: #0f172a;
  color: #e2e8f0;
}
h1 {
  margin: 0 0 0.5rem;
  font-size: 1.6rem;
}
.subtitle {
  color: #94a3b8;
  margin-bottom: 1.5rem;
}
.destination-hash {
  background: #1e293b;
  border: 1px solid #334155;
  border-radius: 0.5rem;
  padding: 0.75rem 1rem;
  margin-bottom: 1.5rem;
  font-family: "SF Mono", "Consolas", monospace;
  font-size: 0.9rem;
  display: flex;
  align-items: center;
  gap: 0.75rem;
}
.destination-hash .label {
  color: #94a3b8;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  white-space: nowrap;
}
.destination-hash .hash {
  color: #22c55e;
  word-break: break-all;
}
.card {
  background: #111827;
  border: 1px solid #1f2937;
  border-radius: 1rem;
  padding: 1.25rem;
  margin-bottom: 1rem;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  margin-bottom: 0.75rem;
}
.card-title {
  font-weight: 700;
  color: #f8fafc;
}
.card-title-sub {
  color: #64748b;
  font-weight: 400;
  font-size: 0.85rem;
  margin-left: 0.5rem;
  font-family: "SF Mono", "Consolas", monospace;
}
.card-actions {
  display: flex;
  gap: 0.25rem;
  align-items: center;
}
.icon-btn {
  background: transparent;
  border: none;
  color: #94a3b8;
  cursor: pointer;
  padding: 0.25rem 0.5rem;
  border-radius: 0.25rem;
  font-size: 1rem;
  text-decoration: none;
  display: inline-flex;
  align-items: center;
  line-height: 1;
}
.icon-btn:hover {
  color: #e2e8f0;
  background: #1f2937;
}
.toggle-chev {
  display: inline-block;
  transition: transform 0.15s ease;
}
.icon-btn[aria-expanded="true"] .toggle-chev {
  transform: rotate(180deg);
}
.editor-wrap {
  position: relative;
}
textarea {
  width: 100%;
  min-height: 20rem;
  background: #0b1220;
  border: 1px solid #334155;
  border-radius: 0.5rem;
  color: #e2e8f0;
  font-family: "SF Mono", "Consolas", monospace;
  font-size: 0.9rem;
  padding: 0.75rem;
  resize: vertical;
  box-sizing: border-box;
}
textarea:focus {
  outline: none;
  border-color: #3b82f6;
}
.btn-row {
  display: flex;
  gap: 0.75rem;
  margin-top: 1rem;
}
.btn {
  padding: 0.6rem 1.25rem;
  border-radius: 0.5rem;
  border: none;
  font-size: 0.9rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s ease;
}
.btn-primary {
  background: linear-gradient(180deg, #3b82f6, #2563eb);
  color: white;
}
.btn-primary:hover {
  background: linear-gradient(180deg, #60a5fa, #3b82f6);
}
.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.btn-danger {
  background: linear-gradient(180deg, #ef4444, #dc2626);
  color: white;
}
.btn-danger:hover {
  background: linear-gradient(180deg, #f87171, #ef4444);
}
.btn-secondary {
  background: #374151;
  color: #e2e8f0;
}
.btn-secondary:hover {
  background: #4b5563;
}
.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.status {
  padding: 0.75rem 1rem;
  border-radius: 0.5rem;
  margin-top: 1rem;
  font-size: 0.9rem;
  display: none;
}
.status.is-success {
  display: block;
  background: rgba(34, 197, 94, 0.15);
  border: 1px solid #22c55e;
  color: #86efac;
}
.status.is-error {
  display: block;
  background: rgba(239, 68, 68, 0.15);
  border: 1px solid #ef4444;
  color: #fca5a5;
}
.status.is-info {
  display: block;
  background: rgba(59, 130, 246, 0.15);
  border: 1px solid #3b82f6;
  color: #93c5fd;
}
.logs-controls {
  display: flex;
  gap: 0.75rem;
  align-items: center;
  margin-bottom: 0.75rem;
  flex-wrap: wrap;
}
.logs-lines-label {
  color: #94a3b8;
  font-size: 0.9rem;
}
.logs-lines-label select {
  background: #0b1220;
  color: #e2e8f0;
  border: 1px solid #334155;
  border-radius: 0.375rem;
  padding: 0.3rem 0.5rem;
  margin-left: 0.25rem;
  font-family: inherit;
}
.logs-output {
  background: #0b1220;
  border: 1px solid #334155;
  border-radius: 0.5rem;
  color: #e2e8f0;
  font-family: "SF Mono", "Consolas", monospace;
  font-size: 0.8rem;
  padding: 0.75rem;
  margin: 0;
  overflow: auto;
  white-space: pre;
}
"#;

const DASHBOARD_APP_STYLES: &str = r#"
.app {
  max-width: 48rem;
  margin: 0 auto;
  padding: 1.5rem;
}
.logs-card .logs-output {
  max-height: 24rem;
}
.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
  gap: 0.5rem;
}
.stat {
  background: #0b1220;
  border: 1px solid #1f2937;
  border-radius: 0.5rem;
  padding: 0.6rem 0.75rem;
}
.stat-label {
  color: #94a3b8;
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.2rem;
}
.stat-value {
  font-family: "SF Mono", "Consolas", monospace;
  font-size: 1.05rem;
  font-weight: 600;
  color: #e2e8f0;
}
.stat-value.is-stale {
  color: #475569;
}
"#;

const LOGS_PAGE_STYLES: &str = r#"
html, body {
  height: 100%;
}
.logs-app {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 1rem 1.5rem;
  box-sizing: border-box;
}
.logs-app .destination-hash {
  margin-bottom: 1rem;
}
.logs-app .logs-output {
  flex: 1;
  min-height: 0;
}
"#;

const DASHBOARD_SCRIPT: &str = r#"
(function() {
  const editor = document.getElementById('config-editor');
  const saveBtn = document.getElementById('save-btn');
  const reloadBtn = document.getElementById('reload-btn');
  const restartBtn = document.getElementById('restart-btn');
  const statusEl = document.getElementById('status');

  function showStatus(message, type) {
    statusEl.textContent = message;
    statusEl.className = 'status is-' + type;
  }

  function clearStatus() {
    statusEl.className = 'status';
  }

  async function loadConfig() {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) {
        const data = await resp.json().catch(() => ({}));
        throw new Error(data.detail || 'HTTP ' + resp.status);
      }
      const data = await resp.json();
      editor.value = data.config;
      clearStatus();
    } catch (err) {
      showStatus('Error loading config: ' + err.message, 'error');
    }
  }

  async function saveConfig() {
    saveBtn.disabled = true;
    try {
      const resp = await fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ config: editor.value })
      });
      const data = await resp.json().catch(() => ({}));
      if (!resp.ok) {
        throw new Error(data.detail || 'HTTP ' + resp.status);
      }
      showStatus(data.detail || 'Config saved successfully', 'success');
    } catch (err) {
      showStatus('Error saving config: ' + err.message, 'error');
    } finally {
      saveBtn.disabled = false;
    }
  }

  async function waitForService(maxAttempts, interval) {
    maxAttempts = maxAttempts || 30;
    interval = interval || 500;
    for (let i = 0; i < maxAttempts; i++) {
      await new Promise(r => setTimeout(r, interval));
      try {
        const resp = await fetch('/api/config', { method: 'GET' });
        if (resp.ok) {
          return true;
        }
      } catch (e) {
        // Service not yet available, continue polling
      }
    }
    return false;
  }

  async function restartService() {
    if (!confirm('Are you sure you want to restart the service?')) {
      return;
    }
    restartBtn.disabled = true;
    showStatus('Restarting service...', 'info');
    try {
      await fetch('/api/restart', { method: 'POST' }).catch(() => {});
      await new Promise(r => setTimeout(r, 1000));
      showStatus('Waiting for service to restart...', 'info');
      const isUp = await waitForService();
      if (isUp) {
        showStatus('Service restarted successfully', 'success');
        loadConfig();
      } else {
        showStatus('Service restart timed out - please refresh the page', 'error');
      }
    } catch (err) {
      showStatus('Error restarting service: ' + err.message, 'error');
    } finally {
      restartBtn.disabled = false;
    }
  }

  saveBtn.addEventListener('click', saveConfig);
  reloadBtn.addEventListener('click', loadConfig);
  restartBtn.addEventListener('click', restartService);

  function fmtNum(n) { return n.toLocaleString(); }
  function fmtBytes(n) {
    if (n < 1024) return fmtNum(n) + ' B';
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB';
    if (n < 1024 * 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + ' MB';
    return (n / 1024 / 1024 / 1024).toFixed(2) + ' GB';
  }
  function fmtBps(n) { return fmtBytes(n) + '/s'; }
  function fmtPps(n) { return fmtNum(n) + '/s'; }

  const statsConfig = [
    { id: 'stat-link-in', key: 'link_in_bps', format: fmtBps },
    { id: 'stat-link-out', key: 'link_out_bps', format: fmtBps },
    { id: 'stat-pps-in', key: 'packets_in_per_s', format: fmtPps },
    { id: 'stat-pps-out', key: 'packets_out_per_s', format: fmtPps },
    { id: 'stat-total-pkts-in', key: 'total_packets_in', format: fmtNum },
    { id: 'stat-total-pkts-out', key: 'total_packets_out', format: fmtNum },
    { id: 'stat-total-bytes-in', key: 'total_bytes_in', format: fmtBytes },
    { id: 'stat-total-bytes-out', key: 'total_bytes_out', format: fmtBytes },
    { id: 'stat-extra', key: 'extra', format: fmtBytes },
  ];
  const statsUpdated = document.getElementById('stats-updated');

  function markStatsStale(label) {
    for (const cfg of statsConfig) {
      const el = document.getElementById(cfg.id);
      if (el) {
        el.textContent = '-';
        el.classList.add('is-stale');
      }
    }
    if (statsUpdated) statsUpdated.textContent = label || 'no recent data';
  }

  async function pollStats() {
    try {
      const resp = await fetch('/api/stats');
      if (!resp.ok) {
        markStatsStale('stats unavailable');
        return;
      }
      const data = await resp.json();
      const fresh = data.fresh === true;
      if (!fresh) {
        markStatsStale('no recent data');
        return;
      }
      for (const cfg of statsConfig) {
        const el = document.getElementById(cfg.id);
        if (!el) continue;
        const val = data[cfg.key];
        if (val === null || val === undefined) {
          el.textContent = '-';
          el.classList.add('is-stale');
        } else {
          el.textContent = cfg.format(val);
          el.classList.remove('is-stale');
        }
      }
      if (statsUpdated) {
        if (data.timestamp) {
          const parts = data.timestamp.split('T');
          statsUpdated.textContent = parts[1] ? parts[1].replace('Z', '') : data.timestamp;
        } else {
          statsUpdated.textContent = 'updated';
        }
      }
    } catch (err) {
      markStatsStale('stats unavailable');
    }
  }

  pollStats();
  setInterval(pollStats, 5000);

  const logsToggle = document.getElementById('logs-toggle');
  const logsPanel = document.getElementById('logs-panel');
  const logsRefresh = document.getElementById('logs-refresh');
  const logsLines = document.getElementById('logs-lines');
  const logsOutput = document.getElementById('logs-output');
  let logsLoaded = false;

  async function loadLogs() {
    logsRefresh.disabled = true;
    logsOutput.textContent = 'Loading...';
    try {
      const resp = await fetch('/api/logs?lines=' + encodeURIComponent(logsLines.value));
      const data = await resp.json().catch(() => ({}));
      if (!resp.ok) {
        throw new Error(data.detail || 'HTTP ' + resp.status);
      }
      logsOutput.textContent = (data.text && data.text.length > 0) ? data.text : '(no log entries)';
      logsOutput.scrollTop = logsOutput.scrollHeight;
    } catch (err) {
      logsOutput.textContent = 'Error loading logs: ' + err.message;
    } finally {
      logsRefresh.disabled = false;
    }
  }

  logsToggle.addEventListener('click', () => {
    const expanded = logsToggle.getAttribute('aria-expanded') === 'true';
    if (expanded) {
      logsToggle.setAttribute('aria-expanded', 'false');
      logsPanel.hidden = true;
    } else {
      logsToggle.setAttribute('aria-expanded', 'true');
      logsPanel.hidden = false;
      if (!logsLoaded) {
        logsLoaded = true;
        loadLogs();
      }
    }
  });
  logsRefresh.addEventListener('click', loadLogs);

  loadConfig();
})();
"#;

const LOGS_PAGE_SCRIPT: &str = r#"
(function() {
  const logsRefresh = document.getElementById('logs-refresh');
  const logsLines = document.getElementById('logs-lines');
  const logsOutput = document.getElementById('logs-output');

  async function loadLogs() {
    logsRefresh.disabled = true;
    logsOutput.textContent = 'Loading...';
    try {
      const resp = await fetch('/api/logs?lines=' + encodeURIComponent(logsLines.value));
      const data = await resp.json().catch(() => ({}));
      if (!resp.ok) {
        throw new Error(data.detail || 'HTTP ' + resp.status);
      }
      logsOutput.textContent = (data.text && data.text.length > 0) ? data.text : '(no log entries)';
      logsOutput.scrollTop = logsOutput.scrollHeight;
    } catch (err) {
      logsOutput.textContent = 'Error loading logs: ' + err.message;
    } finally {
      logsRefresh.disabled = false;
    }
  }

  logsRefresh.addEventListener('click', loadLogs);
  loadLogs();
})();
"#;

pub fn get_page(
    title: &str,
    config_name: &str,
    destination_hash: &str,
    service_name: &str,
    extra_stat_label: Option<&str>,
) -> Html<String> {
    let extra_stat_html = match extra_stat_label {
        Some(label) => format!(
            r#"<div class="stat"><div class="stat-label">{label}</div><div class="stat-value" id="stat-extra">-</div></div>"#
        ),
        None => String::new(),
    };
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>{base_styles}{app_styles}</style>
</head>
<body>
  <main class="app">
    <h1>{title}</h1>
    <p class="subtitle">Edit the configuration file and save changes. Restart the service to apply.</p>

    <div class="destination-hash">
      <span class="label">Destination:</span>
      <span class="hash">{destination_hash}</span>
    </div>

    <div class="card stats-card">
      <div class="card-header">
        <div class="card-title">Statistics<span class="card-title-sub" id="stats-updated">no recent data</span></div>
      </div>
      <div class="stats-grid">
        <div class="stat"><div class="stat-label">Link in</div><div class="stat-value is-stale" id="stat-link-in">-</div></div>
        <div class="stat"><div class="stat-label">Link out</div><div class="stat-value is-stale" id="stat-link-out">-</div></div>
        <div class="stat"><div class="stat-label">Packets in</div><div class="stat-value is-stale" id="stat-pps-in">-</div></div>
        <div class="stat"><div class="stat-label">Packets out</div><div class="stat-value is-stale" id="stat-pps-out">-</div></div>
        <div class="stat"><div class="stat-label">Total packets in</div><div class="stat-value is-stale" id="stat-total-pkts-in">-</div></div>
        <div class="stat"><div class="stat-label">Total packets out</div><div class="stat-value is-stale" id="stat-total-pkts-out">-</div></div>
        <div class="stat"><div class="stat-label">Total bytes in</div><div class="stat-value is-stale" id="stat-total-bytes-in">-</div></div>
        <div class="stat"><div class="stat-label">Total bytes out</div><div class="stat-value is-stale" id="stat-total-bytes-out">-</div></div>
        {extra_stat_html}
      </div>
    </div>

    <div class="card">
      <div class="card-header">
        <div class="card-title">{config_name}</div>
      </div>
      <div class="editor-wrap">
        <textarea id="config-editor" spellcheck="false">Loading...</textarea>
      </div>
      <div class="btn-row">
        <button id="save-btn" class="btn btn-primary" type="button">Save Config</button>
        <button id="reload-btn" class="btn btn-secondary" type="button">Reload</button>
        <button id="restart-btn" class="btn btn-danger" type="button">Restart Service</button>
      </div>
      <div id="status" class="status"></div>
    </div>

    <div class="card logs-card">
      <div class="card-header">
        <div class="card-title">Service Logs<span class="card-title-sub">{service_name}</span></div>
        <div class="card-actions">
          <a class="icon-btn" href="/logs" target="_blank" rel="noopener" title="Open logs in new tab" aria-label="Open logs in new tab">&#x2197;</a>
          <button id="logs-toggle" class="icon-btn" type="button" aria-expanded="false" aria-controls="logs-panel" aria-label="Toggle log view"><span class="toggle-chev">&#x25BE;</span></button>
        </div>
      </div>
      <div id="logs-panel" hidden>
        <div class="logs-controls">
          <label class="logs-lines-label">Lines:
            <select id="logs-lines">
              <option value="100" selected>100</option>
              <option value="500">500</option>
              <option value="2000">2000</option>
            </select>
          </label>
          <button id="logs-refresh" class="btn btn-secondary" type="button">Refresh</button>
        </div>
        <pre id="logs-output" class="logs-output">Loading...</pre>
      </div>
    </div>
  </main>
  <script>{script}</script>
</body>
</html>
"#,
        title = title,
        config_name = config_name,
        destination_hash = destination_hash,
        service_name = service_name,
        extra_stat_html = extra_stat_html,
        base_styles = BASE_STYLES,
        app_styles = DASHBOARD_APP_STYLES,
        script = DASHBOARD_SCRIPT,
    ))
}

pub fn get_logs_page(title: &str, service_name: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title} &mdash; Logs</title>
  <style>{base_styles}{page_styles}</style>
</head>
<body>
  <main class="logs-app">
    <h1>{title} &mdash; Logs</h1>
    <div class="destination-hash">
      <span class="label">Service:</span>
      <span class="hash">{service_name}</span>
    </div>
    <div class="logs-controls">
      <label class="logs-lines-label">Lines:
        <select id="logs-lines">
          <option value="100" selected>100</option>
          <option value="500">500</option>
          <option value="2000">2000</option>
          <option value="5000">5000</option>
        </select>
      </label>
      <button id="logs-refresh" class="btn btn-secondary" type="button">Refresh</button>
    </div>
    <pre id="logs-output" class="logs-output">Loading...</pre>
  </main>
  <script>{script}</script>
</body>
</html>
"#,
        title = title,
        service_name = service_name,
        base_styles = BASE_STYLES,
        page_styles = LOGS_PAGE_STYLES,
        script = LOGS_PAGE_SCRIPT,
    ))
}
