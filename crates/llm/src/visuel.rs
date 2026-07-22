//! Agent Visuel : choisit une image licenciee par scene via l'outil rig
//! `choisir_image` (phase 3, voir `docs/architecture.md` §6-7).
//!
//! L'outil est declare a rig ici, dans la facade `llm`, comme le hello-world
//! de la phase 0 ; la recherche et le telechargement effectifs vivent dans
//! `tools::images`, testables sans LLM.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rig_core::tool::Tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use video_core::asset::Asset;
use video_core::config::{LlmConfig, Provider};
use video_core::error::Error;

use crate::client;

/// Prompt systeme du Visuel (v1).
const PREAMBLE_VISUEL: &str = "\
Tu es le Visuel d'un studio de videos educatives : tu illustres chaque scene \
avec une image libre de droits.
Pour chaque scene qui t'est presentee, tu appelles l'outil choisir_image UNE \
SEULE FOIS, avec :
- `requete` : 2 a 5 mots-cles EN ANGLAIS, concrets et visuels (les banques \
d'images sont indexees en anglais), traduits ou adaptes de la description de \
la scene ;
- `scene_id` : l'index exact de la scene ;
- `style` : le style visuel commun de la video, en mots-cles anglais.
Tu ne commentes pas, tu ne proposes pas d'alternative : tu appelles l'outil.";

/// Assets choisis par l'outil, partages avec l'appelant : le resultat d'un
/// outil rig revient au modele, pas a l'orchestrateur — ce collecteur permet
/// de recuperer concretement les `Asset` apres la boucle agent/outil.
pub type ImagesChoisies = Arc<Mutex<Vec<Asset>>>;

/// Arguments de l'outil `choisir_image` (voir `docs/architecture.md` §7).
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArgsChoisirImage {
    /// Requete de recherche : 2 a 5 mots-cles en anglais, concrets et visuels.
    pub requete: String,
    /// Index de la scene a illustrer (0-based).
    pub scene_id: u32,
    /// Style visuel commun de la video, en mots-cles anglais : affine la
    /// pertinence du choix.
    pub style: String,
}

/// Outil rig `choisir_image` : recherche et telecharge une image licenciee
/// dans le dossier du projet, puis consigne l'`Asset` dans le collecteur
/// partage.
#[derive(Clone)]
pub struct ChoisirImage {
    http: reqwest::Client,
    dossier: PathBuf,
    choisies: ImagesChoisies,
}

impl ChoisirImage {
    /// Cree l'outil pour un projet : `dossier` est le dossier de donnees du
    /// projet (`data/<id>/`), `choisies` le collecteur partage des assets.
    pub fn nouveau(dossier: PathBuf, choisies: ImagesChoisies) -> Result<Self, Error> {
        Ok(Self {
            http: tools::images::client_http()?,
            dossier,
            choisies,
        })
    }
}

impl Tool for ChoisirImage {
    const NAME: &'static str = "choisir_image";

    type Error = Error;
    type Args = ArgsChoisirImage;
    type Output = String;

    fn description(&self) -> String {
        "Recherche une image libre de droits (Openverse, Wikimedia Commons), \
         la telecharge dans le dossier du projet et retourne son attribution."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(ArgsChoisirImage))
            .expect("le schema des arguments doit etre serialisable")
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let asset = tools::images::choisir_image(
            &self.http,
            &self.dossier,
            args.scene_id as usize,
            &args.requete,
            &args.style,
        )
        .await?;
        let resume = format!(
            "Image choisie pour la scene {} : {} ({})",
            args.scene_id,
            asset.titre.as_deref().unwrap_or("sans titre"),
            asset.licence
        );
        self.choisies
            .lock()
            .expect("mutex non empoisonne")
            .push(asset);
        Ok(resume)
    }
}

/// Agent Visuel concret (modele Mistral), tel que construit par
/// [`construire_agent_visuel`] : alias public pour que les crates clients
/// nomment le type sans dependre de `rig-core`.
pub type AgentVisuel = rig_core::agent::Agent<rig_core::providers::mistral::CompletionModel>;

/// Construit l'agent Visuel adosse a l'API Mistral, muni de l'outil
/// `choisir_image` et du prompt dedie.
pub fn construire_agent_visuel(
    cle_api: &str,
    modele: &str,
    outil: ChoisirImage,
) -> Result<AgentVisuel, Error> {
    Ok(client::construire_agent_mistral(cle_api, modele)?
        .preamble(PREAMBLE_VISUEL)
        .tool(outil)
        .build())
}

/// Construit l'agent Visuel a partir de la configuration LLM du projet.
///
/// # Erreurs
/// - `Error::Llm` si `MISTRAL_API_KEY` est absente de l'environnement.
/// - `Error::Config` si le provider configure n'est pas supporte (Ollama).
pub fn construire_agent_visuel_depuis_config(
    config_llm: &LlmConfig,
    outil: ChoisirImage,
) -> Result<AgentVisuel, Error> {
    match config_llm.provider {
        Provider::Mistral => {
            let cle = video_core::config::cle_api_mistral().ok_or_else(|| {
                Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string())
            })?;
            construire_agent_visuel(&cle, &config_llm.model, outil)
        }
        Provider::Ollama => Err(Error::config("provider Ollama non supporte en phase 3")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_outil_expose_son_schema() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let outil = ChoisirImage::nouveau(temp.path().to_path_buf(), Arc::new(Mutex::new(vec![])))
            .expect("construction de l'outil");
        let schema = outil.parameters();
        assert_eq!(outil.name(), "choisir_image");
        assert!(schema["properties"]["requete"].is_object());
        assert!(schema["properties"]["scene_id"].is_object());
        assert!(schema["properties"]["style"].is_object());
    }
}
