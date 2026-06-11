use std::net::SocketAddr;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use rns_mavlink::dashboard::{
    error_response, fetch_journalctl_logs, get_logs_page, get_page, ok_response, LogsResponse,
    DEFAULT_LOG_LINES, MAX_LOG_LINES, PLUGIN_TLS_CERT_FILE, PLUGIN_TLS_KEY_FILE,
};
use rns_mavlink::gc::{Config, CONFIG_PATH};

const SERVICE_NAME: &str = "rns-mavlink-gc.service";
const PAGE_TITLE: &str = "RNS-Mavlink GC Dashboard";

#[derive(Clone)]
struct AppState {
    destination_hash: String,
}

#[derive(Debug, Deserialize)]
struct ConfigUpdate {
    config: String,
}

#[derive(Debug, serde::Serialize)]
struct ConfigResponse {
    config: String,
}

#[derive(Debug, serde::Serialize)]
struct DestinationResponse {
    destination_hash: String,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    #[serde(default = "default_log_lines")]
    lines: u32,
}

fn default_log_lines() -> u32 {
    DEFAULT_LOG_LINES
}

async fn handler_get_page(State(state): State<AppState>) -> Html<String> {
    get_page(PAGE_TITLE, CONFIG_PATH, &state.destination_hash, SERVICE_NAME)
}

async fn handler_get_logs_page() -> Html<String> {
    get_logs_page(PAGE_TITLE, SERVICE_NAME)
}

async fn handler_get_config() -> impl IntoResponse {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(config_str) => (StatusCode::OK, Json(ConfigResponse { config: config_str })).into_response(),
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read config file: {err}")).into_response(),
    }
}

async fn handler_put_config(Json(update): Json<ConfigUpdate>) -> impl IntoResponse {
    // Validate config
    let _: Config = match toml::from_str(&update.config) {
        Ok(cfg) => cfg,
        Err(err) => {
            return error_response(StatusCode::BAD_REQUEST, format!("Invalid config: {err}")).into_response();
        }
    };

    // Write to file
    if let Err(err) = std::fs::write(CONFIG_PATH, &update.config) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write config file: {err}")).into_response();
    }

    ok_response("Config saved successfully. Restart the service to apply changes.").into_response()
}

async fn handler_restart() -> impl IntoResponse {
    log::info!("restarting service {SERVICE_NAME}");
    match std::process::Command::new("systemctl")
        .args(["restart", SERVICE_NAME])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                ok_response(format!("Service {SERVICE_NAME} restart initiated")).into_response()
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("systemctl failed: {stderr}")).into_response()
            }
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to run systemctl: {err}")).into_response(),
    }
}

async fn handler_get_logs(Query(query): Query<LogsQuery>) -> impl IntoResponse {
    let lines = query.lines.clamp(1, MAX_LOG_LINES);
    match fetch_journalctl_logs(SERVICE_NAME, lines) {
        Ok(text) => (StatusCode::OK, Json(LogsResponse { text })).into_response(),
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, err).into_response(),
    }
}

async fn handler_get_destination(State(state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(DestinationResponse { destination_hash: state.destination_hash }))
}

pub async fn start_server(bind_addr: SocketAddr, destination_hash: String) -> Result<(), String> {
    let state = AppState { destination_hash };
    let app = Router::new()
        .route("/", get(handler_get_page))
        .route("/logs", get(handler_get_logs_page))
        .route("/api/config", get(handler_get_config).put(handler_put_config))
        .route("/api/restart", post(handler_restart))
        .route("/api/logs", get(handler_get_logs))
        .route("/api/destination", get(handler_get_destination))
        .with_state(state);

    let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        PLUGIN_TLS_CERT_FILE,
        PLUGIN_TLS_KEY_FILE,
    )
    .await
    .unwrap_or_else(|err| {
        log::error!(
            "Failed to load TLS certificates {} and {}: {err}",
            PLUGIN_TLS_CERT_FILE, PLUGIN_TLS_KEY_FILE
        );
        std::process::exit(1);
    });

    log::info!("GC dashboard listening on https://{bind_addr}");

    axum_server::bind_rustls(bind_addr, tls_config)
        .serve(app.into_make_service())
        .await
        .map_err(|err| format!("HTTPS server error: {err}"))
}
