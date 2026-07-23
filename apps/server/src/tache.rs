//! Taches de fond du pipeline (phase 8) : transcription, enchainement des
//! etapes et regeneration d'un affinage s'executent hors de la requete HTTP,
//! interruptibles a tout moment via un `CancellationToken` par projet.
//!
//! Le token est inscrit dans `AppState::taches` avant le lancement (un seul
//! pipeline actif par projet : un second lancement est ignore) et retire en
//! fin de tache. `POST /annuler` declenche le token ; la tache s'interrompt
//! a son prochain point de controle (`video_core::annulation`) et persiste le
//! projet en `EtatPipeline::Annule` — reprendable via `POST /reprendre`,
//! l'etat de reprise etant derive des livrables deja produits.
//!
//! Toute autre erreur d'etape persiste le projet en `EtatPipeline::Erreur` :
//! les POST declencheurs (`/audio`, `/valider`, `/affiner`) ne renvoient plus
//! de 502, l'echec est suivi via `GET /projet/{id}` ou le flux SSE.

use std::sync::Arc;

use pipeline::validation::EtapeValidation;
use video_core::annulation::CancellationToken;
use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::Projet;

use crate::{handlers, AppState};

/// Travail demande a la tache de fond d'un projet.
pub enum Demande {
    /// Transcription STT de l'audio recu, puis enchainement du pipeline.
    Transcription {
        /// Langue de l'audio, si l'utilisateur l'a precisee a l'envoi.
        langue: Option<String>,
    },
    /// Enchainement du pipeline depuis l'etat courant (acceptation d'une
    /// etape en mode validation, ou reprise apres annulation).
    Pipeline,
    /// Regeneration d'une etape avec sa consigne (l'aval a deja ete invalide
    /// par le handler), puis enchainement du pipeline.
    Affinage {
        /// Etape a regenerer.
        etape: EtapeValidation,
        /// Consigne d'affinage de l'utilisateur.
        prompt: String,
    },
}

/// Lance la tache de fond d'un projet ; sans effet si une tache est deja en
/// cours pour ce projet.
pub fn lancer_pipeline(etat: &Arc<AppState>, id: &str, demande: Demande) {
    let token = CancellationToken::new();
    {
        let mut taches = etat.taches.lock().expect("mutex non empoisonne");
        if taches.contains_key(id) {
            return;
        }
        taches.insert(id.to_string(), token.clone());
    }
    let etat = etat.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        executer(&etat, &id, demande, &token).await;
        etat.taches
            .lock()
            .expect("mutex non empoisonne")
            .remove(&id);
    });
}

/// Corps de la tache : execute le travail demande puis persiste l'etat final
/// (`Annule` si l'annulation a ete demandee, `Erreur` en cas d'echec, l'etat
/// atteint sinon) et notifie les abonnes SSE.
async fn executer(etat: &AppState, id: &str, demande: Demande, token: &CancellationToken) {
    let mut projet = match etat.stockage.charger(id).await {
        Ok(Some(projet)) => projet,
        Ok(None) => return, // projet supprime entre-temps
        Err(e) => {
            eprintln!("tache du projet {id} : chargement impossible : {e}");
            return;
        }
    };

    let resultat = match demande {
        Demande::Transcription { langue } => {
            match transcrire(etat, &mut projet, langue.as_deref()).await {
                Ok(()) => handlers::avancer_pipeline(etat, &mut projet, token).await,
                Err(erreur) => Err(erreur),
            }
        }
        Demande::Pipeline => handlers::avancer_pipeline(etat, &mut projet, token).await,
        Demande::Affinage { etape, prompt } => {
            match handlers::regenerer_etape(etat, &mut projet, etape, &prompt, token).await {
                Ok(()) => handlers::avancer_pipeline(etat, &mut projet, token).await,
                Err(erreur) => Err(erreur),
            }
        }
    };

    match resultat {
        // Une etape a pu aboutir apres la demande d'annulation (point de
        // controle suivant non atteint) : l'annulation prime.
        Ok(()) if token.is_cancelled() => projet.etat = EtatPipeline::Annule,
        Ok(()) => {}
        Err(Error::Annulation) => projet.etat = EtatPipeline::Annule,
        Err(erreur) => projet.etat = EtatPipeline::Erreur(erreur.to_string()),
    }
    if let Err((_, message)) = handlers::sauvegarder_et_notifier(etat, &projet).await {
        eprintln!("tache du projet {id} : persistance impossible : {message}");
    }
}

/// Transcrit l'audio du projet (etat `AudioRecu` → `Transcrit`) et persiste
/// ce point d'etape ; sans effet si la transcription existe deja (reprise).
async fn transcrire(
    etat: &AppState,
    projet: &mut Projet,
    langue: Option<&str>,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::AudioRecu {
        return Ok(());
    }
    let cle = etat
        .cle_api
        .as_deref()
        .ok_or_else(|| Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string()))?;
    let nom = projet
        .audio
        .clone()
        .ok_or_else(|| Error::Pipeline("projet sans fichier audio".to_string()))?;
    let chemin = etat.config.data_dir.join(&projet.id).join(nom);
    let transcription = tools::transcrire::transcrire_audio(&chemin, langue, cle).await?;
    projet.transcription = Some(transcription);
    projet.etat = EtatPipeline::Transcrit;
    handlers::sauvegarder_et_notifier(etat, projet)
        .await
        .map_err(|(_, message)| Error::Persistance(message))?;
    Ok(())
}
