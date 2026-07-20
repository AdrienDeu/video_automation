//! Serveur HTTP du projet (axum).
//!
//! Phase 0 : seule la route de santé `GET /health` existe. Les endpoints
//! d'upload audio et de validation arrivent a partir de la phase 1.

use axum::{routing::get, Router};
use video_core::config::Config;
use video_core::error::Error;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Charge un eventuel `.env` local (secrets) sans echouer s'il est absent.
    dotenvy::dotenv().ok();

    let config = Config::load()?;

    let app = Router::new().route("/health", get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind(&config.server_addr).await?;
    eprintln!("serveur en ecoute sur {}", config.server_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
