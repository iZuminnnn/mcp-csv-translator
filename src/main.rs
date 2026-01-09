use csv_translator_mcp::{AppConfig, AppState, CsvTranslatorServer};
use rmcp::{transport::stdio, ServiceExt};
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive("csv_translator_mcp=info".parse()?))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let args: Vec<String> = env::args().collect();

    let config = AppConfig::load_or_default(Some("config.toml"));
    tracing::info!("Loaded configuration: {:?}", config.server.name);

    let app_state = AppState::default();

    if args.len() > 1 && args[1] == "--http" {
        let port = args
            .get(2)
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(9527);

        let bind_addr = args
            .get(3)
            .map(|s| s.as_str())
            .unwrap_or("0.0.0.0");

        run_http_server(app_state, bind_addr, port).await?;
    } else {
        tracing::info!("Starting MCP Server on stdio");
        let server = CsvTranslatorServer::new(app_state);
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
    }

    tracing::info!("MCP Server shutting down");
    Ok(())
}

async fn run_http_server(
    app_state: AppState,
    bind_addr: &str,
    port: u16,
) -> anyhow::Result<()> {
    use axum::{
        extract::State,
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use csv_translator_mcp::csv_processor::analyze_csv;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    type SharedState = Arc<RwLock<AppState>>;

    async fn health() -> impl IntoResponse {
        Json(serde_json::json!({
            "status": "ok",
            "service": "csv-translator-mcp",
            "version": env!("CARGO_PKG_VERSION")
        }))
    }

    async fn info() -> impl IntoResponse {
        Json(serde_json::json!({
            "name": "csv-translator-mcp",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "MCP Server for translating large CSV files - HTTP Mode",
            "endpoints": {
                "GET /health": "Health check",
                "GET /info": "Server info",
                "POST /analyze": "Analyze CSV file",
                "POST /configure": "Configure translation",
                "POST /init": "Initialize translation session",
                "POST /chunk": "Get next chunk",
                "POST /submit": "Submit translation",
                "GET /progress/:session_id": "Get progress"
            }
        }))
    }

    async fn analyze_csv_handler(
        Json(payload): Json<serde_json::Value>,
    ) -> impl IntoResponse {
        let file_path = payload["file_path"].as_str().unwrap_or("");
        let sample_rows = payload["sample_rows"].as_u64().unwrap_or(10) as usize;

        match analyze_csv(file_path, sample_rows).await {
            Ok(metadata) => (StatusCode::OK, Json(serde_json::to_value(metadata).unwrap())),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e.to_string()})),
            ),
        }
    }

    async fn configure_handler(
        State(state): State<SharedState>,
        Json(payload): Json<serde_json::Value>,
    ) -> impl IntoResponse {
        let mut app_state = state.write().await;

        if let Some(source) = payload["source_lang"].as_str() {
            app_state.config.source_lang = source.to_string();
        }
        if let Some(target) = payload["target_lang"].as_str() {
            app_state.config.target_lang = target.to_string();
        }
        if let Some(columns) = payload["columns_to_translate"].as_array() {
            app_state.config.columns_to_translate = columns
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(chunk_size) = payload["chunk_size"].as_u64() {
            app_state.config.chunk_size = chunk_size as usize;
        }
        if let Some(overlap) = payload["overlap_size"].as_u64() {
            app_state.config.overlap_size = overlap as usize;
        }
        if let Some(domain) = payload["domain_context"].as_str() {
            app_state.config.domain_context = Some(domain.to_string());
        }

        Json(serde_json::json!({
            "status": "configured",
            "config": {
                "source_lang": app_state.config.source_lang,
                "target_lang": app_state.config.target_lang,
                "columns_to_translate": app_state.config.columns_to_translate,
                "chunk_size": app_state.config.chunk_size,
                "overlap_size": app_state.config.overlap_size
            }
        }))
    }

    let shared_state: SharedState = Arc::new(RwLock::new(app_state));

    let app = Router::new()
        .route("/health", get(health))
        .route("/info", get(info))
        .route("/analyze", post(analyze_csv_handler))
        .route("/configure", post(configure_handler))
        .with_state(shared_state);

    let addr = format!("{}:{}", bind_addr, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("🚀 HTTP Server listening on http://{}", addr);
    tracing::info!("");
    tracing::info!("Endpoints:");
    tracing::info!("  GET  /health    - Health check");
    tracing::info!("  GET  /info      - Server info");
    tracing::info!("  POST /analyze   - Analyze CSV file");
    tracing::info!("  POST /configure - Configure translation");
    tracing::info!("");
    tracing::info!("Example:");
    tracing::info!("  curl http://{}/health", addr);
    tracing::info!("  curl -X POST http://{}/analyze -H 'Content-Type: application/json' -d '{{\"file_path\": \"/path/to/file.csv\"}}'", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
