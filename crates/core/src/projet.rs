//! Types d'un projet video : fichier audio dicte, transcription, scenario.
//!
//! Un projet est persiste dans la base SQLite `data/pipeline.db` (phase 2) ;
//! ses fichiers (audio, images, voix...) vivent dans `data/<id>/`.

use serde::{Deserialize, Serialize};

use crate::etat::EtatPipeline;
use crate::scenario::Scenario;

/// Un projet video, de l'audio dicte a la publication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Projet {
    /// Identifiant unique (uuid compact), aussi nom du sous-dossier de donnees.
    pub id: String,
    /// Etat courant dans la machine a etats du pipeline.
    pub etat: EtatPipeline,
    /// Nom du fichier audio source dans le dossier du projet (ex. `audio.m4a`).
    pub audio: Option<String>,
    /// Transcription STT, presente une fois l'etat `Transcrit` atteint.
    pub transcription: Option<Transcription>,
    /// Scenario genere par le Scenariste, present une fois l'etat
    /// `ScenarioGenere` atteint.
    pub scenario: Option<Scenario>,
    /// Decision de validation humaine du scenario (`None` tant que la
    /// transition sortante, en mode `validation`, n'a pas ete tranchee).
    pub validation_scenario: Option<DecisionValidation>,
    /// Images licenciees choisies par le Visuel, une par scene, presentes une
    /// fois l'etat `VisuelsPrets` atteint.
    #[serde(default)]
    pub visuels: Vec<crate::asset::Asset>,
    /// Decision de validation humaine des visuels.
    #[serde(default)]
    pub validation_visuels: Option<DecisionValidation>,
    /// Voix off generees par le Conteur, une par scene et par langue,
    /// presentes une fois l'etat `VoixPretes` atteint.
    #[serde(default)]
    pub voix: Vec<crate::voix::VoixScene>,
    /// Fichiers de sous-titres `.srt` generes par le Conteur, un par langue.
    #[serde(default)]
    pub sous_titres: Vec<String>,
    /// Decision de validation humaine des voix.
    #[serde(default)]
    pub validation_voix: Option<DecisionValidation>,
    /// Video finale 1080p produite par le Monteur (ex. `video.mp4`), presente
    /// une fois l'etat `MontagePret` atteint.
    #[serde(default)]
    pub video: Option<String>,
    /// Preview basse resolution pour la validation humaine (ex. `preview.mp4`).
    #[serde(default)]
    pub preview: Option<String>,
    /// Decision de validation humaine du montage.
    #[serde(default)]
    pub validation_montage: Option<DecisionValidation>,
}

/// Decision prise par l'utilisateur sur une etape en mode `validation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionValidation {
    /// L'etape est acceptee, le pipeline peut avancer.
    Accepte,
    /// L'etape est refusee ; le resultat devra etre affine ou regenere.
    Rejete,
}

impl Projet {
    /// Cree un projet tout neuf, en etat `AudioRecu`.
    pub fn nouveau(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            etat: EtatPipeline::AudioRecu,
            audio: None,
            transcription: None,
            scenario: None,
            validation_scenario: None,
            visuels: Vec::new(),
            validation_visuels: None,
            voix: Vec::new(),
            sous_titres: Vec::new(),
            validation_voix: None,
            video: None,
            preview: None,
            validation_montage: None,
        }
    }
}

/// Transcription complete d'un audio : texte integral et segments horodates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcription {
    /// Texte integral de la transcription.
    pub texte: String,
    /// Langue detectee (code ISO), si fournie par le STT.
    pub langue: Option<String>,
    /// Segments horodates, dans l'ordre chronologique.
    pub segments: Vec<Segment>,
}

/// Segment horodate d'une transcription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Debut du segment, en secondes.
    pub debut: f64,
    /// Fin du segment, en secondes.
    pub fin: f64,
    /// Texte du segment.
    pub texte: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projet_nouveau_est_en_etat_audio_recu() {
        let projet = Projet::nouveau("abc123");
        assert_eq!(projet.id, "abc123");
        assert_eq!(projet.etat, EtatPipeline::AudioRecu);
        assert_eq!(projet.audio, None);
        assert_eq!(projet.transcription, None);
    }

    #[test]
    fn serialisation_projet_avec_transcription() {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::Transcrit;
        projet.audio = Some("audio.m4a".to_string());
        projet.transcription = Some(Transcription {
            texte: "Bonjour le monde.".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![Segment {
                debut: 0.0,
                fin: 1.5,
                texte: "Bonjour le monde.".to_string(),
            }],
        });

        let json = serde_json::to_string(&projet).expect("serialisation");
        let relu: Projet = serde_json::from_str(&json).expect("deserialisation");
        assert_eq!(relu, projet);
        assert_eq!(relu.etat, EtatPipeline::Transcrit);
    }
}
