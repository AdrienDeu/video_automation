//! Type `Asset` : une image licenciee associee a une scene (phase 3).
//!
//! Chaque visuel choisi par l'agent Visuel est telecharge dans le dossier du
//! projet et accompagne de tout ce qu'il faut pour l'attribution (inseree
//! dans la description YouTube en phase 6, voir `docs/architecture.md` §9).

use serde::{Deserialize, Serialize};

/// Une image licenciee telechargee pour une scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// Index de la scene illustree (0-based, position dans `Scenario.scenes`).
    pub scene: usize,
    /// Nom du fichier telecharge dans le dossier du projet (ex. `scene-0.jpg`).
    pub fichier: String,
    /// Source de l'image.
    pub source: SourceImage,
    /// Titre de l'oeuvre, si fourni par la source.
    pub titre: Option<String>,
    /// Auteur de l'oeuvre, si fourni par la source.
    pub auteur: Option<String>,
    /// URL de la page de l'oeuvre (lien d'attribution).
    pub url_page: String,
    /// URL directe du fichier image telecharge.
    pub url_fichier: String,
    /// Licence lisible (ex. `CC BY 2.0`, `CC0`, `Public domain`).
    pub licence: String,
    /// URL du texte de licence, si connue.
    pub licence_url: Option<String>,
    /// Largeur de l'image en pixels, si connue.
    pub largeur: Option<u32>,
    /// Hauteur de l'image en pixels, si connue.
    pub hauteur: Option<u32>,
}

/// Source d'une image licenciee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceImage {
    /// Agregateur Creative Commons Openverse.
    Openverse,
    /// Wikimedia Commons.
    WikimediaCommons,
}

impl Asset {
    /// Ligne d'attribution prete a inserer dans une description YouTube,
    /// ex. `« Feuille » par Jane Doe, CC BY 2.0 — https://...`.
    pub fn attribution(&self) -> String {
        let titre = self.titre.as_deref().unwrap_or("Sans titre");
        let auteur = self.auteur.as_deref().unwrap_or("auteur inconnu");
        format!(
            "« {titre} » par {auteur}, {} — {}",
            self.licence, self.url_page
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formate_l_attribution() {
        let asset = Asset {
            scene: 0,
            fichier: "scene-0.jpg".to_string(),
            source: SourceImage::Openverse,
            titre: Some("Feuille".to_string()),
            auteur: Some("Jane Doe".to_string()),
            url_page: "https://example.org/oeuvre/1".to_string(),
            url_fichier: "https://example.org/oeuvre/1.jpg".to_string(),
            licence: "CC BY 2.0".to_string(),
            licence_url: Some("https://creativecommons.org/licenses/by/2.0/".to_string()),
            largeur: Some(1024),
            hauteur: Some(768),
        };
        assert_eq!(
            asset.attribution(),
            "« Feuille » par Jane Doe, CC BY 2.0 — https://example.org/oeuvre/1"
        );
    }

    #[test]
    fn serialisation_asset() {
        let asset = Asset {
            scene: 2,
            fichier: "scene-2.png".to_string(),
            source: SourceImage::WikimediaCommons,
            titre: None,
            auteur: None,
            url_page: "https://commons.wikimedia.org/wiki/File:X.png".to_string(),
            url_fichier: "https://upload.wikimedia.org/X.png".to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur: None,
            hauteur: None,
        };
        let json = serde_json::to_string(&asset).expect("serialisation");
        let relu: Asset = serde_json::from_str(&json).expect("deserialisation");
        assert_eq!(relu, asset);
        assert_eq!(
            relu.attribution(),
            "« Sans titre » par auteur inconnu, CC0 — https://commons.wikimedia.org/wiki/File:X.png"
        );
    }
}
