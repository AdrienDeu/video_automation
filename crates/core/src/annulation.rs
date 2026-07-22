//! Annulation du pipeline : point de controle dans les boucles longues et
//! derivation du point de reprise d'un projet interrompu.
//!
//! Le serveur associe un `CancellationToken` a chaque tache de pipeline en
//! cours (`POST /annuler` le declenche) ; les etapes longues appellent
//! [`point_de_controle`] entre deux unites de travail (scenes, passes ffmpeg,
//! chunks d'upload) et s'interrompent avec [`Error::Annulation`].
//!
//! Un projet interrompu est persiste en [`EtatPipeline::Annule`] ; comme les
//! agents ne font avancer `etat` qu'en cas de succes complet, les livrables
//! deja produits font foi pour la reprise : [`point_de_reprise`] en derive
//! l'etat dans lequel replacer le projet (`POST /reprendre`).

use crate::error::Error;
use crate::etat::EtatPipeline;
use crate::projet::Projet;

// Reexporte pour que les crates du workspace nomment le type sans dependre
// directement de `tokio-util`.
pub use tokio_util::sync::CancellationToken;

/// Verifie qu'aucune annulation n'a ete demandee ; a appeler entre deux
/// unites de travail d'une etape longue.
///
/// # Erreurs
/// `Error::Annulation` si le token a ete annule (`POST /annuler`).
pub fn point_de_controle(token: &CancellationToken) -> Result<(), Error> {
    if token.is_cancelled() {
        return Err(Error::Annulation);
    }
    Ok(())
}

/// Derive l'etat de reprise d'un projet annule de ses livrables deja
/// produits : le dernier livrable present fait foi, comme dans
/// `pipeline::affiner::reinitialiser_aval`.
pub fn point_de_reprise(projet: &Projet) -> EtatPipeline {
    if projet.youtube.is_some() {
        EtatPipeline::Publie
    } else if projet.video.is_some() {
        EtatPipeline::MontagePret
    } else if !projet.voix.is_empty() {
        EtatPipeline::VoixPretes
    } else if !projet.visuels.is_empty() {
        EtatPipeline::VisuelsPrets
    } else if projet.scenario.is_some() {
        EtatPipeline::ScenarioGenere
    } else if projet.transcription.is_some() {
        EtatPipeline::Transcrit
    } else {
        EtatPipeline::AudioRecu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projet::PublicationYoutube;
    use crate::scenario::{Scenario, Scene};
    use crate::voix::VoixScene;

    #[test]
    fn le_point_de_controle_laisse_passer_un_token_intact() {
        let token = CancellationToken::new();
        assert!(point_de_controle(&token).is_ok());
    }

    #[test]
    fn le_point_de_controle_bloque_un_token_annule() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(matches!(point_de_controle(&token), Err(Error::Annulation)));
    }

    #[test]
    fn la_reprise_se_place_au_dernier_livrable_present() {
        let mut projet = Projet::nouveau("abc123");
        assert_eq!(point_de_reprise(&projet), EtatPipeline::AudioRecu);

        projet.transcription = Some(crate::projet::Transcription {
            texte: "Bonjour".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![],
        });
        assert_eq!(point_de_reprise(&projet), EtatPipeline::Transcrit);

        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![Scene {
                narration: "Narration.".to_string(),
                dialogues: vec![],
                description_visuelle: "Visuel".to_string(),
                duree_cible: 8.0,
            }],
        });
        assert_eq!(point_de_reprise(&projet), EtatPipeline::ScenarioGenere);

        projet.voix = vec![VoixScene {
            scene: 0,
            langue: "fr".to_string(),
            fichier: "voix-a1b2.mp3".to_string(),
            duree: 6.0,
        }];
        assert_eq!(point_de_reprise(&projet), EtatPipeline::VoixPretes);

        projet.video = Some("video.mp4".to_string());
        assert_eq!(point_de_reprise(&projet), EtatPipeline::MontagePret);

        projet.youtube = Some(PublicationYoutube {
            id_video: "video123".to_string(),
            url: "https://youtu.be/video123".to_string(),
        });
        assert_eq!(point_de_reprise(&projet), EtatPipeline::Publie);
    }
}
