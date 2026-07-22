//! Agent Publieur : publie la video finale d'un projet sur YouTube (upload
//! reprenable, visibilite `private` par defaut), phase 6 — voir
//! `docs/architecture.md` §7 (outil `publier_youtube`) et §9.
//!
//! Comme le Conteur et le Monteur, le Publieur n'est pas un LLM : il
//! orchestre l'outil `youtube` (refresh token OAuth, upload reprenable) et
//! fait respecter le garde-fou de quota journalier persiste en SQLite. C'est
//! la derniere etape du pipeline : il n'y a pas de validation humaine
//! sortante, la transition `MontagePret -> Publie` est immediate une fois le
//! montage accepte.

use std::path::Path;

use pipeline::stockage::Stockage;
use tools::youtube::{self, EndpointsYoutube};
use video_core::config::{self, Config, SecretsYoutube};
use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::{DecisionValidation, Projet, PublicationYoutube};

/// Ce qu'il faut pour publier : identifiants OAuth resolus (client ID/secret
/// + refresh token) et endpoints de l'API (Google par defaut, mock en test).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextePublication {
    /// Identifiants OAuth (jamais dans `config.toml`).
    pub secrets: SecretsYoutube,
    /// Endpoints OAuth et upload de l'API YouTube.
    pub endpoints: EndpointsYoutube,
}

impl ContextePublication {
    /// Resout le contexte depuis l'environnement (`YOUTUBE_CLIENT_ID`,
    /// `YOUTUBE_CLIENT_SECRET`, `YOUTUBE_REFRESH_TOKEN` ou
    /// `<data_dir>/youtube_token.json` ecrit par `cli youtube-auth`).
    ///
    /// Retourne `None` si les identifiants manquent : la publication est
    /// alors simplement desactivee (le pipeline s'arrete a `MontagePret`).
    pub fn depuis_environnement(data_dir: &Path) -> Option<Self> {
        Some(Self {
            secrets: config::secrets_youtube(data_dir)?,
            endpoints: EndpointsYoutube::default(),
        })
    }
}

/// Fait passer un projet de `MontagePret` (montage accepte) a `Publie` :
/// construit les metadonnees depuis le scenario (titre, description avec
/// attributions des images, tags, langue de la transcription), uploade
/// `video.mp4` avec la visibilite configuree (`private` par defaut) et
/// consigne l'identifiant et l'URL YouTube dans le projet.
///
/// Garde-fou quota (10 000 unites/jour cote API, un upload = 1 600) : au
/// dela de `config.youtube.quota_uploads_jour` uploads comptabilises pour le
/// jour courant, la publication est refusee proprement ; le compteur n'est
/// incremente qu'apres un upload reussi.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas en etat `MontagePret`, si le
///   montage n'a pas ete accepte, si la video est absente, ou si le quota
///   journalier est atteint.
/// - `Error::Tool` si l'echange OAuth ou l'upload echoue.
/// - `Error::Annulation` si l'annulation est demandee entre deux chunks
///   d'upload.
pub async fn produire_publication(
    projet: &mut Projet,
    config: &Config,
    stockage: &Stockage,
    contexte: &ContextePublication,
    token: &video_core::annulation::CancellationToken,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::MontagePret {
        return Err(Error::Pipeline(format!(
            "publication demandee sur un projet en etat {:?} (attendu : MontagePret)",
            projet.etat
        )));
    }
    if projet.validation_montage != Some(DecisionValidation::Accepte) {
        return Err(Error::Pipeline(
            "publication demandee avant acceptation du montage".to_string(),
        ));
    }
    let video = projet
        .video
        .clone()
        .ok_or_else(|| Error::Pipeline("projet sans video finale".to_string()))?;

    let uploads = stockage.uploads_du_jour().await?;
    let limite = config.youtube.quota_uploads_jour;
    if uploads >= limite {
        return Err(Error::Pipeline(format!(
            "quota journalier d'uploads YouTube atteint ({uploads}/{limite}) : \
             publication reportee a demain"
        )));
    }

    let metadonnees = youtube::construire_metadonnees(projet, &config.youtube)?;
    let chemin = config.data_dir.join(&projet.id).join(&video);
    let http = youtube::client_http()?;
    let jeton = youtube::rafraichir_token(&http, &contexte.endpoints, &contexte.secrets).await?;
    let id_video =
        youtube::publier_video(&http, &contexte.endpoints, &jeton, &metadonnees, &chemin, token)
            .await?;
    stockage.incrementer_uploads().await?;

    projet.youtube = Some(PublicationYoutube {
        url: format!("https://youtu.be/{id_video}"),
        id_video,
    });
    projet.etat = EtatPipeline::Publie;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::{Form, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{post, put};
    use axum::Router;
    use video_core::asset::{Asset, SourceImage};
    use video_core::config::{
        AudioConfig, LlmConfig, PipelineConfig, Provider, VoixConfig, YoutubeConfig,
    };
    use video_core::projet::{Segment, Transcription};
    use video_core::scenario::{Scenario, Scene};

    fn config_de_test(data_dir: &Path, quota: u32) -> Config {
        Config {
            data_dir: data_dir.to_path_buf(),
            server_addr: "127.0.0.1:0".to_string(),
            llm: LlmConfig {
                provider: Provider::Mistral,
                model: "mistral-large-latest".to_string(),
                ollama_url: None,
            },
            audio: AudioConfig::default(),
            pipeline: PipelineConfig::default(),
            voix: VoixConfig::default(),
            youtube: YoutubeConfig {
                visibilite: "private".to_string(),
                tags: vec!["education".to_string()],
                quota_uploads_jour: quota,
            },
        }
    }

    /// Projet en etat `MontagePret`, montage accepte, une scene illustree.
    fn projet_montage_accepte() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::MontagePret;
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.validation_voix = Some(DecisionValidation::Accepte);
        projet.validation_montage = Some(DecisionValidation::Accepte);
        projet.transcription = Some(Transcription {
            texte: "Sujet dicte.".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![Segment {
                debut: 0.0,
                fin: 1.0,
                texte: "Sujet dicte.".to_string(),
            }],
        });
        projet.scenario = Some(Scenario {
            titre: "Les volcans".to_string(),
            public: "college".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![Scene {
                narration: "Un volcan entre en eruption.".to_string(),
                dialogues: vec![],
                description_visuelle: "Un volcan".to_string(),
                duree_cible: 8.0,
            }],
        });
        projet.visuels = vec![Asset {
            scene: 0,
            fichier: "scene-0.jpg".to_string(),
            source: SourceImage::Openverse,
            titre: Some("Etna".to_string()),
            auteur: Some("Jane Doe".to_string()),
            url_page: "https://example.org/etna".to_string(),
            url_fichier: "https://example.org/etna.jpg".to_string(),
            licence: "CC BY 2.0".to_string(),
            licence_url: None,
            largeur: None,
            hauteur: None,
        }];
        projet.video = Some("video.mp4".to_string());
        projet.preview = Some("preview.mp4".to_string());
        projet
    }

    // --- Mock YouTube minimal (un seul chunk par upload) --------------------

    #[derive(Clone, Default)]
    struct Mock {
        descriptions: Arc<Mutex<Vec<String>>>,
        base: Arc<Mutex<String>>,
    }

    async fn mock_token(Form(champs): Form<HashMap<String, String>>) -> impl IntoResponse {
        assert_eq!(champs["grant_type"], "refresh_token");
        axum::Json(serde_json::json!({"access_token": "jeton-test"}))
    }

    async fn mock_init(State(mock): State<Mock>, corps: Bytes) -> impl IntoResponse {
        let corps: serde_json::Value = serde_json::from_slice(&corps).expect("corps JSON d'init");
        mock.descriptions.lock().expect("verrou").push(
            corps["snippet"]["description"]
                .as_str()
                .expect("description texte")
                .to_string(),
        );
        let base = mock.base.lock().expect("verrou").clone();
        (
            StatusCode::OK,
            [(
                axum::http::header::LOCATION.as_str(),
                format!("{base}/session/abc"),
            )],
            "",
        )
    }

    async fn mock_chunk(State(_mock): State<Mock>, _headers: HeaderMap) -> impl IntoResponse {
        (
            StatusCode::CREATED,
            axum::Json(serde_json::json!({"kind": "youtube#video", "id": "video123"})),
        )
    }

    /// Demarre le mock et renvoie un contexte de publication pointant dessus.
    async fn contexte_mock() -> (ContextePublication, Mock) {
        let mock = Mock::default();
        let app = Router::new()
            .route("/token", post(mock_token))
            .route("/upload", post(mock_init))
            .route("/session/abc", put(mock_chunk))
            .with_state(mock.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ecoute du mock");
        let adresse = listener.local_addr().expect("adresse du mock");
        tokio::spawn(async move { axum::serve(listener, app).await });
        let base = format!("http://{adresse}");
        *mock.base.lock().expect("verrou") = base.clone();
        let contexte = ContextePublication {
            secrets: SecretsYoutube {
                client_id: "client-test".to_string(),
                client_secret: "secret-test".to_string(),
                refresh_token: "refresh-test".to_string(),
            },
            endpoints: EndpointsYoutube {
                oauth: format!("{base}/token"),
                upload: format!("{base}/upload"),
            },
        };
        (contexte, mock)
    }

    #[tokio::test]
    async fn publie_la_video_et_passe_a_publie() {
        let (contexte, mock) = contexte_mock().await;
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path(), 6);
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let mut projet = projet_montage_accepte();
        let dossier = temp.path().join(&projet.id);
        std::fs::create_dir_all(&dossier).expect("dossier projet");
        std::fs::write(dossier.join("video.mp4"), b"fausse video").expect("video");

        produire_publication(
            &mut projet,
            &config,
            &stockage,
            &contexte,
            &video_core::annulation::CancellationToken::new(),
        )
        .await
        .expect("la publication doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::Publie);
        let publication = projet.youtube.expect("publication consignee");
        assert_eq!(publication.id_video, "video123");
        assert_eq!(publication.url, "https://youtu.be/video123");
        // Le quota du jour a ete incremente apres l'upload reussi.
        assert_eq!(stockage.uploads_du_jour().await.expect("compteur"), 1);
        // La description envoyee contient les attributions des images.
        let descriptions = mock.descriptions.lock().expect("verrou");
        assert_eq!(descriptions.len(), 1);
        assert!(descriptions[0].contains("Attributions des images :"));
        assert!(descriptions[0].contains("« Etna » par Jane Doe, CC BY 2.0"));
    }

    #[tokio::test]
    async fn refuse_la_publication_au_dela_du_quota() {
        let (contexte, _mock) = contexte_mock().await;
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path(), 2); // limite basse pour le test
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        stockage.incrementer_uploads().await.expect("increment");
        stockage.incrementer_uploads().await.expect("increment");
        let mut projet = projet_montage_accepte();

        let resultat = produire_publication(
            &mut projet,
            &config,
            &stockage,
            &contexte,
            &video_core::annulation::CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("quota"), "{message}");
                assert!(message.contains("2/2"), "{message}");
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
        // Le projet reste en etat MontagePret, rien n'a ete publie.
        assert_eq!(projet.etat, EtatPipeline::MontagePret);
        assert_eq!(projet.youtube, None);
        // Aucun upload supplementaire n'a ete comptabilise.
        assert_eq!(stockage.uploads_du_jour().await.expect("compteur"), 2);
    }

    #[tokio::test]
    async fn refuse_un_projet_hors_montage_pret() {
        let (contexte, _mock) = contexte_mock().await;
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path(), 6);
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let mut projet = projet_montage_accepte();
        projet.etat = EtatPipeline::VoixPretes;

        let resultat = produire_publication(
            &mut projet,
            &config,
            &stockage,
            &contexte,
            &video_core::annulation::CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => assert!(message.contains("MontagePret"), "{message}"),
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_un_montage_non_accepte() {
        let (contexte, _mock) = contexte_mock().await;
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path(), 6);
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let mut projet = projet_montage_accepte();
        projet.validation_montage = None; // montage pas encore tranche

        let resultat = produire_publication(
            &mut projet,
            &config,
            &stockage,
            &contexte,
            &video_core::annulation::CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("acceptation du montage"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_un_projet_sans_video() {
        let (contexte, _mock) = contexte_mock().await;
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path(), 6);
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let mut projet = projet_montage_accepte();
        projet.video = None;

        let resultat = produire_publication(
            &mut projet,
            &config,
            &stockage,
            &contexte,
            &video_core::annulation::CancellationToken::new(),
        )
        .await;
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
    }
}
