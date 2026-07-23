//! Serveur HTTP du projet (axum).
//!
//! Phase 1 : ingestion audio (`POST /audio`), transcription STT via Voxtral
//! (API Mistral) et consultation d'un projet (`GET /projet/{id}`).
//! Phase 2 : generation du scenario par l'agent Scenariste apres le STT,
//! persistance SQLite (`pipeline::stockage`) et validation humaine du
//! scenario (`POST /valider`).
//! Phase 3 : choix des visuels licencies par l'agent Visuel.
//! Phase 4 : voix off et sous-titres `.srt` par l'agent Conteur.
//! Phase 5 : montage ffmpeg (preview + video finale 1080p) par l'agent
//! Monteur ; interface web embarquee (`GET /`) : envoi d'audio, liste des
//! projets (`GET /projets`), suivi et validation par etape, service des
//! fichiers du projet (`GET /projet/{id}/fichier/{nom}`).
//! Phase 6 : publication YouTube par l'agent Publieur une fois le montage
//! accepte (si les identifiants OAuth sont configures).
//! Phase 7 : affinage d'une etape avec propagation en aval (`POST /affiner`)
//! et suivi temps reel de l'etat d'un projet en SSE
//! (`GET /projet/{id}/events`).
//! Phase 8 : annulation a n'importe quelle etape (`POST /annuler`) et reprise
//! (`POST /reprendre`). Les etapes s'executent en tache de fond (module
//! `tache`), interruptibles via un `CancellationToken` par projet ; les POST
//! declencheurs (`/audio`, `/valider`, `/affiner`) repondent des l'etat
//! courant persiste, sans attendre la fin du traitement.

mod audio;
mod handlers;
mod store;
mod tache;
mod ui;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use pipeline::stockage::Stockage;
use video_core::config::{self, Config};
use video_core::error::Error;

/// Etat partage du serveur.
pub struct AppState {
    pub config: Config,
    /// Cle API Mistral capturee au demarrage ; `None` desactive la
    /// transcription et la generation de scenario (l'audio est alors
    /// simplement stocke).
    pub cle_api: Option<String>,
    /// Contexte de publication YouTube capture au demarrage ; `None`
    /// desactive la publication (le pipeline s'arrete a `MontagePret`).
    pub youtube: Option<agents::publieur::ContextePublication>,
    /// Persistance SQLite des projets.
    pub stockage: Stockage,
    /// Extracteur du Scenariste construit au demarrage ; `None` sans cle API
    /// Mistral. Trait object pour permettre l'injection d'un mock dans les
    /// tests (phase 7).
    pub scenariste: Option<Arc<dyn llm::scenariste::ExtracteurScenario>>,
    /// Notifications de changement d'etat des projets (SSE, phase 7) : chaque
    /// sauvegarde d'un projet y publie l'identifiant du projet modifie.
    pub evenements: tokio::sync::broadcast::Sender<String>,
    /// Tokens d'annulation des taches de pipeline en cours, par identifiant
    /// de projet (phase 8) : `POST /annuler` y puise le token a declencher,
    /// `tache::lancer_pipeline` l'y inscrit puis le retire en fin de tache.
    pub taches: std::sync::Mutex<
        std::collections::HashMap<String, video_core::annulation::CancellationToken>,
    >,
}

/// Construit le routeur de l'application (isole de `main` pour les tests).
fn construire_routeur(etat: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/", get(ui::get_index))
        .route("/app.js", get(ui::get_app_js))
        .route("/style.css", get(ui::get_style_css))
        .route("/audio", post(handlers::post_audio))
        .route("/projets", get(handlers::get_projets))
        .route("/projet/{id}", get(handlers::get_projet))
        .route("/projet/{id}/events", get(handlers::get_projet_events))
        .route("/projet/{id}/fichier/{nom}", get(handlers::get_fichier))
        .route("/valider", post(handlers::post_valider))
        .route("/visuel/remplacer", post(handlers::post_remplacer_visuel))
        .route("/affiner", post(handlers::post_affiner))
        .route("/annuler", post(handlers::post_annuler))
        .route("/reprendre", post(handlers::post_reprendre))
        .with_state(etat)
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Charge un eventuel `.env` local (secrets) sans echouer s'il est absent.
    dotenvy::dotenv().ok();

    let config = Config::load()?;
    let adresse = config.server_addr.clone();

    let stockage = Stockage::ouvrir(&config.data_dir).await?;
    let youtube = agents::publieur::ContextePublication::depuis_environnement(&config.data_dir);
    let cle_api = config::cle_api_mistral();
    // Extracteur du Scenariste construit une fois au demarrage ; absent sans
    // cle API (les etapes LLM sont alors desactivees, cf. l'avertissement).
    let scenariste = if cle_api.is_some() {
        llm::scenariste::construire_extracteur_scenario_depuis_config(&config.llm)
            .ok()
            .map(|extracteur| Arc::new(extracteur) as Arc<dyn llm::scenariste::ExtracteurScenario>)
    } else {
        None
    };
    let (evenements, _) = tokio::sync::broadcast::channel(64);
    let etat = Arc::new(AppState {
        cle_api,
        youtube,
        config,
        stockage,
        scenariste,
        evenements,
        taches: std::sync::Mutex::new(std::collections::HashMap::new()),
    });
    if etat.cle_api.is_none() {
        eprintln!("attention : MISTRAL_API_KEY absente, transcription et scenario sont desactives");
    }
    if etat.youtube.is_none() {
        eprintln!(
            "attention : identifiants YouTube absents (YOUTUBE_CLIENT_ID/SECRET + refresh token), \
             la publication est desactivee — lancez `cli youtube-auth` pour l'activer"
        );
    }

    let app = construire_routeur(etat);
    let listener = tokio::net::TcpListener::bind(&adresse).await?;
    eprintln!("serveur en ecoute sur {adresse}");
    axum::serve(listener, app).await?;
    Ok(())
}
