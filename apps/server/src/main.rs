//! Serveur HTTP du projet (axum).
//!
//! Phase 1 : ingestion audio (`POST /audio`), transcription STT via Voxtral
//! (API Mistral) et consultation d'un projet (`GET /projet/{id}`). Les
//! endpoints de validation arrivent en phase 2.

mod audio;
mod handlers;
mod store;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use video_core::config::{self, Config};
use video_core::error::Error;

/// Etat partage du serveur.
pub struct AppState {
    pub config: Config,
    /// Cle API Mistral capturee au demarrage ; `None` desactive la
    /// transcription (l'audio est alors simplement stocke).
    pub cle_api: Option<String>,
}

/// Construit le routeur de l'application (isole de `main` pour les tests).
fn construire_routeur(etat: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/audio", post(handlers::post_audio))
        .route("/projet/{id}", get(handlers::get_projet))
        .with_state(etat)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Charge un eventuel `.env` local (secrets) sans echouer s'il est absent.
    dotenvy::dotenv().ok();

    let config = Config::load()?;
    let adresse = config.server_addr.clone();

    let etat = Arc::new(AppState {
        cle_api: config::cle_api_mistral(),
        config,
    });
    if etat.cle_api.is_none() {
        eprintln!("attention : MISTRAL_API_KEY absente, la transcription est desactivee");
    }

    let app = construire_routeur(etat);
    let listener = tokio::net::TcpListener::bind(&adresse).await?;
    eprintln!("serveur en ecoute sur {adresse}");
    axum::serve(listener, app).await?;
    Ok(())
}
