//! Outil `publier_youtube` : upload reprenable d'une video sur YouTube via
//! la Data API v3 (phase 6, voir `docs/architecture.md` §7 et §9).
//!
//! **Choix d'implementation** : l'agenda mentionne les crates `yup-oauth2` et
//! `google-youtube3`, mais celles-ci sont lourdes (generation automatique,
//! arbre de dependances volumineux) et `google-youtube3` n'est plus
//! maintenue. La surface d'API necessaire tient en deux endpoints — echange
//! de jetons OAuth (`POST https://oauth2.googleapis.com/token`) et upload
//! reprenable (`POST /upload/youtube/v3/videos?uploadType=resumable` puis
//! `PUT` par chunks avec `Content-Range`) — on les implemente donc
//! directement sur `reqwest`, deja dans le workspace, sans nouvelle
//! dependance de runtime.
//!
//! Le flux est decoupe en fonctions pures testables sans reseau
//! (construction des metadonnees, corps des requetes, parsing des reponses,
//! calcul des chunks) ; les tests d'integration tournent contre un serveur
//! HTTP local, jamais contre la vraie API.

use std::path::Path;

use serde::Deserialize;
use video_core::config::{SecretsYoutube, YoutubeConfig};
use video_core::error::Error;
use video_core::projet::Projet;

/// Taille d'un chunk d'upload (8 Mio, multiple des 256 Kio exiges par
/// l'API ; seul le dernier chunk peut etre plus petit).
const TAILLE_CHUNK: u64 = 8 * 1024 * 1024;

/// Nombre maximal de tentatives par chunk (erreur reseau ou 5xx).
const TENTATIVES_MAX_CHUNK: u32 = 3;

/// Portée OAuth demandée lors du bootstrap : upload de vidéos uniquement.
const SCOPE_UPLOAD: &str = "https://www.googleapis.com/auth/youtube.upload";

/// Endpoints HTTP de l'API Google (surchargeables pour les tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointsYoutube {
    /// Endpoint d'echange de jetons OAuth (`POST .../token`).
    pub oauth: String,
    /// Endpoint d'upload de la Data API v3 (`POST .../videos`).
    pub upload: String,
}

impl Default for EndpointsYoutube {
    fn default() -> Self {
        Self {
            oauth: "https://oauth2.googleapis.com/token".to_string(),
            upload: "https://www.googleapis.com/upload/youtube/v3/videos".to_string(),
        }
    }
}

/// Construit le client HTTP des appels YouTube (meme User-Agent que les
/// autres outils reseau).
pub fn client_http() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .user_agent("video-automation/0.1 (pipeline de videos educatives)")
        .build()
        .map_err(|e| Error::Tool(format!("construction du client HTTP : {e}")))
}

// --- Metadonnees ------------------------------------------------------------

/// Metadonnees d'une video a publier (titre, description avec attributions,
/// tags, langue, visibilite).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadonneesVideo {
    /// Titre de la video (titre du scenario).
    pub titre: String,
    /// Description, incluant la section d'attributions des images.
    pub description: String,
    /// Tags (issus de la configuration).
    pub tags: Vec<String>,
    /// Langue principale (code ISO, detectee a la transcription).
    pub langue: String,
    /// Visibilite : `private` (defaut), `unlisted` ou `public`.
    pub visibilite: String,
}

/// Visibilites acceptees par la Data API v3 (`status.privacyStatus`).
const VISIBILITES: &[&str] = &["private", "unlisted", "public"];

/// Construit les metadonnees d'une video a partir du projet et de la
/// configuration `[youtube]`.
///
/// La description reprend le titre du scenario et ajoute la section
/// **attributions** exigee par les licences CC (une ligne par visuel, via
/// `Asset::attribution()`). La langue est celle detectee a la transcription,
/// `fr` a defaut.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'a pas de scenario.
/// - `Error::Config` si la visibilite configuree est inconnue.
pub fn construire_metadonnees(
    projet: &Projet,
    config: &YoutubeConfig,
) -> Result<MetadonneesVideo, Error> {
    if !VISIBILITES.contains(&config.visibilite.as_str()) {
        return Err(Error::config(format!(
            "visibilite YouTube inconnue : {:?} (attendu : {})",
            config.visibilite,
            VISIBILITES.join(", ")
        )));
    }
    let scenario = projet
        .scenario
        .as_ref()
        .ok_or_else(|| Error::Pipeline("projet sans scenario".to_string()))?;

    let mut description = format!(
        "{}\n\nVideo educative produite automatiquement a partir d'une dictee.",
        scenario.titre
    );
    if !projet.visuels.is_empty() {
        // Attributions dans l'ordre des scenes (voir docs/architecture.md §9).
        let mut visuels = projet.visuels.clone();
        visuels.sort_by_key(|asset| asset.scene);
        description.push_str("\n\nAttributions des images :");
        for visuel in &visuels {
            description.push_str("\n- ");
            description.push_str(&visuel.attribution());
        }
    }

    let langue = projet
        .transcription
        .as_ref()
        .and_then(|t| t.langue.clone())
        .unwrap_or_else(|| "fr".to_string());

    Ok(MetadonneesVideo {
        titre: scenario.titre.clone(),
        description,
        tags: config.tags.clone(),
        langue,
        visibilite: config.visibilite.clone(),
    })
}

/// Corps JSON de la requete d'initialisation d'upload (`videos.insert`,
/// champs anglais, convention de la Data API v3).
pub fn corps_init_upload(metadonnees: &MetadonneesVideo) -> serde_json::Value {
    serde_json::json!({
        "snippet": {
            "title": metadonnees.titre,
            "description": metadonnees.description,
            "tags": metadonnees.tags,
            "defaultLanguage": metadonnees.langue,
        },
        "status": {
            "privacyStatus": metadonnees.visibilite,
        },
    })
}

// --- OAuth2 -----------------------------------------------------------------

/// Reponse brute de l'endpoint d'echange de jetons OAuth.
#[derive(Debug, Deserialize)]
struct ReponseJeton {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Extrait le premier jeton non vide d'une reponse OAuth JSON, ou traduit
/// l'erreur API (`{"error": ..., "error_description": ...}`).
fn parser_jeton(corps: &str, champ: &str) -> Result<String, Error> {
    let reponse: ReponseJeton = serde_json::from_str(corps)
        .map_err(|e| Error::Tool(format!("reponse OAuth illisible : {e}")))?;
    let jeton = match champ {
        "access_token" => reponse.access_token,
        _ => reponse.refresh_token,
    };
    if let Some(jeton) = jeton.filter(|j| !j.is_empty()) {
        return Ok(jeton);
    }
    let detail = match (reponse.error, reponse.error_description) {
        (Some(erreur), Some(description)) => format!("{erreur} : {description}"),
        (Some(erreur), None) => erreur,
        _ => format!("reponse OAuth sans {champ}"),
    };
    Err(Error::Tool(format!(
        "echange de jeton OAuth refuse : {detail}"
    )))
}

/// Echange le refresh token contre un access token (flux installed app,
/// `grant_type=refresh_token`).
///
/// # Erreurs
/// `Error::Tool` si l'appel HTTP echoue ou si l'API refuse l'echange.
pub async fn rafraichir_token(
    http: &reqwest::Client,
    endpoints: &EndpointsYoutube,
    secrets: &SecretsYoutube,
) -> Result<String, Error> {
    let reponse = http
        .post(&endpoints.oauth)
        .form(&[
            ("client_id", secrets.client_id.as_str()),
            ("client_secret", secrets.client_secret.as_str()),
            ("refresh_token", secrets.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| Error::Tool(format!("appel a l'endpoint OAuth : {e}")))?;
    let corps = reponse
        .text()
        .await
        .map_err(|e| Error::Tool(format!("lecture de la reponse OAuth : {e}")))?;
    parser_jeton(&corps, "access_token")
}

/// Construit l'URL de consentement OAuth (flux installed app avec redirection
/// loopback `http://127.0.0.1:<port>`, portée upload seule, refresh token
/// force via `access_type=offline` + `prompt=consent`).
pub fn url_consentement(client_id: &str, redirect_uri: &str) -> String {
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .expect("URL de consentement valide");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE_UPLOAD)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    url.into()
}

/// Extrait le code d'autorisation d'une requete de redirection loopback
/// (premiere ligne `GET /?code=...&scope=... HTTP/1.1`).
pub fn extraire_code(requete_http: &str) -> Option<String> {
    let chemin = requete_http.lines().next()?.split_whitespace().nth(1)?;
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{chemin}")).ok()?;
    url.query_pairs()
        .find(|(cle, _)| cle == "code")
        .map(|(_, code)| code.into_owned())
}

/// Echange un code d'autorisation contre un refresh token
/// (`grant_type=authorization_code`), lors du bootstrap `cli youtube-auth`.
///
/// # Erreurs
/// `Error::Tool` si l'appel HTTP echoue ou si l'API refuse l'echange.
pub async fn echanger_code(
    http: &reqwest::Client,
    endpoints: &EndpointsYoutube,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<String, Error> {
    let reponse = http
        .post(&endpoints.oauth)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| Error::Tool(format!("appel a l'endpoint OAuth : {e}")))?;
    let corps = reponse
        .text()
        .await
        .map_err(|e| Error::Tool(format!("lecture de la reponse OAuth : {e}")))?;
    parser_jeton(&corps, "refresh_token")
}

// --- Upload reprenable ------------------------------------------------------

/// Decoupe un fichier de `taille` octets en chunks `(debut, fin)` (bornes
/// inclusives) d'au plus `taille_chunk` octets.
pub fn decouper_chunks(taille: u64, taille_chunk: u64) -> Vec<(u64, u64)> {
    let mut chunks = Vec::new();
    let mut debut = 0;
    while debut < taille {
        let fin = (debut + taille_chunk).min(taille) - 1;
        chunks.push((debut, fin));
        debut = fin + 1;
    }
    chunks
}

/// Ouvre une session d'upload reprenable et retourne son URI (en-tete
/// `Location` de la reponse d'initialisation).
async fn initier_upload(
    http: &reqwest::Client,
    endpoints: &EndpointsYoutube,
    jeton: &str,
    metadonnees: &MetadonneesVideo,
    taille: u64,
) -> Result<String, Error> {
    let reponse = http
        .post(format!(
            "{}?uploadType=resumable&part=snippet,status",
            endpoints.upload
        ))
        .bearer_auth(jeton)
        .header("X-Upload-Content-Type", "video/mp4")
        .header("X-Upload-Content-Length", taille.to_string())
        .json(&corps_init_upload(metadonnees))
        .send()
        .await
        .map_err(|e| Error::Tool(format!("initialisation de l'upload YouTube : {e}")))?;
    let statut = reponse.status();
    if !statut.is_success() {
        let detail = reponse.text().await.unwrap_or_default();
        return Err(Error::Tool(format!(
            "l'API YouTube a refuse l'initialisation de l'upload ({statut}) : {detail}"
        )));
    }
    reponse
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|valeur| valeur.to_str().ok())
        .filter(|uri| !uri.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            Error::Tool("l'API YouTube n'a pas renvoye d'URI de session d'upload".to_string())
        })
}

/// Resultat de l'envoi d'un chunk.
#[derive(Debug, PartialEq, Eq)]
enum ReponseChunk {
    /// L'API attend la suite (`308 Resume Incomplete`).
    Incomplet,
    /// Upload termine : identifiant de la video creee.
    Termine(String),
}

/// Extrait l'identifiant de la video du corps JSON final (`{"id": "..."}`).
fn parser_id_video(corps: &str) -> Result<String, Error> {
    #[derive(Deserialize)]
    struct VideoCreee {
        id: Option<String>,
    }
    let video: VideoCreee = serde_json::from_str(corps)
        .map_err(|e| Error::Tool(format!("reponse finale YouTube illisible : {e}")))?;
    video
        .id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::Tool("reponse finale YouTube sans identifiant de video".to_string()))
}

/// Envoie un chunk sur la session d'upload (`PUT` avec `Content-Range`).
async fn envoyer_chunk(
    http: &reqwest::Client,
    session: &str,
    jeton: &str,
    octets: &[u8],
    debut: u64,
    fin: u64,
    total: u64,
) -> Result<ReponseChunk, Error> {
    let reponse = http
        .put(session)
        .bearer_auth(jeton)
        .header(
            reqwest::header::CONTENT_RANGE,
            format!("bytes {debut}-{fin}/{total}"),
        )
        .body(octets[debut as usize..=fin as usize].to_vec())
        .send()
        .await
        .map_err(|e| Error::Tool(format!("envoi d'un chunk a YouTube : {e}")))?;
    let statut = reponse.status();
    if statut == reqwest::StatusCode::PERMANENT_REDIRECT {
        // 308 Resume Incomplete : l'API a bien recu le chunk, on enchaine.
        return Ok(ReponseChunk::Incomplet);
    }
    let corps = reponse
        .text()
        .await
        .map_err(|e| Error::Tool(format!("lecture de la reponse YouTube : {e}")))?;
    if statut.is_success() {
        return Ok(ReponseChunk::Termine(parser_id_video(&corps)?));
    }
    Err(Error::Tool(format!(
        "l'API YouTube a refuse un chunk ({statut}) : {corps}"
    )))
}

/// Implementation de [`publier_video`], parametree par la taille de chunk :
/// testable en plusieurs chunks avec un petit fichier.
async fn publier_video_taille(
    http: &reqwest::Client,
    endpoints: &EndpointsYoutube,
    jeton: &str,
    metadonnees: &MetadonneesVideo,
    chemin: &Path,
    taille_chunk: u64,
) -> Result<String, Error> {
    let octets = std::fs::read(chemin)
        .map_err(|e| Error::Tool(format!("lecture de {} : {e}", chemin.display())))?;
    if octets.is_empty() {
        return Err(Error::Tool(format!(
            "fichier video vide : {}",
            chemin.display()
        )));
    }
    let total = octets.len() as u64;
    let session = initier_upload(http, endpoints, jeton, metadonnees, total).await?;

    for (debut, fin) in decouper_chunks(total, taille_chunk) {
        let mut tentatives = 0;
        loop {
            match envoyer_chunk(http, &session, jeton, &octets, debut, fin, total).await {
                Ok(ReponseChunk::Incomplet) => break,
                Ok(ReponseChunk::Termine(id)) => return Ok(id),
                Err(erreur) => {
                    // Reprise simple : le meme chunk est renvoye tel quel
                    // (l'API accepte un chunk re-emis depuis l'offset attendu).
                    tentatives += 1;
                    if tentatives >= TENTATIVES_MAX_CHUNK {
                        return Err(erreur);
                    }
                }
            }
        }
    }
    Err(Error::Tool(
        "upload YouTube termine sans identifiant de video".to_string(),
    ))
}

/// Uploade une video sur YouTube via le protocole d'upload reprenable de la
/// Data API v3 et retourne l'identifiant de la video creee.
///
/// # Erreurs
/// `Error::Tool` si le fichier est illisible ou vide, si l'initialisation
/// echoue, ou si un chunk est refuse apres plusieurs tentatives.
pub async fn publier_video(
    http: &reqwest::Client,
    endpoints: &EndpointsYoutube,
    jeton: &str,
    metadonnees: &MetadonneesVideo,
    chemin: &Path,
) -> Result<String, Error> {
    publier_video_taille(http, endpoints, jeton, metadonnees, chemin, TAILLE_CHUNK).await
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
    use video_core::projet::{Segment, Transcription};
    use video_core::scenario::{Scenario, Scene};

    fn projet_complet() -> Projet {
        let mut projet = Projet::nouveau("abc123");
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
        projet
    }

    #[test]
    fn construit_les_metadonnees_avec_attributions() {
        let projet = projet_complet();
        let config = YoutubeConfig {
            visibilite: "private".to_string(),
            tags: vec!["education".to_string()],
            quota_uploads_jour: 6,
        };
        let metadonnees = construire_metadonnees(&projet, &config).expect("metadonnees");
        assert_eq!(metadonnees.titre, "Les volcans");
        assert_eq!(metadonnees.langue, "fr");
        assert_eq!(metadonnees.visibilite, "private");
        assert_eq!(metadonnees.tags, vec!["education"]);
        assert!(metadonnees.description.starts_with("Les volcans\n"));
        assert!(metadonnees
            .description
            .contains("Attributions des images :"));
        assert!(
            metadonnees
                .description
                .contains("« Etna » par Jane Doe, CC BY 2.0 — https://example.org/etna"),
            "{}",
            metadonnees.description
        );
    }

    #[test]
    fn metadonnees_sans_visuels_n_ont_pas_de_section_attributions() {
        let mut projet = projet_complet();
        projet.visuels.clear();
        projet.transcription = None; // langue par defaut : fr
        let metadonnees =
            construire_metadonnees(&projet, &YoutubeConfig::default()).expect("metadonnees");
        assert!(!metadonnees.description.contains("Attributions"));
        assert_eq!(metadonnees.langue, "fr");
    }

    #[test]
    fn metadonnees_refusent_un_projet_sans_scenario_ou_visibilite_inconnue() {
        let projet = Projet::nouveau("abc123");
        let resultat = construire_metadonnees(&projet, &YoutubeConfig::default());
        assert!(matches!(resultat, Err(Error::Pipeline(_))));

        let projet = projet_complet();
        let config = YoutubeConfig {
            visibilite: "friends".to_string(),
            ..YoutubeConfig::default()
        };
        let resultat = construire_metadonnees(&projet, &config);
        assert!(matches!(resultat, Err(Error::Config(_))));
    }

    #[test]
    fn le_corps_d_init_suit_la_data_api_v3() {
        let projet = projet_complet();
        let metadonnees =
            construire_metadonnees(&projet, &YoutubeConfig::default()).expect("metadonnees");
        let corps = corps_init_upload(&metadonnees);
        assert_eq!(corps["snippet"]["title"], "Les volcans");
        assert_eq!(corps["snippet"]["defaultLanguage"], "fr");
        assert_eq!(corps["status"]["privacyStatus"], "private");
        assert_eq!(corps["snippet"]["tags"], serde_json::json!([]));
    }

    #[test]
    fn decoupe_en_chunks_contigus() {
        assert_eq!(decouper_chunks(0, 8), Vec::<(u64, u64)>::new());
        assert_eq!(decouper_chunks(5, 8), vec![(0, 4)]);
        assert_eq!(decouper_chunks(16, 8), vec![(0, 7), (8, 15)]);
        assert_eq!(decouper_chunks(17, 8), vec![(0, 7), (8, 15), (16, 16)]);
    }

    #[test]
    fn parse_les_reponses_oauth() {
        let jeton = parser_jeton(
            r#"{"access_token":"ya29.jeton","expires_in":3600,"token_type":"Bearer"}"#,
            "access_token",
        )
        .expect("jeton");
        assert_eq!(jeton, "ya29.jeton");

        let resultat = parser_jeton(
            r#"{"error":"invalid_grant","error_description":"Token has been revoked."}"#,
            "access_token",
        );
        match resultat {
            Err(Error::Tool(message)) => {
                assert!(message.contains("invalid_grant"), "{message}");
                assert!(message.contains("revoked"), "{message}");
            }
            autre => panic!("une erreur Tool est attendue, pas {autre:?}"),
        }

        let resultat = parser_jeton("pas du json", "access_token");
        assert!(matches!(resultat, Err(Error::Tool(_))));
    }

    #[test]
    fn parse_l_identifiant_de_video() {
        assert_eq!(
            parser_id_video(r#"{"kind":"youtube#video","id":"dQw4w9WgXcQ"}"#).expect("id"),
            "dQw4w9WgXcQ"
        );
        assert!(parser_id_video(r#"{"kind":"youtube#video"}"#).is_err());
    }

    #[test]
    fn construit_l_url_de_consentement() {
        let url = url_consentement("client-123", "http://127.0.0.1:8081");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8081"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fyoutube.upload"));
    }

    #[test]
    fn extrait_le_code_d_une_redirection_loopback() {
        let requete = "GET /?code=4%2F0AbC-d_e&scope=https://www.googleapis.com/auth/youtube.upload HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(extraire_code(requete).as_deref(), Some("4/0AbC-d_e"));
        assert_eq!(extraire_code("GET /?error=access_denied HTTP/1.1"), None);
        assert_eq!(extraire_code("n'importe quoi"), None);
    }

    // --- Serveur mock local -------------------------------------------------

    /// Etat partage du mock YouTube : octets recus, Content-Range vus, corps
    /// de la requete d'initialisation.
    #[derive(Clone, Default)]
    struct Mock {
        octets: Arc<Mutex<Vec<u8>>>,
        ranges: Arc<Mutex<Vec<String>>>,
        init: Arc<Mutex<Option<serde_json::Value>>>,
        init_headers: Arc<Mutex<Vec<(String, String)>>>,
        base: Arc<Mutex<String>>,
    }

    async fn mock_token(Form(champs): Form<HashMap<String, String>>) -> impl IntoResponse {
        assert_eq!(champs["grant_type"], "refresh_token");
        assert_eq!(champs["client_id"], "client-test");
        assert_eq!(champs["refresh_token"], "refresh-test");
        axum::Json(serde_json::json!({
            "access_token": "jeton-test",
            "expires_in": 3600,
            "token_type": "Bearer",
        }))
    }

    async fn mock_init(
        State(mock): State<Mock>,
        headers: HeaderMap,
        corps: Bytes,
    ) -> impl IntoResponse {
        *mock.init.lock().expect("verrou") =
            Some(serde_json::from_slice(&corps).expect("corps JSON d'init"));
        *mock.init_headers.lock().expect("verrou") = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let base = mock.base.lock().expect("verrou").clone();
        (
            StatusCode::OK,
            [(
                reqwest::header::LOCATION.as_str(),
                format!("{base}/session/abc"),
            )],
            "",
        )
    }

    async fn mock_chunk(
        State(mock): State<Mock>,
        headers: HeaderMap,
        corps: Bytes,
    ) -> impl IntoResponse {
        let range = headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .expect("Content-Range present")
            .to_string();
        mock.ranges.lock().expect("verrou").push(range.clone());
        mock.octets
            .lock()
            .expect("verrou")
            .extend_from_slice(&corps);
        // "bytes D-F/T" : chunk final si F == T - 1.
        let (plage, total) = range
            .strip_prefix("bytes ")
            .and_then(|r| r.split_once('/'))
            .expect("Content-Range bien forme");
        let fin: u64 = plage
            .split('-')
            .nth(1)
            .expect("borne haute")
            .parse()
            .expect("borne");
        let total: u64 = total.parse().expect("total");
        if fin + 1 == total {
            (
                StatusCode::CREATED,
                axum::Json(serde_json::json!({"kind": "youtube#video", "id": "video123"})),
            )
                .into_response()
        } else {
            StatusCode::PERMANENT_REDIRECT.into_response()
        }
    }

    /// Demarre un mock YouTube sur 127.0.0.1 (endpoints OAuth + upload).
    async fn serveur_mock() -> (EndpointsYoutube, Mock) {
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
        (
            EndpointsYoutube {
                oauth: format!("{base}/token"),
                upload: format!("{base}/upload"),
            },
            mock,
        )
    }

    fn secrets_de_test() -> SecretsYoutube {
        SecretsYoutube {
            client_id: "client-test".to_string(),
            client_secret: "secret-test".to_string(),
            refresh_token: "refresh-test".to_string(),
        }
    }

    #[tokio::test]
    async fn rafraichit_le_token_contre_le_mock() {
        let (endpoints, _mock) = serveur_mock().await;
        let http = client_http().expect("client HTTP");
        let jeton = rafraichir_token(&http, &endpoints, &secrets_de_test())
            .await
            .expect("refresh token accepte");
        assert_eq!(jeton, "jeton-test");
    }

    #[tokio::test]
    async fn publie_une_video_en_un_seul_chunk() {
        let (endpoints, mock) = serveur_mock().await;
        let http = client_http().expect("client HTTP");
        let projet = projet_complet();
        let metadonnees =
            construire_metadonnees(&projet, &YoutubeConfig::default()).expect("metadonnees");
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let video = temp.path().join("video.mp4");
        std::fs::write(&video, b"fausse video mp4").expect("ecriture de la video");

        let id = publier_video(&http, &endpoints, "jeton-test", &metadonnees, &video)
            .await
            .expect("upload reussi");
        assert_eq!(id, "video123");

        // Le mock a recu les octets du fichier, les bons en-tetes d'init et
        // le corps de metadonnees attendu.
        assert_eq!(
            mock.octets.lock().expect("verrou").as_slice(),
            b"fausse video mp4"
        );
        let headers = mock.init_headers.lock().expect("verrou");
        let entete = |nom: &str| {
            headers
                .iter()
                .find(|(k, _)| k == nom)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(entete("authorization"), "Bearer jeton-test");
        assert_eq!(entete("x-upload-content-type"), "video/mp4");
        assert_eq!(
            entete("x-upload-content-length"),
            b"fausse video mp4".len().to_string()
        );
        let init = mock.init.lock().expect("verrou").clone().expect("init vue");
        assert_eq!(init["snippet"]["title"], "Les volcans");
        assert_eq!(init["status"]["privacyStatus"], "private");
        assert_eq!(
            mock.ranges.lock().expect("verrou").as_slice(),
            ["bytes 0-15/16"]
        );
    }

    #[tokio::test]
    async fn publie_une_video_en_plusieurs_chunks_avec_308() {
        let (endpoints, mock) = serveur_mock().await;
        let http = client_http().expect("client HTTP");
        let projet = projet_complet();
        let metadonnees =
            construire_metadonnees(&projet, &YoutubeConfig::default()).expect("metadonnees");
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let video = temp.path().join("video.mp4");
        // 10 octets avec des chunks de 4 : 3 PUT, les deux premiers en 308.
        std::fs::write(&video, b"0123456789").expect("ecriture de la video");

        let id = publier_video_taille(&http, &endpoints, "jeton-test", &metadonnees, &video, 4)
            .await
            .expect("upload reussi");
        assert_eq!(id, "video123");
        assert_eq!(
            mock.ranges.lock().expect("verrou").as_slice(),
            ["bytes 0-3/10", "bytes 4-7/10", "bytes 8-9/10"]
        );
        assert_eq!(
            mock.octets.lock().expect("verrou").as_slice(),
            b"0123456789"
        );
    }

    #[tokio::test]
    async fn refuse_un_fichier_video_vide() {
        let (endpoints, _mock) = serveur_mock().await;
        let http = client_http().expect("client HTTP");
        let projet = projet_complet();
        let metadonnees =
            construire_metadonnees(&projet, &YoutubeConfig::default()).expect("metadonnees");
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let video = temp.path().join("video.mp4");
        std::fs::write(&video, b"").expect("ecriture de la video");

        let resultat = publier_video(&http, &endpoints, "jeton-test", &metadonnees, &video).await;
        match resultat {
            Err(Error::Tool(message)) => assert!(message.contains("vide"), "{message}"),
            autre => panic!("une erreur Tool est attendue, pas {autre:?}"),
        }
    }

    /// Verification reelle du rafraichissement de jeton contre l'API Google :
    /// ignoree tant que `VIDEO_TEST_RESEAU` et les secrets OAuth ne sont pas
    /// definis (donc en CI).
    #[tokio::test]
    async fn rafraichit_un_vrai_token() {
        if std::env::var("VIDEO_TEST_RESEAU").is_err() {
            eprintln!("VIDEO_TEST_RESEAU absente : rafraichit_un_vrai_token ignore.");
            return;
        }
        let secrets = video_core::config::secrets_youtube(std::path::Path::new("data"))
            .expect("YOUTUBE_CLIENT_ID/SECRET et refresh token requis");
        let http = client_http().expect("client HTTP");
        let jeton = rafraichir_token(&http, &EndpointsYoutube::default(), &secrets)
            .await
            .expect("le refresh token doit etre accepte par Google");
        assert!(!jeton.is_empty());
    }
}
