//! Portes de validation humaines du pipeline (`POST /valider`).
//!
//! Quand une transition est en mode `validation`, le pipeline bloque jusqu'a
//! une decision explicite : acceptation (l'etape est figee) ou rejet (elle
//! devra etre affinee ou regeneree, cf. `POST /affiner` en phase 7).

use serde::{Deserialize, Serialize};
use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::{DecisionValidation, Projet};

/// Etape du pipeline soumise a validation humaine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtapeValidation {
    /// Le scenario produit par le Scenariste (phase 2).
    Scenario,
    /// Les images choisies par le Visuel (phase 3).
    Visuels,
    /// Les voix off et sous-titres produits par le Conteur (phase 4).
    Voix,
    /// Le montage produit par le Monteur (phase 5).
    Montage,
}

/// Enregistre la decision de validation d'une etape d'un projet.
///
/// # Erreurs
/// `Error::Pipeline` si le projet n'est pas dans l'etat attendu pour cette
/// etape, si le livrable de l'etape est absent, ou si l'etape a deja ete
/// tranchee.
pub fn appliquer_decision(
    projet: &mut Projet,
    etape: EtapeValidation,
    decision: DecisionValidation,
) -> Result<(), Error> {
    let (etat_attendu, livrable_present, deja_tranchee) = match etape {
        EtapeValidation::Scenario => (
            EtatPipeline::ScenarioGenere,
            projet.scenario.is_some(),
            projet.validation_scenario.is_some(),
        ),
        EtapeValidation::Visuels => (
            EtatPipeline::VisuelsPrets,
            !projet.visuels.is_empty(),
            projet.validation_visuels.is_some(),
        ),
        EtapeValidation::Voix => (
            EtatPipeline::VoixPretes,
            !projet.voix.is_empty(),
            projet.validation_voix.is_some(),
        ),
        EtapeValidation::Montage => (
            EtatPipeline::MontagePret,
            projet.video.is_some(),
            projet.validation_montage.is_some(),
        ),
    };

    if projet.etat != etat_attendu {
        return Err(Error::Pipeline(format!(
            "validation demandee sur un projet en etat {:?} (attendu : {:?})",
            projet.etat, etat_attendu
        )));
    }
    if !livrable_present {
        return Err(Error::Pipeline(format!(
            "projet en etat {etat_attendu:?} sans livrable a valider"
        )));
    }
    if deja_tranchee {
        return Err(Error::Pipeline(
            "cette etape a deja ete validee ou rejetee".to_string(),
        ));
    }

    match etape {
        EtapeValidation::Scenario => projet.validation_scenario = Some(decision),
        EtapeValidation::Visuels => projet.validation_visuels = Some(decision),
        EtapeValidation::Voix => projet.validation_voix = Some(decision),
        EtapeValidation::Montage => projet.validation_montage = Some(decision),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_core::asset::{Asset, SourceImage};
    use video_core::scenario::Scenario;

    fn projet_scenario_genere() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::ScenarioGenere;
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![],
        });
        projet
    }

    fn projet_visuels_prets() -> Projet {
        let mut projet = projet_scenario_genere();
        projet.etat = EtatPipeline::VisuelsPrets;
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
        projet
    }

    #[test]
    fn accepte_puis_bloque_toute_nouvelle_decision() {
        let mut projet = projet_scenario_genere();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Scenario,
            DecisionValidation::Accepte,
        )
        .expect("premiere decision");
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );

        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Scenario,
            DecisionValidation::Rejete,
        );
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
        // La premiere decision est conservee.
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
    }

    #[test]
    fn enregistre_un_rejet() {
        let mut projet = projet_scenario_genere();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Scenario,
            DecisionValidation::Rejete,
        )
        .expect("decision");
        assert_eq!(projet.validation_scenario, Some(DecisionValidation::Rejete));
    }

    #[test]
    fn refuse_un_projet_hors_scenario_genere() {
        let mut projet = Projet::nouveau("abc123"); // etat AudioRecu
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Scenario,
            DecisionValidation::Accepte,
        );
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("ScenarioGenere"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[test]
    fn valide_les_visuels() {
        let mut projet = projet_visuels_prets();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Visuels,
            DecisionValidation::Accepte,
        )
        .expect("decision");
        assert_eq!(projet.validation_visuels, Some(DecisionValidation::Accepte));
    }

    #[test]
    fn refuse_des_visuels_sans_images() {
        let mut projet = projet_visuels_prets();
        projet.visuels.clear();
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Visuels,
            DecisionValidation::Accepte,
        );
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
    }

    #[test]
    fn refuse_des_visuels_hors_etat() {
        let mut projet = projet_scenario_genere(); // pas encore VisuelsPrets
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Visuels,
            DecisionValidation::Accepte,
        );
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("VisuelsPrets"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    fn projet_voix_pretes() -> Projet {
        let mut projet = projet_visuels_prets();
        projet.etat = EtatPipeline::VoixPretes;
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.voix = vec![video_core::voix::VoixScene {
            scene: 0,
            langue: "fr".to_string(),
            fichier: "voix-a1b2.mp3".to_string(),
            duree: 6.0,
        }];
        projet.sous_titres = vec!["sous-titres-fr.srt".to_string()];
        projet
    }

    #[test]
    fn valide_les_voix() {
        let mut projet = projet_voix_pretes();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Voix,
            DecisionValidation::Accepte,
        )
        .expect("decision");
        assert_eq!(projet.validation_voix, Some(DecisionValidation::Accepte));
    }

    #[test]
    fn refuse_des_voix_sans_audio() {
        let mut projet = projet_voix_pretes();
        projet.voix.clear();
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Voix,
            DecisionValidation::Accepte,
        );
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
    }

    #[test]
    fn refuse_des_voix_hors_etat() {
        let mut projet = projet_visuels_prets(); // pas encore VoixPretes
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Voix,
            DecisionValidation::Accepte,
        );
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("VoixPretes"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    fn projet_montage_pret() -> Projet {
        let mut projet = projet_voix_pretes();
        projet.etat = EtatPipeline::MontagePret;
        projet.validation_voix = Some(DecisionValidation::Accepte);
        projet.video = Some("video.mp4".to_string());
        projet.preview = Some("preview.mp4".to_string());
        projet
    }

    #[test]
    fn valide_le_montage() {
        let mut projet = projet_montage_pret();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Montage,
            DecisionValidation::Accepte,
        )
        .expect("decision");
        assert_eq!(projet.validation_montage, Some(DecisionValidation::Accepte));
    }

    #[test]
    fn refuse_un_montage_sans_video() {
        let mut projet = projet_montage_pret();
        projet.video = None;
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Montage,
            DecisionValidation::Accepte,
        );
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
    }

    #[test]
    fn refuse_un_montage_hors_etat() {
        let mut projet = projet_voix_pretes(); // pas encore MontagePret
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Montage,
            DecisionValidation::Accepte,
        );
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("MontagePret"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[test]
    fn bloque_une_seconde_decision_sur_le_montage() {
        let mut projet = projet_montage_pret();
        appliquer_decision(
            &mut projet,
            EtapeValidation::Montage,
            DecisionValidation::Rejete,
        )
        .expect("premiere decision");
        let resultat = appliquer_decision(
            &mut projet,
            EtapeValidation::Montage,
            DecisionValidation::Accepte,
        );
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
        assert_eq!(projet.validation_montage, Some(DecisionValidation::Rejete));
    }
}
