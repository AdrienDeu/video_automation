//! Types du scenario produit par l'agent Scenariste (voir
//! `docs/architecture.md` §6).
//!
//! Ces types servent a la fois de schema de structured output pour le LLM
//! (via `schemars::JsonSchema`, version alignee sur `rig-core`) et de
//! representation persistee dans le projet. Les doc-comments alimentent le
//! JSON schema envoye au modele : ils doivent donc decrire precisement le
//! contenu attendu.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Scenario complet d'une video educative, produit par le Scenariste.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scenario {
    /// Titre de la video, accrocheur et fidele au sujet dicte.
    pub titre: String,
    /// Public vise (ex. "debutants en cuisine", "lyceens").
    pub public: String,
    /// Direction visuelle commune a toutes les scenes (ex. "photos realistes,
    /// tons chauds") : guide la recherche d'images en phase 3.
    pub style_images: String,
    /// Scenes de la video, dans l'ordre de diffusion.
    pub scenes: Vec<Scene>,
}

/// Une scene de la video : ce qui est dit et ce qui est montre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Scene {
    /// Voix off de la scene (texte integral prononce par le Conteur).
    pub narration: String,
    /// Repliques dialoguees de la scene, s'il y en a (vide sinon).
    pub dialogues: Vec<Dialogue>,
    /// Description precise de ce qui doit etre visible a l'ecran : sert de
    /// requete de recherche d'image en phase 3.
    pub description_visuelle: String,
    /// Duree cible de la scene, en secondes.
    pub duree_cible: f64,
}

/// Une replique dialoguee dans une scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Dialogue {
    /// Nom du personnage qui parle.
    pub personnage: String,
    /// Texte de la replique.
    pub replique: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialisation_scenario_complet() {
        let scenario = Scenario {
            titre: "La photosynthese".to_string(),
            public: "collegiens".to_string(),
            style_images: "photos macro, tons verts".to_string(),
            scenes: vec![Scene {
                narration: "Les plantes fabriquent leur propre nourriture.".to_string(),
                dialogues: vec![Dialogue {
                    personnage: "Prof".to_string(),
                    replique: "Comment s'appelle ce processus ?".to_string(),
                }],
                description_visuelle: "Gros plan sur une feuille verte au soleil".to_string(),
                duree_cible: 12.0,
            }],
        };

        let json = serde_json::to_string(&scenario).expect("serialisation");
        let relu: Scenario = serde_json::from_str(&json).expect("deserialisation");
        assert_eq!(relu, scenario);
    }

    #[test]
    fn le_schema_json_decrit_les_champs() {
        // Le schema part au LLM : les champs cles doivent y figurer.
        let schema =
            serde_json::to_value(schemars::schema_for!(Scenario)).expect("schema serialisable");
        let schema = schema.to_string();
        assert!(schema.contains("style_images"));
        assert!(schema.contains("description_visuelle"));
        assert!(schema.contains("duree_cible"));
    }
}
