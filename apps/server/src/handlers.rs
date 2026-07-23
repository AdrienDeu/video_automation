//! Gestionnaires des routes HTTP du serveur.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::Json;
use pipeline::stockage::ProjetResume;
use serde::Deserialize;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use video_core::annulation::{point_de_controle, CancellationToken};
use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::{DecisionValidation, Projet};

use crate::store;
use crate::{audio, tache, AppState};

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

/// Flux SSE d'un projet : un evenement JSON par changement d'etat.
type FluxProjet = Sse<ReceiverStream<Result<Event, Infallible>>>;

/// Persiste le projet puis notifie les abonnes SSE (`GET /projet/{id}/events`)
/// du changement. Sans abonne, la notification est simplement perdue.
pub(crate) async fn sauvegarder_et_notifier(
    etat: &AppState,
    projet: &Projet,
) -> Result<(), ErreurHttp> {
    etat.stockage
        .sauvegarder(projet)
        .await
        .map_err(|e| erreur_interne("persistance du projet", e))?;
    let _ = etat.evenements.send(projet.id.clone());
    Ok(())
}

/// Persiste un point d'etape intermediaire depuis la tache de fond (les
/// abonnes SSE suivent la progression et un crash ne perd pas l'etape).
async fn sauvegarder_point_d_etape(etat: &AppState, projet: &Projet) -> Result<(), Error> {
    sauvegarder_et_notifier(etat, projet)
        .await
        .map_err(|(_, message)| Error::Persistance(message))
}

/// Fait avancer le pipeline tant que les portes sont ouvertes : transcription
/// → scenario (Scenariste), puis, si le scenario est accepte, → visuels
/// (Visuel), puis, si les visuels sont acceptes, → voix (Conteur), puis, si
/// les voix sont acceptees, → montage (Monteur), puis, si le montage est
/// accepte, → publication (Publieur, derniere etape). S'arrete des qu'une
/// transition en mode `validation` bloque.
///
/// La publication n'est tentee que si les identifiants OAuth YouTube sont
/// configures (sinon le projet reste en `MontagePret` accepte ; ni le montage
/// ni la publication n'ont besoin de la cle Mistral).
///
/// Executee dans la tache de fond du projet (`tache::lancer_pipeline`) :
/// chaque etape franchie est persistee aussitot (suivi SSE, reprise apres
/// crash) et le token d'annulation est verifie avant chaque etape — les
/// etapes elles-memes le verifient aussi dans leurs boucles internes.
///
/// # Erreurs
/// `Error::Annulation` si l'annulation est demandee entre deux etapes ;
/// sinon l'erreur de l'etape en echec.
pub(crate) async fn avancer_pipeline(
    etat: &AppState,
    projet: &mut Projet,
    token: &CancellationToken,
) -> Result<(), Error> {
    if projet.etat == EtatPipeline::Transcrit {
        point_de_controle(token)?;
        let extracteur = etat
            .scenariste
            .as_ref()
            .ok_or_else(|| Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string()))?;
        agents::realisateur::produire_scenario(
            projet,
            extracteur.as_ref(),
            etat.config.pipeline.scenario,
        )
        .await?;
        sauvegarder_point_d_etape(etat, projet).await?;
    }
    if projet.etat == EtatPipeline::ScenarioGenere
        && projet.validation_scenario == Some(DecisionValidation::Accepte)
    {
        point_de_controle(token)?;
        agents::visuel::produire_visuels_depuis_config(
            projet,
            &etat.config,
            etat.config.pipeline.visuels,
            token,
        )
        .await?;
        sauvegarder_point_d_etape(etat, projet).await?;
    }
    if projet.etat == EtatPipeline::VisuelsPrets
        && projet.validation_visuels == Some(DecisionValidation::Accepte)
    {
        point_de_controle(token)?;
        agents::conteur::produire_voix(projet, &etat.config, etat.config.pipeline.voix, token)
            .await?;
        sauvegarder_point_d_etape(etat, projet).await?;
    }
    if projet.etat == EtatPipeline::VoixPretes
        && projet.validation_voix == Some(DecisionValidation::Accepte)
    {
        point_de_controle(token)?;
        agents::monteur::produire_montage(
            projet,
            &etat.config,
            etat.config.pipeline.montage,
            token,
        )
        .await?;
        sauvegarder_point_d_etape(etat, projet).await?;
    }
    if projet.etat == EtatPipeline::MontagePret
        && projet.validation_montage == Some(DecisionValidation::Accepte)
    {
        if let Some(contexte) = &etat.youtube {
            point_de_controle(token)?;
            agents::publieur::produire_publication(
                projet,
                &etat.config,
                &etat.stockage,
                contexte,
                token,
            )
            .await?;
            sauvegarder_point_d_etape(etat, projet).await?;
        }
    }
    Ok(())
}

/// `POST /audio` : recoit un fichier audio (multipart, champ `audio`, champ
/// optionnel `langue`), le stocke dans `data/<id>/` puis, si
/// `MISTRAL_API_KEY` est definie, lance la transcription STT (Voxtral) et la
/// suite du pipeline en tache de fond (`tache::lancer_pipeline`).
///
/// Sans cle API, l'audio est simplement stocke et le projet reste en etat
/// `AudioRecu`. La reponse est immediate (`201`) : la progression est suivie
/// via `GET /projet/{id}` ou le flux SSE ; un echec d'etape est persiste en
/// etat `Erreur`, une annulation (`POST /annuler`) en etat `Annule`.
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
    sauvegarder_et_notifier(&etat, &projet).await?;

    // Sans cle API, l'audio est stocke et les etapes LLM restent a faire.
    if etat.cle_api.is_some() {
        tache::lancer_pipeline(&etat, &projet.id, tache::Demande::Transcription { langue });
    }
    Ok((StatusCode::CREATED, Json(projet)))
}

/// `GET /projets` : liste legere des projets (id, etat, date de mise a jour),
/// du plus recent au plus ancien, pour l'interface web.
pub async fn get_projets(
    State(etat): State<Arc<AppState>>,
) -> Result<Json<Vec<ProjetResume>>, ErreurHttp> {
    etat.stockage
        .lister()
        .await
        .map(Json)
        .map_err(|e| erreur_interne("liste des projets", e))
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

/// `GET /projet/{id}/fichier/{nom}` : sert un fichier du dossier du projet
/// (images, voix, sous-titres) pour l'interface web.
///
/// Le nom doit etre un simple nom de fichier : tout separateur de chemin ou
/// `..` est rejete (`400`), comme les identifiants invalides. Un fichier
/// absent renvoie `404`.
pub async fn get_fichier(
    State(etat): State<Arc<AppState>>,
    Path((id, nom)): Path<(String, String)>,
) -> Result<([(header::HeaderName, &'static str); 1], Vec<u8>), ErreurHttp> {
    if !store::id_valide(&id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "identifiant de projet invalide".to_string(),
        ));
    }
    if nom.is_empty() || nom.contains(['/', '\\']) || nom.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            "nom de fichier invalide".to_string(),
        ));
    }
    let chemin = store::dossier_projet(&etat.config.data_dir, &id).join(&nom);
    let octets = tokio::fs::read(&chemin).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => (StatusCode::NOT_FOUND, format!("fichier inconnu : {nom}")),
        _ => erreur_interne("lecture du fichier", e),
    })?;
    Ok(([(header::CONTENT_TYPE, type_mime(&nom))], octets))
}

/// Content-Type deduit de l'extension du fichier servi.
fn type_mime(nom: &str) -> &'static str {
    let extension = nom.rsplit('.').next().unwrap_or("").to_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "srt" => "text/plain; charset=utf-8",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// Corps de `POST /valider`.
#[derive(Debug, Deserialize)]
pub struct RequeteValidation {
    /// Identifiant du projet a trancher.
    pub id: String,
    /// Decision prise sur l'etape (`accepte` ou `rejete`).
    pub decision: DecisionValidation,
    /// Etape concernee (`scenario` par defaut, `visuels`, `voix` ou
    /// `montage`).
    pub etape: Option<pipeline::validation::EtapeValidation>,
}

/// `POST /valider` : enregistre la decision humaine sur une etape en mode
/// `validation` (scenario par defaut, visuels, voix ou montage via `etape`).
///
/// Apres une acceptation, la suite du pipeline est lancee en tache de fond si
/// possible (visuels apres scenario, voix apres visuels, montage apres voix —
/// ces etapes exigent la cle Mistral ; apres acceptation du montage, la
/// publication — qui n'en a pas besoin — est lancee des lors que les
/// identifiants YouTube sont configures). La reponse est immediate : la
/// progression est suivie via `GET /projet/{id}` ou le flux SSE.
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

    sauvegarder_et_notifier(&etat, &projet).await?;

    // Etape acceptee : la suite du pipeline part en tache de fond
    // (`avancer_pipeline` ne franchit que les portes ouvertes).
    let suite_possible = etat.cle_api.is_some()
        || (etape == pipeline::validation::EtapeValidation::Montage && etat.youtube.is_some());
    if requete.decision == DecisionValidation::Accepte && suite_possible {
        tache::lancer_pipeline(&etat, &projet.id, tache::Demande::Pipeline);
    }

    Ok((StatusCode::OK, Json(projet)))
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

    sauvegarder_et_notifier(&etat, &projet).await?;
    Ok(Json(projet))
}

/// Corps de `POST /affiner`.
#[derive(Debug, Deserialize)]
pub struct RequeteAffinage {
    /// Identifiant du projet.
    pub id: String,
    /// Etape a regenerer (`scenario`, `visuels`, `voix` ou `montage`).
    pub etape: pipeline::validation::EtapeValidation,
    /// Consigne d'affinage de l'utilisateur (integree au prompt du
    /// Scenariste ou du Visuel ; journalisee mais sans effet pour les
    /// regenerations mecaniques des voix et du montage).
    pub prompt: String,
}

/// `POST /affiner` : regenere une etape avec une consigne utilisateur, puis
/// relance uniquement les etapes impactees (phase 7).
///
/// Les artefacts et validations des etapes strictement en aval sont invalides
/// (`pipeline::affiner::reinitialiser_aval`) et persistes tels quels, puis la
/// regeneration et l'enchainement partent en tache de fond
/// (`tache::lancer_pipeline`) : la reponse est immediate, la progression est
/// suivie via `GET /projet/{id}` ou le flux SSE. La validation de l'etape
/// affinee devra etre re-tranchee si sa porte est en mode `validation`.
///
/// Renvoie `404` si le projet est inconnu, `409` s'il n'a pas encore atteint
/// l'etape demandee ; un echec de regeneration est persiste en etat `Erreur`.
pub async fn post_affiner(
    State(etat): State<Arc<AppState>>,
    Json(requete): Json<RequeteAffinage>,
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

    pipeline::affiner::reinitialiser_aval(&mut projet, requete.etape)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    sauvegarder_et_notifier(&etat, &projet).await?;

    tache::lancer_pipeline(
        &etat,
        &projet.id,
        tache::Demande::Affinage {
            etape: requete.etape,
            prompt: requete.prompt,
        },
    );
    Ok((StatusCode::OK, Json(projet)))
}

/// Regenere le livrable d'une etape avec la consigne d'affinage, une fois
/// l'aval invalide. Les voix et le montage sont des regenerations mecaniques
/// (pas de LLM) : la consigne est journalisee, sans effet.
pub(crate) async fn regenerer_etape(
    etat: &AppState,
    projet: &mut Projet,
    etape: pipeline::validation::EtapeValidation,
    prompt: &str,
    token: &CancellationToken,
) -> Result<(), Error> {
    match etape {
        pipeline::validation::EtapeValidation::Scenario => {
            let extracteur = etat.scenariste.as_ref().ok_or_else(|| {
                Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string())
            })?;
            agents::realisateur::affiner_scenario(
                projet,
                extracteur.as_ref(),
                prompt,
                etat.config.pipeline.scenario,
            )
            .await
        }
        pipeline::validation::EtapeValidation::Visuels => {
            agents::visuel::affiner_visuels_depuis_config(
                projet,
                &etat.config,
                etat.config.pipeline.visuels,
                prompt,
                token,
            )
            .await
        }
        pipeline::validation::EtapeValidation::Voix => {
            eprintln!(
                "affinage des voix du projet {} : regeneration mecanique, \
                 consigne journalisee seulement : {prompt}",
                projet.id
            );
            agents::conteur::produire_voix(projet, &etat.config, etat.config.pipeline.voix, token)
                .await
        }
        pipeline::validation::EtapeValidation::Montage => {
            eprintln!(
                "affinage du montage du projet {} : regeneration mecanique, \
                 consigne journalisee seulement : {prompt}",
                projet.id
            );
            agents::monteur::produire_montage(
                projet,
                &etat.config,
                etat.config.pipeline.montage,
                token,
            )
            .await
        }
    }
}

/// Corps de `POST /annuler` et `POST /reprendre`.
#[derive(Debug, Deserialize)]
pub struct RequeteProjet {
    /// Identifiant du projet.
    pub id: String,
}

/// `POST /annuler` : interrompt le traitement d'un projet, a n'importe quelle
/// etape (phase 8).
///
/// Si une tache de pipeline est en cours, son token d'annulation est
/// declenche : elle s'arrete a son prochain point de controle (entre deux
/// scenes, deux rendus ffmpeg — le process est alors tue — ou deux chunks
/// d'upload) et persiste le projet en etat `Annule` ; la reponse est alors
/// `202`, l'etat `Annule` etant suivi via `GET /projet/{id}` ou le flux SSE.
/// Sans tache en cours, le projet est marque `Annule` immediatement (`200`).
///
/// Un projet annule est reprendable via `POST /reprendre`. Renvoie `404` si
/// le projet est inconnu, `409` s'il est deja publie ou deja annule.
pub async fn post_annuler(
    State(etat): State<Arc<AppState>>,
    Json(requete): Json<RequeteProjet>,
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
    match projet.etat {
        EtatPipeline::Publie => {
            return Err((
                StatusCode::CONFLICT,
                "projet deja publie : annulation impossible".to_string(),
            ))
        }
        EtatPipeline::Annule => {
            return Err((StatusCode::CONFLICT, "projet deja annule".to_string()))
        }
        _ => {}
    }

    // Tache en cours ? C'est elle qui persistera `Annule` apres son point de
    // controle : on se contente de declencher son token.
    let token = etat
        .taches
        .lock()
        .expect("mutex non empoisonne")
        .get(&requete.id)
        .cloned();
    if let Some(token) = token {
        token.cancel();
        return Ok((StatusCode::ACCEPTED, Json(projet)));
    }

    projet.etat = EtatPipeline::Annule;
    sauvegarder_et_notifier(&etat, &projet).await?;
    Ok((StatusCode::OK, Json(projet)))
}

/// `POST /reprendre` : relance le pipeline d'un projet annule depuis son
/// dernier point stable (phase 8).
///
/// L'etat de reprise est derive des livrables deja produits
/// (`video_core::annulation::point_de_reprise`) : les etapes abouties ne sont
/// pas rejouees, les voix deja synthetisees sont reutilisees (cache TTS).
/// La suite part en tache de fond si une cle API est disponible (ou si seule
/// la publication reste a faire et que YouTube est configure).
///
/// Renvoie `404` si le projet est inconnu, `409` s'il n'est pas en etat
/// `Annule`.
pub async fn post_reprendre(
    State(etat): State<Arc<AppState>>,
    Json(requete): Json<RequeteProjet>,
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
    if projet.etat != EtatPipeline::Annule {
        return Err((
            StatusCode::CONFLICT,
            format!("reprise demandee sur un projet en etat {:?}", projet.etat),
        ));
    }

    projet.etat = video_core::annulation::point_de_reprise(&projet);
    sauvegarder_et_notifier(&etat, &projet).await?;

    let suite_possible = etat.cle_api.is_some()
        || (projet.etat == EtatPipeline::MontagePret && etat.youtube.is_some());
    if suite_possible {
        tache::lancer_pipeline(&etat, &projet.id, tache::Demande::Pipeline);
    }
    Ok(Json(projet))
}

/// `GET /projet/{id}/events` : flux SSE (`text/event-stream`) de l'etat d'un
/// projet (phase 7). L'etat courant est emis a l'abonnement, puis a chaque
/// changement persiste par les handlers (`sauvegarder_et_notifier`).
///
/// Renvoie `404` si le projet est inconnu.
pub async fn get_projet_events(
    State(etat): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<FluxProjet, ErreurHttp> {
    if !store::id_valide(&id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "identifiant de projet invalide".to_string(),
        ));
    }
    match etat.stockage.charger(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, format!("projet inconnu : {id}"))),
        Err(e) => return Err(erreur_interne("lecture du projet", e)),
    }

    // Abonnement AVANT de repondre : aucune notification ne peut etre perdue
    // entre l'etat initial et l'ecoute.
    let mut abonnement = etat.evenements.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        let mut premiere = true;
        loop {
            // Premier tour : etat initial. Ensuite : une notification visant
            // ce projet (ou un rattrapage apres lag) declenche un renvoi.
            let a_envoyer = if premiere {
                premiere = false;
                true
            } else {
                match abonnement.recv().await {
                    Ok(id_notifie) => id_notifie == id,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            };
            if !a_envoyer {
                continue;
            }
            // L'etat est recharge a chaque notification : le canal ne
            // transporte que des signaux, jamais de donnees potentiellement
            // perimees.
            let projet = match etat.stockage.charger(&id).await {
                Ok(Some(projet)) => projet,
                Ok(None) => break, // projet supprime : fin du flux
                Err(_) => continue,
            };
            let donnees = match serde_json::to_string(&projet) {
                Ok(donnees) => donnees,
                Err(_) => continue,
            };
            if tx
                .send(Ok(Event::default().event("projet").data(donnees)))
                .await
                .is_err()
            {
                break; // client deconnecte
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)))
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
        app_de_test_youtube(data_dir, None).await
    }

    /// Construit l'application avec un contexte de publication YouTube
    /// optionnel (mock local dans les tests de phase 6).
    async fn app_de_test_youtube(
        data_dir: std::path::PathBuf,
        youtube: Option<agents::publieur::ContextePublication>,
    ) -> Router {
        app_de_test_complete(data_dir, youtube, None).await
    }

    /// Construit l'application avec un Scenariste injecte (mock sans reseau
    /// pour les tests d'affinage de phase 7).
    async fn app_de_test_avec_scenariste(
        data_dir: std::path::PathBuf,
        scenariste: Arc<dyn llm::scenariste::ExtracteurScenario>,
    ) -> Router {
        app_de_test_complete(data_dir, None, Some(scenariste)).await
    }

    /// Constructeur commun des applications de test.
    async fn app_de_test_complete(
        data_dir: std::path::PathBuf,
        youtube: Option<agents::publieur::ContextePublication>,
        scenariste: Option<Arc<dyn llm::scenariste::ExtracteurScenario>>,
    ) -> Router {
        let stockage = Stockage::ouvrir(&data_dir)
            .await
            .expect("ouverture de la base de test");
        let (evenements, _) = tokio::sync::broadcast::channel(64);
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
            youtube: video_core::config::YoutubeConfig::default(),
        };
        construire_routeur(Arc::new(AppState {
            config,
            cle_api: None,
            youtube,
            stockage,
            scenariste,
            evenements,
            taches: std::sync::Mutex::new(std::collections::HashMap::new()),
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

    /// Attend que la tache de fond d'un projet aboutisse : recharge le projet
    /// en base jusqu'a ce que `predicat` soit vrai (timeout ~5 s).
    async fn attendre_projet(
        data_dir: &std::path::Path,
        id: &str,
        predicat: impl Fn(&Projet) -> bool,
    ) -> Projet {
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        for _ in 0..250 {
            let projet = stockage
                .charger(id)
                .await
                .expect("chargement")
                .expect("projet present");
            if predicat(&projet) {
                return projet;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("la tache de fond du projet {id} n'a pas abouti avant le timeout");
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

    /// Cree en base un projet en etat `MontagePret`, pret a etre valide.
    async fn semer_projet_montage(data_dir: &std::path::Path) -> Projet {
        let mut projet = semer_projet_voix(data_dir).await;
        projet.etat = EtatPipeline::MontagePret;
        projet.validation_voix = Some(DecisionValidation::Accepte);
        projet.video = Some("video.mp4".to_string());
        projet.preview = Some("preview.mp4".to_string());
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        stockage.sauvegarder(&projet).await.expect("persistance");
        projet
    }

    #[tokio::test]
    async fn post_valider_accepte_le_montage() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_montage(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "montage",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.validation_montage, Some(DecisionValidation::Accepte));
        // Sans cle Mistral ni identifiants YouTube, le pipeline s'arrete la.
        assert_eq!(projet.etat, EtatPipeline::MontagePret);
    }

    /// Demarre un mock YouTube local (OAuth + upload reprenable) et renvoie
    /// le contexte de publication pointant dessus.
    async fn contexte_youtube_mock() -> agents::publieur::ContextePublication {
        use axum::extract::State;
        use axum::response::IntoResponse;
        use axum::routing::{post, put};

        #[derive(Clone)]
        struct EtatMock {
            base: Arc<String>,
        }

        async fn token() -> Json<serde_json::Value> {
            Json(serde_json::json!({"access_token": "jeton-test"}))
        }
        async fn init(State(etat): State<EtatMock>) -> impl IntoResponse {
            (
                StatusCode::OK,
                [(
                    axum::http::header::LOCATION.as_str(),
                    format!("{}/session", etat.base),
                )],
                "",
            )
        }
        async fn chunk() -> impl IntoResponse {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"kind": "youtube#video", "id": "video123"})),
            )
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ecoute du mock");
        let adresse = listener.local_addr().expect("adresse du mock");
        let base = format!("http://{adresse}");
        let app = Router::new()
            .route("/token", post(token))
            .route("/upload", post(init))
            .route("/session", put(chunk))
            .with_state(EtatMock {
                base: Arc::new(base.clone()),
            });
        tokio::spawn(async move { axum::serve(listener, app).await });
        agents::publieur::ContextePublication {
            secrets: video_core::config::SecretsYoutube {
                client_id: "client-test".to_string(),
                client_secret: "secret-test".to_string(),
                refresh_token: "refresh-test".to_string(),
            },
            endpoints: tools::youtube::EndpointsYoutube {
                oauth: format!("{base}/token"),
                upload: format!("{base}/upload"),
            },
        }
    }

    /// Ecrit la fausse video finale attendue par le Publieur.
    async fn semer_video(data_dir: &std::path::Path) {
        let dossier = data_dir.join("projetscenario");
        tokio::fs::create_dir_all(&dossier)
            .await
            .expect("creation du dossier du projet");
        tokio::fs::write(dossier.join("video.mp4"), b"fausse video")
            .await
            .expect("ecriture de la video");
    }

    #[tokio::test]
    async fn post_valider_montage_enchaine_la_publication_sans_cle_mistral() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let contexte = contexte_youtube_mock().await;
        let app = app_de_test_youtube(temp.path().to_path_buf(), Some(contexte)).await;
        semer_projet_montage(temp.path()).await;
        semer_video(temp.path()).await;

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "montage",
            ))
            .await
            .expect("reponse");
        // La decision est enregistree immediatement ; la publication part en
        // tache de fond.
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.validation_montage, Some(DecisionValidation::Accepte));
        assert_eq!(projet.etat, EtatPipeline::MontagePret);

        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            p.etat == EtatPipeline::Publie
        })
        .await;
        let publication = projet.youtube.expect("publication consignee");
        assert_eq!(publication.id_video, "video123");
        assert_eq!(publication.url, "https://youtu.be/video123");
    }

    #[tokio::test]
    async fn post_valider_montage_marque_le_projet_en_erreur_au_dela_du_quota() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let contexte = contexte_youtube_mock().await;
        let app = app_de_test_youtube(temp.path().to_path_buf(), Some(contexte)).await;
        semer_projet_montage(temp.path()).await;
        semer_video(temp.path()).await;
        // Quota du jour epuise (6 uploads par defaut).
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        for _ in 0..6 {
            stockage.incrementer_uploads().await.expect("increment");
        }

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "montage",
            ))
            .await
            .expect("reponse");
        // La decision est acceptee ; l'echec de publication survient dans la
        // tache de fond, qui persiste le projet en etat Erreur.
        assert_eq!(reponse.status(), StatusCode::OK);

        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            matches!(p.etat, EtatPipeline::Erreur(_))
        })
        .await;
        match &projet.etat {
            EtatPipeline::Erreur(message) => assert!(message.contains("quota"), "{message}"),
            autre => panic!("un etat Erreur est attendu, pas {autre:?}"),
        }
        assert_eq!(projet.youtube, None);
    }

    #[tokio::test]
    async fn post_valider_montage_refuse_un_projet_hors_etat() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_voix(temp.path()).await; // etat VoixPretes seulement

        let reponse = app
            .oneshot(requete_validation_etape(
                "projetscenario",
                "accepte",
                "montage",
            ))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn get_fichier_sert_une_video_mp4() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_montage(temp.path()).await;
        let dossier = temp.path().join("projetscenario");
        tokio::fs::create_dir_all(&dossier)
            .await
            .expect("creation du dossier du projet");
        tokio::fs::write(dossier.join("preview.mp4"), b"fausse video")
            .await
            .expect("ecriture de la video");

        let reponse = app
            .oneshot(
                Request::get("/projet/projetscenario/fichier/preview.mp4")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        assert_eq!(
            reponse.headers()["content-type"],
            axum::http::HeaderValue::from_static("video/mp4")
        );
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

    #[tokio::test]
    async fn get_projets_liste_les_projets_semes() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        stockage
            .sauvegarder(&Projet::nouveau("autreprojet"))
            .await
            .expect("persistance");

        let reponse = app
            .oneshot(
                Request::get("/projets")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let octets = reponse
            .into_body()
            .collect()
            .await
            .expect("lecture du corps")
            .to_bytes();
        let resumes: Vec<serde_json::Value> =
            serde_json::from_slice(&octets).expect("corps JSON valide");
        assert_eq!(resumes.len(), 2);
        let ids: Vec<&str> = resumes
            .iter()
            .map(|r| r["id"].as_str().expect("id texte"))
            .collect();
        assert!(ids.contains(&"projetscenario"));
        assert!(ids.contains(&"autreprojet"));
        let scenario = resumes
            .iter()
            .find(|r| r["id"] == "projetscenario")
            .expect("resume present");
        assert_eq!(scenario["etat"], "scenario_genere");
        assert!(scenario["maj"].as_str().expect("maj texte").len() > 10);
    }

    #[tokio::test]
    async fn get_fichier_sert_un_fichier_du_projet() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_visuels(temp.path()).await;
        let image = [0xFF, 0xD8, 0xFF, 0xD9]; // en-tete/pied JPEG minimaux
        let dossier = temp.path().join("projetscenario");
        tokio::fs::create_dir_all(&dossier)
            .await
            .expect("creation du dossier du projet");
        tokio::fs::write(dossier.join("scene-0.jpg"), image)
            .await
            .expect("ecriture de l'image");

        let reponse = app
            .oneshot(
                Request::get("/projet/projetscenario/fichier/scene-0.jpg")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        assert_eq!(
            reponse.headers()["content-type"],
            axum::http::HeaderValue::from_static("image/jpeg")
        );
        let octets = reponse
            .into_body()
            .collect()
            .await
            .expect("lecture du corps")
            .to_bytes();
        assert_eq!(&octets[..], &image);
    }

    #[tokio::test]
    async fn get_fichier_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(
                Request::get("/projet/projetscenario/fichier/absent.jpg")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_fichier_refuse_la_traversee_de_repertoire() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .clone()
            .oneshot(
                Request::get("/projet/projetscenario/fichier/..%2F..%2Fsecret.txt")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::BAD_REQUEST);

        let reponse = app
            .oneshot(
                Request::get("/projet/pas%20un%20id/fichier/scene-0.jpg")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn l_interface_est_servie_a_la_racine() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .clone()
            .oneshot(
                Request::get("/")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let html = reponse
            .into_body()
            .collect()
            .await
            .expect("lecture du corps")
            .to_bytes();
        let html = String::from_utf8(html.to_vec()).expect("html en utf-8");
        assert!(html.contains("zone-upload"));

        let reponse = app
            .clone()
            .oneshot(
                Request::get("/app.js")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        assert!(reponse.headers()["content-type"]
            .to_str()
            .expect("content-type lisible")
            .starts_with("text/javascript"));

        let reponse = app
            .oneshot(
                Request::get("/style.css")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        assert!(reponse.headers()["content-type"]
            .to_str()
            .expect("content-type lisible")
            .starts_with("text/css"));
    }

    // --- Phase 7 : affinage et suivi temps reel -----------------------------

    /// Construit une requete `POST /affiner`.
    fn requete_affinage(id: &str, etape: &str, prompt: &str) -> Request<Body> {
        Request::post("/affiner")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{ "id": "{id}", "etape": "{etape}", "prompt": "{prompt}" }}"#
            )))
            .expect("construction de la requete")
    }

    /// Scenariste factice : capture la demande recue et soumet un scenario
    /// corrige, sans aucun appel LLM (pattern d'injection des tests agents).
    struct ScenaristeFactice {
        demandes: std::sync::Mutex<Vec<String>>,
    }

    impl llm::scenariste::ExtracteurScenario for ScenaristeFactice {
        fn extraire(&self, demande: String) -> llm::scenariste::FuturScenario<'_> {
            self.demandes
                .lock()
                .expect("mutex non empoisonne")
                .push(demande);
            Box::pin(async {
                Ok(Scenario {
                    titre: "Scenario affine".to_string(),
                    public: "tout public".to_string(),
                    style_images: "photos".to_string(),
                    scenes: vec![Scene {
                        narration: "Version corrigee.".to_string(),
                        dialogues: vec![],
                        description_visuelle: "Visuel corrige".to_string(),
                        duree_cible: 6.0,
                    }],
                })
            })
        }
    }

    /// Fait passer le projet seme au bout du pipeline (publication YouTube
    /// consignee) : point de depart des tests de propagation en aval.
    async fn semer_projet_publie(data_dir: &std::path::Path) -> Projet {
        let mut projet = semer_projet_montage(data_dir).await;
        projet.etat = EtatPipeline::Publie;
        // La transcription est exigee par la regeneration du scenario.
        projet.transcription = Some(video_core::projet::Transcription {
            texte: "Un sujet dicte au telephone.".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![],
        });
        projet.youtube = Some(video_core::projet::PublicationYoutube {
            id_video: "video123".to_string(),
            url: "https://youtu.be/video123".to_string(),
        });
        let stockage = Stockage::ouvrir(data_dir).await.expect("ouverture");
        stockage.sauvegarder(&projet).await.expect("persistance");
        projet
    }

    #[tokio::test]
    async fn post_affiner_scenario_regenere_et_invalide_l_aval() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let scenariste = Arc::new(ScenaristeFactice {
            demandes: std::sync::Mutex::new(vec![]),
        });
        let app = app_de_test_avec_scenariste(temp.path().to_path_buf(), scenariste.clone()).await;
        semer_projet_publie(temp.path()).await;

        let reponse = app
            .oneshot(requete_affinage(
                "projetscenario",
                "scenario",
                "Raccourcis la video",
            ))
            .await
            .expect("reponse");
        // L'aval est invalide immediatement ; la regeneration part en tache
        // de fond (le projet est au point de reprise du scenario).
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.etat, EtatPipeline::Transcrit);
        assert_eq!(projet.validation_scenario, None);
        assert!(projet.visuels.is_empty());
        assert_eq!(projet.validation_visuels, None);
        assert!(projet.voix.is_empty());
        assert!(projet.sous_titres.is_empty());
        assert_eq!(projet.validation_voix, None);
        assert_eq!(projet.video, None);
        assert_eq!(projet.preview, None);
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);

        // Le scenario est regenere et l'etat repart a ScenarioGenere (mode
        // validation par defaut : la decision devra etre re-tranchee).
        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            p.etat == EtatPipeline::ScenarioGenere
        })
        .await;
        let scenario = projet.scenario.expect("scenario regenere");
        assert_eq!(scenario.titre, "Scenario affine");
        assert_eq!(projet.validation_scenario, None);

        // La demande au Scenariste contient le scenario actuel et la consigne.
        let demandes = scenariste.demandes.lock().expect("mutex non empoisonne");
        assert_eq!(demandes.len(), 1);
        assert!(
            demandes[0].contains("Raccourcis la video"),
            "{}",
            demandes[0]
        );
        assert!(demandes[0].contains("Sujet dicte"), "{}", demandes[0]);
    }

    #[tokio::test]
    async fn post_affiner_scenario_sans_scenariste_marque_le_projet_en_erreur() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        // app_de_test : pas de Scenariste injecte (cle API absente).
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(requete_affinage("projetscenario", "scenario", "Raccourcis"))
            .await
            .expect("reponse");
        // La reponse est immediate ; l'echec de regeneration survient dans la
        // tache de fond, qui persiste le projet en etat Erreur.
        assert_eq!(reponse.status(), StatusCode::OK);

        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            matches!(p.etat, EtatPipeline::Erreur(_))
        })
        .await;
        match &projet.etat {
            EtatPipeline::Erreur(message) => {
                assert!(message.contains("MISTRAL_API_KEY"), "{message}")
            }
            autre => panic!("un etat Erreur est attendu, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn post_affiner_refuse_une_etape_non_atteinte() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await; // pas de voix produites

        let reponse = app
            .oneshot(requete_affinage("projetscenario", "voix", "Plus lent"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);

        // Le projet est inchange.
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let projet = stockage
            .charger("projetscenario")
            .await
            .expect("chargement")
            .expect("projet present");
        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
        assert!(projet.scenario.is_some());
    }

    #[tokio::test]
    async fn post_affiner_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_affinage("inconnu123", "scenario", "Raccourcis"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    /// PNG 1x1 valide (rouge), fixture visuelle des scenes (meme fixture que
    /// les tests du Monteur).
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc,
        0xcf, 0xc0, 0x50, 0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Genere un WAV valide (tonalite 440 Hz, PCM 16 bits mono, 8 kHz) ;
    /// comme pour les tests du Monteur, pas un silence numerique (`loudnorm`
    /// produit des NaN sur un silence parfait).
    fn wav_tonalite(duree_ms: u32) -> Vec<u8> {
        let taille_donnees = duree_ms * 16;
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
        for i in 0..(duree_ms * 8) {
            let echantillon = (f64::sin(2.0 * std::f64::consts::PI * 440.0 * f64::from(i) / 8000.0)
                * 8000.0) as i16;
            wav.extend_from_slice(&echantillon.to_le_bytes());
        }
        wav
    }

    #[tokio::test]
    async fn post_affiner_montage_regenere_et_invalide_la_publication() {
        if !tools::ffmpeg::ffmpeg_disponible().await {
            eprintln!(
                "ffmpeg absent : post_affiner_montage_regenere_et_invalide_la_publication ignore."
            );
            return;
        }
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        // Projet publie, avec des fichiers courts pour un rendu rapide.
        let mut projet = semer_projet_publie(temp.path()).await;
        projet.visuels[0].fichier = "scene-0.png".to_string();
        projet.voix[0].fichier = "voix-0.wav".to_string();
        projet.voix[0].duree = 1.5;
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        stockage.sauvegarder(&projet).await.expect("persistance");
        let dossier = temp.path().join("projetscenario");
        tokio::fs::create_dir_all(&dossier)
            .await
            .expect("creation du dossier du projet");
        tokio::fs::write(dossier.join("scene-0.png"), PNG_1X1)
            .await
            .expect("ecriture de l'image");
        tokio::fs::write(dossier.join("voix-0.wav"), wav_tonalite(1500))
            .await
            .expect("ecriture de la voix");
        tokio::fs::write(
            dossier.join("sous-titres-fr.srt"),
            "1\n00:00:00,000 --> 00:00:01,500\nBonjour.\n",
        )
        .await
        .expect("ecriture du srt");

        let reponse = app
            .oneshot(requete_affinage(
                "projetscenario",
                "montage",
                "Rends la preview plus lumineuse",
            ))
            .await
            .expect("reponse");
        // L'aval (publication) est invalide immediatement ; la regeneration
        // du montage part en tache de fond.
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.etat, EtatPipeline::VoixPretes);
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);

        // Le montage est regenere (mode validation par defaut : la decision
        // devra etre re-tranchee).
        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            p.etat == EtatPipeline::MontagePret
        })
        .await;
        assert_eq!(projet.video.as_deref(), Some("video.mp4"));
        assert_eq!(projet.preview.as_deref(), Some("preview.mp4"));
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);
        // L'amont n'est pas touche.
        assert!(projet.scenario.is_some());
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
        assert_eq!(projet.visuels.len(), 1);
        assert_eq!(projet.voix.len(), 1);
        // Les rendus existent sur disque.
        assert!(dossier.join("video.mp4").exists());
        assert!(dossier.join("preview.mp4").exists());
    }

    #[tokio::test]
    async fn sse_enet_l_etat_initial_puis_les_changements() {
        use tokio_stream::StreamExt;

        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .clone()
            .oneshot(
                Request::get("/projet/projetscenario/events")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        assert_eq!(
            reponse.headers()["content-type"],
            axum::http::HeaderValue::from_static("text/event-stream")
        );
        let mut flux = reponse.into_body().into_data_stream();

        // L'etat courant est emis a l'abonnement.
        let initial = tokio::time::timeout(std::time::Duration::from_secs(5), flux.next())
            .await
            .expect("evenement initial avant le timeout")
            .expect("flux ouvert")
            .expect("trame lisible");
        let texte = String::from_utf8_lossy(&initial);
        assert!(texte.contains("event: projet"), "{texte}");
        assert!(texte.contains("scenario_genere"), "{texte}");

        // Une sauvegarde (validation) pousse un evenement aux abonnes.
        let reponse = app
            .clone()
            .oneshot(requete_validation("projetscenario", "accepte"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);

        let recu = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut accumule = String::new();
            while !accumule.contains("accepte") {
                match flux.next().await {
                    Some(Ok(trame)) => accumule.push_str(&String::from_utf8_lossy(&trame)),
                    Some(Err(e)) => panic!("trame illisible : {e}"),
                    None => break,
                }
            }
            accumule
        })
        .await
        .expect("evenement de validation recu avant le timeout");
        assert!(recu.contains("event: projet"), "{recu}");
        assert!(recu.contains("accepte"), "{recu}");
    }

    #[tokio::test]
    async fn sse_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(
                Request::get("/projet/inconnu123/events")
                    .body(Body::empty())
                    .expect("construction de la requete"),
            )
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    // --- Phase 8 : annulation et reprise -------------------------------------

    /// Construit une requete `POST /annuler` ou `POST /reprendre`.
    fn requete_projet(chemin: &str, id: &str) -> Request<Body> {
        Request::post(chemin)
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{ "id": "{id}" }}"#)))
            .expect("construction de la requete")
    }

    #[tokio::test]
    async fn post_annuler_marque_un_projet_au_repos() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        assert_eq!(projet.etat, EtatPipeline::Annule);
        // Le scenario produit est conserve pour la reprise.
        assert!(projet.scenario.is_some());

        // L'etat est bien persiste.
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let relu = stockage
            .charger("projetscenario")
            .await
            .expect("chargement")
            .expect("projet present");
        assert_eq!(relu.etat, EtatPipeline::Annule);
    }

    #[tokio::test]
    async fn post_annuler_refuse_un_projet_publie() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_publie(temp.path()).await;

        let reponse = app
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_annuler_refuse_un_projet_deja_annule() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .clone()
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);

        let reponse = app
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_annuler_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_projet("/annuler", "inconnu123"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_reprendre_replace_le_projet_a_son_point_stable() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_visuels(temp.path()).await;

        let reponse = app
            .clone()
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);

        let reponse = app
            .oneshot(requete_projet("/reprendre", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        let projet = projet_depuis(reponse).await;
        // Point de reprise derive des livrables : les visuels sont produits.
        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
        assert_eq!(projet.visuels.len(), 1);
    }

    #[tokio::test]
    async fn post_reprendre_refuse_un_projet_non_annule() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;
        semer_projet_scenario(temp.path()).await;

        let reponse = app
            .oneshot(requete_projet("/reprendre", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_reprendre_projet_inconnu_renvoie_404() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let app = app_de_test(temp.path().to_path_buf()).await;

        let reponse = app
            .oneshot(requete_projet("/reprendre", "inconnu123"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::NOT_FOUND);
    }

    /// Scenariste bloquant : le premier appel attend d'etre relache par le
    /// test, ce qui maintient une tache d'affinage en cours d'execution.
    struct ScenaristeBloquant {
        debut: Arc<tokio::sync::Notify>,
        relacher: Arc<tokio::sync::Notify>,
        appele: Arc<std::sync::atomic::AtomicBool>,
    }

    impl llm::scenariste::ExtracteurScenario for ScenaristeBloquant {
        fn extraire(&self, _demande: String) -> llm::scenariste::FuturScenario<'_> {
            let premiere_fois = !self.appele.swap(true, std::sync::atomic::Ordering::SeqCst);
            self.debut.notify_one();
            let relacher = self.relacher.clone();
            Box::pin(async move {
                if premiere_fois {
                    relacher.notified().await;
                }
                Ok(Scenario {
                    titre: "Scenario affine".to_string(),
                    public: "tout public".to_string(),
                    style_images: "photos".to_string(),
                    scenes: vec![Scene {
                        narration: "Version corrigee.".to_string(),
                        dialogues: vec![],
                        description_visuelle: "Visuel corrige".to_string(),
                        duree_cible: 6.0,
                    }],
                })
            })
        }
    }

    #[tokio::test]
    async fn post_annuler_interrompt_une_tache_en_cours() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let debut = Arc::new(tokio::sync::Notify::new());
        let relacher = Arc::new(tokio::sync::Notify::new());
        let scenariste = Arc::new(ScenaristeBloquant {
            debut: debut.clone(),
            relacher: relacher.clone(),
            appele: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let app = app_de_test_avec_scenariste(temp.path().to_path_buf(), scenariste).await;
        semer_projet_publie(temp.path()).await;

        // La tache d'affinage demarre et se bloque dans le Scenariste.
        let reponse = app
            .clone()
            .oneshot(requete_affinage("projetscenario", "scenario", "Raccourcis"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::OK);
        tokio::time::timeout(std::time::Duration::from_secs(5), debut.notified())
            .await
            .expect("la tache a demarre avant le timeout");

        // Annulation pendant que la tache tourne : 202, la tache persistera
        // `Annule` a son prochain point de controle.
        let reponse = app
            .oneshot(requete_projet("/annuler", "projetscenario"))
            .await
            .expect("reponse");
        assert_eq!(reponse.status(), StatusCode::ACCEPTED);

        // La tache se termine : le projet est persiste en etat Annule.
        relacher.notify_one();
        let projet = attendre_projet(temp.path(), "projetscenario", |p| {
            p.etat == EtatPipeline::Annule
        })
        .await;
        assert!(projet.scenario.is_some());

        // Et le projet est reprendable a son point stable : le scenario
        // regenere est produit, l'aval a ete invalide par l'affinage.
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        let relu = stockage
            .charger("projetscenario")
            .await
            .expect("chargement")
            .expect("projet present");
        assert_eq!(
            video_core::annulation::point_de_reprise(&relu),
            EtatPipeline::ScenarioGenere
        );
    }
}
