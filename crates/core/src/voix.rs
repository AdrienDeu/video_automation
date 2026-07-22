//! Types des voix off produites par l'agent Conteur (phase 4, voir
//! `docs/architecture.md` §6-7).
//!
//! Chaque scene du scenario est doublee par l'outil `generer_voix` : un
//! fichier audio par scene et par langue, dont la duree reelle (mesuree via
//! ffprobe) sert a synchroniser les sous-titres `.srt`.

use serde::{Deserialize, Serialize};

/// La voix off d'une scene, dans une langue donnee.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoixScene {
    /// Index de la scene doublee (0-based, position dans `Scenario.scenes`).
    pub scene: usize,
    /// Langue du doublage (code ISO, ex. `fr`).
    pub langue: String,
    /// Nom du fichier audio dans le dossier du projet (ex. `voix-a1b2....mp3`).
    pub fichier: String,
    /// Duree reelle de l'audio en secondes (mesuree via ffprobe ; a defaut,
    /// la duree cible de la scene).
    pub duree: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialisation_voix_scene() {
        let voix = VoixScene {
            scene: 1,
            langue: "fr".to_string(),
            fichier: "voix-a1b2c3.mp3".to_string(),
            duree: 7.5,
        };
        let json = serde_json::to_string(&voix).expect("serialisation");
        let relu: VoixScene = serde_json::from_str(&json).expect("deserialisation");
        assert_eq!(relu, voix);
    }
}
