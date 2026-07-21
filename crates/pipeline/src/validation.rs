//! Porte de validation humaine du scenario (`POST /valider`).
//!
//! Quand la transition sortant de `ScenarioGenere` est en mode `validation`,
//! le pipeline bloque jusqu'a une decision explicite : acceptation (le
//! scenario est fige) ou rejet (il devra etre affine ou regenere, cf.
//! `POST /affiner` en phase 7).

use video_core::error::Error;
use video_core::etat::EtatPipeline;
use video_core::projet::{DecisionValidation, Projet};

/// Enregistre la decision de validation du scenario d'un projet.
///
/// # Erreurs
/// `Error::Pipeline` si le projet n'est pas en etat `ScenarioGenere`, s'il
/// n'a pas de scenario, ou si le scenario a deja ete tranche.
pub fn appliquer_decision_scenario(
    projet: &mut Projet,
    decision: DecisionValidation,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::ScenarioGenere {
        return Err(Error::Pipeline(format!(
            "validation demandee sur un projet en etat {:?} (attendu : ScenarioGenere)",
            projet.etat
        )));
    }
    if projet.scenario.is_none() {
        return Err(Error::Pipeline(
            "projet en etat ScenarioGenere sans scenario".to_string(),
        ));
    }
    if projet.validation_scenario.is_some() {
        return Err(Error::Pipeline(
            "le scenario de ce projet a deja ete valide ou rejete".to_string(),
        ));
    }
    projet.validation_scenario = Some(decision);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn accepte_puis_bloque_toute_nouvelle_decision() {
        let mut projet = projet_scenario_genere();
        appliquer_decision_scenario(&mut projet, DecisionValidation::Accepte)
            .expect("premiere decision");
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );

        let resultat = appliquer_decision_scenario(&mut projet, DecisionValidation::Rejete);
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
        appliquer_decision_scenario(&mut projet, DecisionValidation::Rejete).expect("decision");
        assert_eq!(projet.validation_scenario, Some(DecisionValidation::Rejete));
    }

    #[test]
    fn refuse_un_projet_hors_scenario_genere() {
        let mut projet = Projet::nouveau("abc123"); // etat AudioRecu
        let resultat = appliquer_decision_scenario(&mut projet, DecisionValidation::Accepte);
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("attendu : ScenarioGenere"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }
}
