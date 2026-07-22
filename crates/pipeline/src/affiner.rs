//! Regeneration ciblee d'une etape avec propagation en aval (phase 7,
//! `POST /affiner`, voir `docs/agenda.md`).
//!
//! Affiner une etape invalide tout ce qui en decoule : les artefacts et les
//! decisions de validation des etapes strictement en aval sont effaces, la
//! validation de l'etape affinee est remise a `None` (elle devra etre
//! re-tranchee si sa porte est en mode `validation`) et l'etat du projet est
//! replace au point de reprise — l'etat d'entree de l'agent producteur, ce
//! qui permet a `avancer_pipeline` d'enchainer comme d'habitude.
//!
//! Les fichiers de `data/<id>/` ne sont PAS supprimes : les voix sont nommees
//! par hash de contenu (le cache TTS est reutilise si le texte ne change
//! pas), les rendus portent des noms constants (`video.mp4`, `preview.mp4`)
//! ecrases a la regeneration, et les images telechargees peuvent etre
//! rechoisies. D'eventuels fichiers devenus orphelins restent sur disque.

use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::Projet;

use crate::validation::EtapeValidation;

/// Prepare un projet a la regeneration de `etape` : invalide les artefacts et
/// validations des etapes strictement en aval, remet la validation de l'etape
/// a `None` et replace `etat` au point de reprise (`Transcrit` pour le
/// scenario, `ScenarioGenere` pour les visuels, `VisuelsPrets` pour les voix,
/// `VoixPretes` pour le montage).
///
/// Les validations des etapes strictement en amont sont conservees : leurs
/// portes restent ouvertes pour la regeneration.
///
/// # Erreurs
/// `Error::Pipeline` si le projet n'a pas encore atteint l'etape demandee
/// (livrable de l'etape absent).
pub fn reinitialiser_aval(projet: &mut Projet, etape: EtapeValidation) -> Result<(), Error> {
    let livrable_present = match etape {
        EtapeValidation::Scenario => projet.scenario.is_some(),
        EtapeValidation::Visuels => !projet.visuels.is_empty(),
        EtapeValidation::Voix => !projet.voix.is_empty(),
        EtapeValidation::Montage => projet.video.is_some(),
    };
    if !livrable_present {
        return Err(Error::Pipeline(format!(
            "affinage de l'etape {etape:?} demande sur un projet en etat {:?} \
             qui n'a pas encore atteint cette etape",
            projet.etat
        )));
    }

    match etape {
        EtapeValidation::Scenario => {
            projet.validation_scenario = None;
            projet.visuels.clear();
            projet.validation_visuels = None;
            projet.voix.clear();
            projet.sous_titres.clear();
            projet.validation_voix = None;
            projet.video = None;
            projet.preview = None;
            projet.validation_montage = None;
            projet.youtube = None;
            projet.etat = EtatPipeline::Transcrit;
        }
        EtapeValidation::Visuels => {
            projet.validation_visuels = None;
            projet.voix.clear();
            projet.sous_titres.clear();
            projet.validation_voix = None;
            projet.video = None;
            projet.preview = None;
            projet.validation_montage = None;
            projet.youtube = None;
            projet.etat = EtatPipeline::ScenarioGenere;
        }
        EtapeValidation::Voix => {
            projet.validation_voix = None;
            projet.video = None;
            projet.preview = None;
            projet.validation_montage = None;
            projet.youtube = None;
            projet.etat = EtatPipeline::VisuelsPrets;
        }
        EtapeValidation::Montage => {
            projet.validation_montage = None;
            projet.youtube = None;
            projet.etat = EtatPipeline::VoixPretes;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_core::asset::{Asset, SourceImage};
    use video_core::projet::{DecisionValidation, PublicationYoutube};
    use video_core::scenario::{Scenario, Scene};
    use video_core::voix::VoixScene;

    /// Projet arrive au bout du pipeline : scenario, visuels, voix, montage
    /// et publication, toutes les portes acceptees.
    fn projet_publie() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::Publie;
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![Scene {
                narration: "Voici le sujet.".to_string(),
                dialogues: vec![],
                description_visuelle: "Visuel 0".to_string(),
                duree_cible: 8.0,
            }],
        });
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.visuels = vec![Asset {
            scene: 0,
            fichier: "scene-0.jpg".to_string(),
            source: SourceImage::Openverse,
            titre: None,
            auteur: None,
            url_page: "https://example.org/oeuvre".to_string(),
            url_fichier: "https://example.org/oeuvre.jpg".to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur: None,
            hauteur: None,
        }];
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.voix = vec![VoixScene {
            scene: 0,
            langue: "fr".to_string(),
            fichier: "voix-a1b2.mp3".to_string(),
            duree: 6.0,
        }];
        projet.sous_titres = vec!["sous-titres-fr.srt".to_string()];
        projet.validation_voix = Some(DecisionValidation::Accepte);
        projet.video = Some("video.mp4".to_string());
        projet.preview = Some("preview.mp4".to_string());
        projet.validation_montage = Some(DecisionValidation::Accepte);
        projet.youtube = Some(PublicationYoutube {
            id_video: "video123".to_string(),
            url: "https://youtu.be/video123".to_string(),
        });
        projet
    }

    #[test]
    fn affiner_le_scenario_invalide_tout_l_aval() {
        let mut projet = projet_publie();
        reinitialiser_aval(&mut projet, EtapeValidation::Scenario).expect("reinitialisation");

        // Point de reprise : l'entree du Scenariste ; le scenario actuel est
        // conserve (il sert a l'affinage) mais devra etre re-valide.
        assert_eq!(projet.etat, EtatPipeline::Transcrit);
        assert!(projet.scenario.is_some());
        assert_eq!(projet.validation_scenario, None);
        // Tout l'aval est invalide.
        assert!(projet.visuels.is_empty());
        assert_eq!(projet.validation_visuels, None);
        assert!(projet.voix.is_empty());
        assert!(projet.sous_titres.is_empty());
        assert_eq!(projet.validation_voix, None);
        assert_eq!(projet.video, None);
        assert_eq!(projet.preview, None);
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);
    }

    #[test]
    fn affiner_les_visuels_preserve_le_scenario() {
        let mut projet = projet_publie();
        reinitialiser_aval(&mut projet, EtapeValidation::Visuels).expect("reinitialisation");

        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
        // L'amont est intact, porte du scenario toujours ouverte.
        assert!(projet.scenario.is_some());
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
        // Les visuels sont conserves jusqu'a leur regeneration mais la
        // decision est effacee.
        assert!(!projet.visuels.is_empty());
        assert_eq!(projet.validation_visuels, None);
        // L'aval est invalide.
        assert!(projet.voix.is_empty());
        assert!(projet.sous_titres.is_empty());
        assert_eq!(projet.video, None);
        assert_eq!(projet.youtube, None);
    }

    #[test]
    fn affiner_les_voix_preserve_scenario_et_visuels() {
        let mut projet = projet_publie();
        reinitialiser_aval(&mut projet, EtapeValidation::Voix).expect("reinitialisation");

        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
        // Scenario et visuels intacts.
        assert!(projet.scenario.is_some());
        assert!(!projet.visuels.is_empty());
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
        assert_eq!(projet.validation_visuels, Some(DecisionValidation::Accepte));
        // Voix conservees jusqu'a regeneration, decision effacee.
        assert!(!projet.voix.is_empty());
        assert!(!projet.sous_titres.is_empty());
        assert_eq!(projet.validation_voix, None);
        // Montage et publication invalides.
        assert_eq!(projet.video, None);
        assert_eq!(projet.preview, None);
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);
    }

    #[test]
    fn affiner_le_montage_n_invalide_que_la_publication() {
        let mut projet = projet_publie();
        reinitialiser_aval(&mut projet, EtapeValidation::Montage).expect("reinitialisation");

        assert_eq!(projet.etat, EtatPipeline::VoixPretes);
        assert!(!projet.voix.is_empty());
        assert_eq!(projet.validation_voix, Some(DecisionValidation::Accepte));
        assert_eq!(projet.validation_montage, None);
        assert_eq!(projet.youtube, None);
        // Le scenario et les visuels ne sont pas touches.
        assert!(projet.scenario.is_some());
        assert!(!projet.visuels.is_empty());
    }

    #[test]
    fn refuse_une_etape_non_atteinte() {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::ScenarioGenere;
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![],
        });
        // Pas de voix produites : l'affinage des voix est refuse.
        let resultat = reinitialiser_aval(&mut projet, EtapeValidation::Voix);
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
        // Le projet n'a pas ete modifie.
        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
        assert!(projet.scenario.is_some());
    }
}
