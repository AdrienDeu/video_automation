//! Gestionnaires des routes HTTP du serveur.

use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;
use video_core::etat::EtatPipeline;
use video_core::projet::Projet;

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

/// `POST /audio` : recoit un fichier audio (multipart, champ `audio`, champ
/// optionnel `langue`), le stocke dans `data/<id>/` puis le transcrit via
/// l'API Mistral si `MISTRAL_API_KEY` est definie.
///
/// Sans cle API, l'audio est simplement stocke et le projet reste en etat
/// `AudioRecu` (la transcription pourra etre relancee). En cas d'echec du
/// STT, le projet est persiste en etat `Erreur` et renvoye avec un statut
/// `502`.
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
    store::sauvegarder(&etat.config.data_dir, &projet)
        .await
        .map_err(|e| erreur_interne("persistance du projet", e))?;

    // Sans cle API, l'audio est stocke et la transcription reste a faire.
    let Some(cle) = &etat.cle_api else {
        return Ok((StatusCode::CREATED, Json(projet)));
    };

    match tools::transcrire::transcrire_audio(&chemin_audio, langue.as_deref(), cle).await {
        Ok(transcription) => {
            projet.transcription = Some(transcription);
            projet.etat = EtatPipeline::Transcrit;
            store::sauvegarder(&etat.config.data_dir, &projet)
                .await
                .map_err(|e| erreur_interne("persistance du projet", e))?;
            Ok((StatusCode::CREATED, Json(projet)))
        }
        Err(erreur) => {
            projet.etat = EtatPipeline::Erreur(erreur.to_string());
            store::sauvegarder(&etat.config.data_dir, &projet)
                .await
                .map_err(|e| erreur_interne("persistance du projet", e))?;
            Ok((StatusCode::BAD_GATEWAY, Json(projet)))
        }
    }
}

/// `GET /projet/{id}` : renvoie l'etat d'un projet (et sa transcription une
/// fois l'etape STT terminee).
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
    match store::charger(&etat.config.data_dir, &id).await {
        Ok(Some(projet)) => Ok(Json(projet)),
        Ok(None) => Err((StatusCode::NOT_FOUND, format!("projet inconnu : {id}"))),
        Err(e) => Err(erreur_interne("lecture du projet", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use video_core::config::{AudioConfig, Config, LlmConfig, Provider};

    use crate::construire_routeur;

    /// Construit l'application avec un dossier de donnees temporaire et sans
    /// cle API (la transcription est alors desactivee).
    fn app_de_test(data_dir: std::path::PathBuf) -> Router {
        let config = Config {
            data_dir,
            server_addr: "127.0.0.1:0".to_string(),
            llm: LlmConfig {
                provider: Provider::Mistral,
                model: "mistral-large-latest".to_string(),
                ollama_url: None,
            },
            audio: AudioConfig::default(),
        };
        construire_routeur(Arc::new(AppState {
            config,
            cle_api: None,
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

    #[tokio::test]
    async fn post_audio_puis_get_projet() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf());

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
        assert!(temp.path().join(&projet.id).join("projet.json").exists());

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
        let app = app_de_test(temp.path().to_path_buf());

        let reponse = app
            .oneshot(requete_audio("notes.txt", b"du texte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        // Rien n'a ete persiste.
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn get_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf());

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
        let app = app_de_test(temp.path().to_path_buf());

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
}
