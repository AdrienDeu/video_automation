//! Machine a etats du pipeline (voir `docs/architecture.md` §8).

use serde::{Deserialize, Serialize};

/// Etat d'avancement d'une video dans le pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtatPipeline {
    /// Le fichier audio a ete recu par le serveur.
    AudioRecu,
    /// La transcription STT est disponible.
    Transcrit,
    /// Le scenario a ete genere par l'agent Scenariste.
    ScenarioGenere,
    /// Les visuels licencies sont prets.
    VisuelsPrets,
    /// Les voix off (et sous-titres) sont pretes.
    VoixPretes,
    /// Le montage ffmpeg est termine.
    MontagePret,
    /// La video est publiee sur YouTube.
    Publie,
    /// Une etape a echoue ; le detail est conserve pour le Realisateur.
    Erreur(String),
}

/// Mode de transition entre deux etats du pipeline.
///
/// Defaut prudent : `Validation` (relecture humaine).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeTransition {
    /// La transition s'enchaine sans intervention humaine.
    Auto,
    /// Le pipeline bloque en attente d'une validation utilisateur.
    #[default]
    Validation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialisation_etat() {
        let etat = EtatPipeline::ScenarioGenere;
        let json = serde_json::to_string(&etat).expect("serialisation");
        assert_eq!(json, r#""scenario_genere""#);

        let erreur = EtatPipeline::Erreur("API injoignable".to_string());
        let json = serde_json::to_string(&erreur).expect("serialisation");
        let relu: EtatPipeline = serde_json::from_str(&json).expect("deserialisation");
        assert_eq!(relu, erreur);
    }

    #[test]
    fn serialisation_mode() {
        let json = serde_json::to_string(&ModeTransition::Validation).expect("serialisation");
        assert_eq!(json, r#""validation""#);
    }
}
