#![forbid(unsafe_code)]

use std::sync::Arc;

use anyhow::Context;
use cyberscope::{config::Config, state::AppState, web};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Arc::new(Config::from_env().context("配置无效")?);
    let state = Arc::new(
        AppState::new(config.clone())
            .await
            .context("初始化应用状态失败")?,
    );
    let listener = TcpListener::bind(config.web_bind_address)
        .await
        .with_context(|| format!("绑定 Web 地址 {} 失败", config.web_bind_address))?;

    tracing::info!(
        address = %config.web_bind_address,
        event = "web_server_started"
    );

    axum::serve(listener, web::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Web 服务运行失败")?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::new("off,cyberscope=info");
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!(event = "web_server_shutdown");
}
