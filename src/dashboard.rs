use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::Json;
use serde::Serialize;

pub const PLUGIN_TLS_CERT_FILE: &str = "plugin-tls.crt";
pub const PLUGIN_TLS_KEY_FILE: &str = "plugin-tls.key";

pub const DEFAULT_LOG_LINES: u32 = 100;
pub const MAX_LOG_LINES: u32 = 5000;

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub text: String,
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
) -> Html<String> {
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
