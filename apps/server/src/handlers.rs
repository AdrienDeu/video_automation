//! Gestionnaires des routes HTTP du serveur.

use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;
use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::{DecisionValidation, Projet};

use crate::store;
use crate::{audio, AppState};

/// Taille maximale d'un fichier audio envoye (100 Mio).
const TAILLE_MAX_AUDIO: usize = 100 * 1024 * 1024;

/// Extensions audio acceptees a l'envoi (formats courants de dictee).
const EXTENSIONS_ACCEPTEES: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "ogg", "opus", "webm"];

/// Erreur HTTP : code de statut et message lisible par l'appelant.
type ErreurHttp = (StatusCode, String);

fn erreur_interne(contexte: &str, e: impl std::fmt::Display) -> ErreurHttp {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("{contexte} : {e}"),
    )
}

/// Fait avancer le pipeline tant que les portes sont ouvertes : transcription
/// → scenario (Scenariste), puis, si le scenario est accepte, → visuels
/// (Visuel), puis, si les visuels sont acceptes, → voix (Conteur). S'arrete
/// des qu'une transition en mode `validation` bloque.
async fn avancer_pipeline(etat: &AppState, projet: &mut Projet) -> Result<(), Error> {
    if projet.etat == EtatPipeline::Transcrit {
        let extracteur =
            llm::scenariste::construire_extracteur_scenario_depuis_config(&etat.config.llm)?;
        agents::realisateur::produire_scenario(projet, &extracteur, etat.config.pipeline.scenario)
            .await?;
    }
    if projet.etat == EtatPipeline::ScenarioGenere
        && projet.validation_scenario == Some(DecisionValidation::Accepte)
    {
        agents::visuel::produire_visuels_depuis_config(
            projet,
            &etat.config,
            etat.config.pipeline.visuels,
        )
        .await?;
    }
    if projet.etat == EtatPipeline::VisuelsPrets
        && projet.validation_visuels == Some(DecisionValidation::Accepte)
    {
        agents::conteur::produire_voix(projet, &etat.config, etat.config.pipeline.voix).await?;
    }
    Ok(())
}

/// `POST /audio` : recoit un fichier audio (multipart, champ `audio`, champ
/// optionnel `langue`), le stocke dans `data/<id>/` puis, si
/// `MISTRAL_API_KEY` est definie, enchaine transcription STT (Voxtral) et
/// generation du scenario (Scenariste).
///
/// Sans cle API, l'audio est simplement stocke et le projet reste en etat
/// `AudioRecu`. Avec cle : le projet atteint `Transcrit`, puis
/// `ScenarioGenere` ; en cas d'echec d'une de ces etapes, il est persiste en
/// etat `Erreur` et renvoye avec un statut `502`.
pub async fn post_audio(
    State(etat): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<Projet>), ErreurHttp> {
    let mut audio: Option<(String, Vec<u8>)> = None;
    let mut langue: Option<String> = None;

    while let Some(mut champ) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("multipart illisible : {e}"),
        )
    })? {
        match champ.name() {
            Some("audio") => {
                let nom = champ.file_name().unwrap_or("audio").to_string();
                // Lecture par morceaux pour pouvoir rejeter un fichier trop
                // volumineux sans le charger integralement en memoire.
                let mut octets = Vec::new();
                while let Some(morceau) = champ
                    .chunk()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("lecture du fichier : {e}")))?
                {
                    if octets.len() + morceau.len() > TAILLE_MAX_AUDIO {
                        return Err((
                            StatusCode::PAYLOAD_TOO_LARGE,
                            format!(
                                "fichier trop volumineux (maximum {} Mio)",
                                TAILLE_MAX_AUDIO / 1024 / 1024
                            ),
                        ));
                    }
                    octets.extend_from_slice(&morceau);
                }
                audio = Some((nom, octets));
            }
            Some("langue") => {
                langue =
                    Some(champ.text().await.map_err(|e| {
                        (StatusCode::BAD_REQUEST, format!("langue illisible : {e}"))
                    })?);
            }
            _ => {}
        }
    }

    let (nom, octets) = audio.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "champ `audio` manquant".to_string(),
        )
    })?;
    if octets.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "fichier audio vide".to_string()));
    }

    let extension = nom
        .rsplit('.')
        .next()
        .map(str::to_lowercase)
        .filter(|ext| EXTENSIONS_ACCEPTEES.contains(&ext.as_str()))
        .ok_or_else(|| {
            (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!(
                    "format non supporte (acceptes : {})",
                    EXTENSIONS_ACCEPTEES.join(", ")
                ),
            )
        })?;

    // Stockage : data/<id>/audio.<ext>
    let id = Uuid::new_v4().simple().to_string();
    let dossier = store::dossier_projet(&etat.config.data_dir, &id);
    let nom_audio = format!("audio.{extension}");
    let chemin_audio = dossier.join(&nom_audio);
    tokio::fs::create_dir_all(&dossier)
        .await
        .map_err(|e| erreur_interne("creation du dossier du projet", e))?;
    tokio::fs::write(&chemin_audio, &octets)
        .await
        .map_err(|e| erreur_interne("ecriture du fichier audio", e))?;

    // Controle de duree via ffprobe ; saute si ffprobe est indisponible (la
    // validation definitive a lieu cote API de transcription).
    if let Some(duree) = audio::duree_secondes(&chemin_audio).await {
        let max = etat.config.audio.duree_max_secondes;
        if duree > max as f64 {
            let _ = tokio::fs::remove_dir_all(&dossier).await;
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("audio trop long ({duree:.0} s, maximum {max} s)"),
            ));
        }
    }

    let mut projet = Projet::nouveau(id);
    projet.audio = Some(nom_audio);
    etat.stockage
        .sauvegarder(&projet)
        .await
        .map_err(|e| erreur_interne("persistance du projet", e))?;

    // Sans cle API, l'audio est stocke et les etapes LLM restent a faire.
    let Some(cle) = &etat.cle_api else {
        return Ok((StatusCode::CREATED, Json(projet)));
    };

    match tools::transcrire::transcrire_audio(&chemin_audio, langue.as_deref(), cle).await {
        Ok(transcription) => {
            projet.transcription = Some(transcription);
            projet.etat = EtatPipeline::Transcrit;
            // Enchaine scenario puis visuels tant que les portes le
            // permettent (modes auto/validation).
            let statut = match avancer_pipeline(&etat, &mut projet).await {
                Ok(()) => StatusCode::CREATED,
                Err(erreur) => {
                    projet.etat = EtatPipeline::Erreur(erreur.to_string());
                    StatusCode::BAD_GATEWAY
                }
            };
            etat.stockage
                .sauvegarder(&projet)
                .await
                .map_err(|e| erreur_interne("persistance du projet", e))?;
            Ok((statut, Json(projet)))
        }
        Err(erreur) => {
            projet.etat = EtatPipeline::Erreur(erreur.to_string());
            etat.stockage
                .sauvegarder(&projet)
                .await
                .map_err(|e| erreur_interne("persistance du projet", e))?;
            Ok((StatusCode::BAD_GATEWAY, Json(projet)))
        }
    }
}

/// `GET /projet/{id}` : renvoie l'etat d'un projet (transcription, scenario,
/// decision de validation... selon son avancement).
pub async fn get_projet(
    State(etat): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Projet>, ErreurHttp> {
    if !store::id_valide(&id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "identifiant de projet invalide".to_string(),
        ));
    }
    match etat.stockage.charger(&id).await {
        Ok(Some(projet)) => Ok(Json(projet)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("projet inconnu : {id}"))),
        Err(e) => Err(erreur_interne("lecture du projet", e)),
    }
}

/// Corps de `POST /valider`.
#[derive(Debug, Deserialize)]
pub struct RequeteValidation {
    /// Identifiant du projet a trancher.
    pub id: String,
    /// Decision prise sur l'etape (`accepte` ou `rejete`).
    pub decision: DecisionValidation,
    /// Etape concernee (`scenario` par defaut, `visuels` ou `voix`).
    pub etape: Option<pipeline::validation::EtapeValidation>,
}

/// `POST /valider` : enregistre la decision humaine sur une etape en mode
/// `validation` (scenario par defaut, visuels ou voix via `etape`).
///
/// Apres une acceptation, le pipeline enchaine avec l'etape suivante si une
/// cle API est disponible (visuels apres scenario, voix apres visuels).
///
/// Renvoie `409` si le projet n'attend pas de decision (mauvais etat ou etape
/// deja tranchee), `404` si le projet est inconnu.
pub async fn post_valider(
    State(etat): State<Arc<AppState>>,
    Json(requete): Json<RequeteValidation>,
) -> Result<(StatusCode, Json<Projet>), ErreurHttp> {
    if !store::id_valide(&requete.id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "identifiant de projet invalide".to_string(),
        ));
    }
    let mut projet = match etat.stockage.charger(&requete.id).await {
        Ok(Some(projet)) => projet,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("projet inconnu : {}", requete.id),
            ))
        }
        Err(e) => return Err(erreur_interne("lecture du projet", e)),
    };

    let etape = requete
        .etape
        .unwrap_or(pipeline::validation::EtapeValidation::Scenario);
    pipeline::validation::appliquer_decision(&mut projet, etape, requete.decision)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    // Etape acceptee : on enchaine avec la suite du pipeline si possible
    // (avancer_pipeline ne franchit que les portes ouvertes).
    let statut = if requete.decision == DecisionValidation::Accepte && etat.cle_api.is_some() {
        match avancer_pipeline(&etat, &mut projet).await {
            Ok(()) => StatusCode::OK,
            Err(erreur) => {
                projet.etat = EtatPipeline::Erreur(erreur.to_string());
                StatusCode::BAD_GATEWAY
            }
        }
    } else {
        StatusCode::OK
    };

    etat.stockage
        .sauvegarder(&projet)
        .await
        .map_err(|e| erreur_interne("persistance du projet", e))?;
    Ok((statut, Json(projet)))
}

/// Corps de `POST /visuel/remplacer`.
#[derive(Debug, Deserialize)]
pub struct RequeteRemplacement {
    /// Identifiant du projet.
    pub id: String,
    /// Index de la scene dont l'image doit etre remplacee (0-based).
    pub scene: usize,
    /// Nouvelle requete de recherche d'image (le « prompt » de remplacement).
    pub requete: String,
}

/// `POST /visuel/remplacer` : remplace l'image d'une scene par une nouvelle
/// recherche (mode validation : remplacement par prompt).
///
/// Apres remplacement, la validation des visuels est a refaire. Renvoie `409`
/// si le projet n'est pas en etat `VisuelsPrets` ou si la scene n'a pas
/// d'image, `404` si le projet est inconnu, `502` si la recherche echoue.
pub async fn post_remplacer_visuel(
    State(etat): State<Arc<AppState>>,
    Json(requete): Json<RequeteRemplacement>,
) -> Result<Json<Projet>, ErreurHttp> {
    if !store::id_valide(&requete.id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "identifiant de projet invalide".to_string(),
        ));
    }
    let mut projet = match etat.stockage.charger(&requete.id).await {
        Ok(Some(projet)) => projet,
        Ok(None) => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("projet inconnu : {}", requete.id),
            ))
        }
        Err(e) => return Err(erreur_interne("lecture du projet", e)),
    };

    agents::visuel::remplacer_image(&mut projet, &etat.config, requete.scene, &requete.requete)
        .await
        .map_err(|e| match e {
            Error::Pipeline(_) => (StatusCode::CONFLICT, e.to_string()),
            _ => (StatusCode::BAD_GATEWAY, e.to_string()),
        })?;

    etat.stockage
        .sauvegarder(&projet)
        .await
        .map_err(|e| erreur_interne("persistance du projet", e))?;
    Ok(Json(projet))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use http_body_util::BodyExt;
    use pipeline::stockage::Stockage;
    use tower::ServiceExt;
    use video_core::asset::{Asset, SourceImage};
    use video_core::config::{
        AudioConfig, Config, LlmConfig, PipelineConfig, Provider, VoixConfig,
    };
    use video_core::scenario::{Scenario, Scene};

    use crate::construire_routeur;

    /// Construit l'application avec un dossier de donnees temporaire et sans
    /// cle API (transcription et scenario sont alors desactives).
    async fn app_de_test(data_dir: std::path::PathBuf) -> Router {
        let stockage = Stockage::ouvrir(&data_dir)
            .await
            .expect("ouverture de la base de test");
        let config = Config {
            data_dir,
            server_addr: "127.0.0.1:0".to_string(),
            llm: LlmConfig {
                provider: Provider::Mistral,
                model: "mistral-large-latest".to_string(),
                ollama_url: None,
            },
            audio: AudioConfig::default(),
            pipeline: PipelineConfig::default(),
            voix: VoixConfig::default(),
        };
        construire_routeur(Arc::new(AppState {
            config,
            cle_api: None,
            stockage,
        }))
    }

    /// Genere un WAV valide (silence PCM 16 bits mono, 8 kHz) pour que le
    /// controle ffprobe reussisse aussi lorsque ffprobe est installe.
    fn wav_silence(duree_ms: u32) -> Vec<u8> {
        let taille_donnees = duree_ms * 16; // 16 octets/ms a 8 kHz, 16 bits
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + taille_donnees).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&taille_donnees.to_le_bytes());
        wav.resize(wav.len() + taille_donnees as usize, 0);
        wav
    }

    /// Construit une requete multipart contenant un seul champ fichier.
    fn requete_audio(nom_fichier: &str, contenu: &[u8]) -> Request<Body> {
        let boundary = "FRONTIERETEST";
        let mut corps = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"{nom_fichier}\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .into_bytes();
        corps.extend_from_slice(contenu);
        corps.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        Request::post("/audio")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(corps))
            .expect("construction de la requete")
    }

    /// Construit une requete `POST /valider`.
    fn requete_validation(id: &str, decision: &str) -> Request<Body> {
        Request::post("/valider")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{ "id": "{id}", "decision": "{decision}" }}"#
            )))
            .expect("construction de la requete")
    }

    /// Construit une requete `POST /valider` pour une etape donnee.
    fn requete_validation_etape(id: &str, decision: &str, etape: &str) -> Request<Body> {
        Request::post("/valider")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{ "id": "{id}", "decision": "{decision}", "etape": "{etape}" }}"#
            )))
            .expect("construction de la requete")
    }

    /// Construit une requete `POST /visuel/remplacer`.
    fn requete_remplacement(id: &str, scene: usize, requete: &str) -> Request<Body> {
        Request::post("/visuel/remplacer")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{ "id": "{id}", "scene": {scene}, "requete": "{requete}" }}"#
            )))
            .expect("construction de la requete")
    }

    /// Lit un corps de reponse JSON en `Projet`.
    async fn projet_depuis(reponse: axum::response::Response) -> Projet {
        let octets = reponse
            .into_body()
            .collect()
            .await
            .expect("lecture du corps")
            .to_bytes();
        serde_json::from_slice(&octets).expect("corps JSON valide")
    }

    /// Cree en base un projet en etat `ScenarioGenere`, pret a etre valide.
    async fn semer_projet_scenario(data_dir: &std::path::Path) -> Projet {
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        let mut projet = Projet::nouveau("projetscenario");
        projet.etat = EtatPipeline::ScenarioGenere;
        projet.scenario = Some(Scenario {
            titre: "Sujet dicte".to_string(),
            public: "tout public".to_string(),
            style_images: "photos documentaires".to_string(),
            scenes: vec![Scene {
                narration: "Voici le sujet.".to_string(),
                dialogues: vec![],
                description_visuelle: "Une image d'illustration".to_string(),
                duree_cible: 8.0,
            }],
        });
        stockage.sauvegarder(&projet).await.expect("persistance");
        projet
    }

    /// Cree en base un projet en etat `VisuelsPrets`, pret a etre valide.
    async fn semer_projet_visuels(data_dir: &std::path::Path) -> Projet {
        let mut projet = semer_projet_scenario(data_dir).await;
        projet.etat = EtatPipeline::VisuelsPrets;
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.visuels = vec![Asset {
            scene: 0,
            fichier: "scene-0.jpg".to_string(),
            source: SourceImage::Openverse,
            titre: Some("Feuille".to_string()),
            auteur: Some("Jane Doe".to_string()),
            url_page: "https://example.org/oeuvre".to_string(),
            url_fichier: "https://example.org/oeuvre.jpg".to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur: Some(1024),
            hauteur: Some(768),
        }];
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        stockage.sauvegarder(&projet).await.expect("persistance");
        projet
    }

    #[tokio::test]
    async fn post_audio_puis_get_projet() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .clone()
            .oneshot(requete_audio("note.wav", &wav_silence(200)))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CREATED);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.etat, EtatPipeline::AudioRecu);
        assert_eq!(projet.audio.as_deref(), Some("audio.wav"));
        assert!(temp.path().join(&projet.id).join("audio.wav").exists());
        assert!(temp.path().join("pipeline.db").exists());

        let reponse = app
            .oneshot(
                Request::get(format!("/projet/{}", projet.id))
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let relu = projet_depuis(reponse).await;
        assert_eq!(relu, projet);
    }

    #[tokio::test]
    async fn post_audio_refuse_un_format_inconnu() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_audio("notes.txt", b"du texte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        // Aucun dossier de projet n'a ete cree.
        assert_eq!(
            std::fs::read_dir(temp.path())
                .unwrap()
                .filter(|e| e.as_ref().unwrap().path().is_dir())
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn get_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(
                Request::get("/projet/inconnu123")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_projet_refuse_un_id_invalide() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(
                Request::get("/projet/pas%20un%20id")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_valider_accepte_le_scenario() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation("projetscenario", "accepte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
    }

    #[tokio::test]
    async fn post_valider_rejette_le_scenario() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation("projetscenario", "rejete"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.validation_scenario, Some(DecisionValidation::Rejete));
    }

    #[tokio::test]
    async fn post_valider_bloque_une_seconde_decision() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .clone()
            .oneshot(requete_validation("projetscenario", "accepte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);

        let reponse = app
            .oneshot(requete_validation("projetscenario", "rejete"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_valider_refuse_un_projet_sans_scenario() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        // Projet cree par upload d'audio : etat AudioRecu, pas de scenario.
        let reponse = app
            .clone()
            .oneshot(requete_audio("note.wav", &wav_silence(200)))
            .await
            .expect("reponse");
        let projet = projet_depuis(reponse).await;

        let reponse = app
            .oneshot(requete_validation(&projet.id, "accepte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_valider_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_validation("inconnu123", "accepte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_valider_accepte_les_visuels() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_visuels(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "visuels",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.validation_visuels, Some(DecisionValidation::Accepte));
        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
    }

    #[tokio::test]
    async fn post_valider_visuels_refuse_un_projet_sans_visuels() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await; // etat ScenarioGenere seulement

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "visuels",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    /// Cree en base un projet en etat `VoixPretes`, pret a etre valide.
    async fn semer_projet_voix(data_dir: &std::path::Path) -> Projet {
        let mut projet = semer_projet_visuels(data_dir).await;
        projet.etat = EtatPipeline::VoixPretes;
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.voix = vec![video_core::voix::VoixScene {
            scene: 0,
            langue: "fr".to_string(),
            fichier: "voix-a1b2.mp3".to_string(),
            duree: 6.0,
        }];
        projet.sous_titres = vec!["sous-titres-fr.srt".to_string()];
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        stockage.sauvegarder(&projet).await.expect("persistance");
        projet
    }

    #[tokio::test]
    async fn post_valider_accepte_les_voix() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_voix(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "voix",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.validation_voix, Some(DecisionValidation::Accepte));
        assert_eq!(projet.etat, EtatPipeline::VoixPretes);
    }

    #[tokio::test]
    async fn post_valider_voix_refuse_un_projet_hors_etat() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_visuels(temp.path()).await; // etat VisuelsPrets seulement

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "voix",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_remplacer_visuel_refuse_un_projet_hors_visuels_prets() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await; // etat ScenarioGenere

        let reponse = app
            .oneshot(requete_remplacement("projetscenario", 0, "une autre image"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_remplacer_visuel_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_remplacement("inconnu123", 0, "une autre image"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }
}
