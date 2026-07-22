//! Agent Scenariste : produit un `Scenario` structure a partir d'une
//! transcription, par extraction structuree (JSON schema strict, outil
//! `submit` de rig — voir `docs/architecture.md` §6).
//!
//! Le prompt systeme est versionne ici, dans la facade `llm`, conformement a
//! l'architecture (§5) : les crates clients ne voient que des fonctions.

use std::future::Future;
use std::pin::Pin;

use rig_core::client::CompletionClient;
use rig_core::completion::CompletionModel;
use rig_core::extractor::{Extractor, ExtractorBuilder};
use rig_core::providers::mistral;
use video_core::config::{LlmConfig, Provider};
use video_core::error::Error;
use video_core::projet::Transcription;
use video_core::scenario::Scenario;

/// Prompt systeme du Scenariste (v1).
///
/// Complete le preamble generique de l'extracteur de rig, qui impose
/// l'appel a l'outil `submit`.
const PREAMBLE_SCENARISTE: &str = "\
Tu es le Scenariste d'un studio de videos educatives. A partir de la \
transcription d'une note dictee, tu rediges le scenario complet d'une video \
courte et fidele au sujet : tu ne rajoutes pas de notion absente de la \
transcription.
Regles :
- 3 a 8 scenes, chacune avec une narration claire et orale, une description \
visuelle precise (elle servira de requete de recherche d'images libres de \
droits) et une duree cible realiste en secondes.
- `dialogues` reste vide sauf si la transcription met en scene plusieurs \
personnages.
- `style_images` decrit une direction visuelle coherente pour toute la video.
- Tu ecris dans la langue de la transcription.";

/// Construit l'extracteur de `Scenario` adosse a l'API Mistral, a partir
/// d'une cle API et d'un nom de modele (ex. `mistral-large-latest`).
///
/// Deux tentatives sont accordees a l'extraction : le modele peut manquer
/// l'appel a `submit` une premiere fois.
pub fn construire_extracteur_scenario(
    cle_api: &str,
    modele: &str,
) -> Result<Extractor<mistral::CompletionModel, Scenario>, Error> {
    let client = mistral::Client::new(cle_api).map_err(|e| Error::Llm(e.to_string()))?;
    Ok(client
        .extractor::<Scenario>(modele)
        .preamble(PREAMBLE_SCENARISTE)
        .retries(1)
        .build())
}

/// Construit l'extracteur de `Scenario` a partir de la configuration LLM du
/// projet. La cle API est lue dans l'environnement (`MISTRAL_API_KEY`).
///
/// # Erreurs
/// - `Error::Llm` si `MISTRAL_API_KEY` est absente de l'environnement.
/// - `Error::Config` si le provider configure n'est pas supporte (Ollama).
pub fn construire_extracteur_scenario_depuis_config(
    config_llm: &LlmConfig,
) -> Result<Extractor<mistral::CompletionModel, Scenario>, Error> {
    match config_llm.provider {
        Provider::Mistral => {
            let cle = video_core::config::cle_api_mistral().ok_or_else(|| {
                Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string())
            })?;
            construire_extracteur_scenario(&cle, &config_llm.model)
        }
        Provider::Ollama => Err(Error::config("provider Ollama non supporte en phase 2")),
    }
}

/// Construit un extracteur de `Scenario` sur un modele quelconque (utile pour
/// les tests avec un modele mocke).
pub fn extracteur_sur_modele<M: CompletionModel>(modele: M) -> Extractor<M, Scenario> {
    ExtractorBuilder::new(modele)
        .preamble(PREAMBLE_SCENARISTE)
        .retries(1)
        .build()
}

/// Futur boxe d'une extraction de scenario : rend le trait object-safe.
pub type FuturScenario<'a> = Pin<Box<dyn Future<Output = Result<Scenario, Error>> + Send + 'a>>;

/// Abstraction object-safe de l'extraction structuree de scenario.
///
/// L'`Extractor` rig concret est utilise en production ; le serveur stocke un
/// `Arc<dyn ExtracteurScenario>` ce qui permet aux tests HTTP d'injecter un
/// mock sans reseau (phase 7, `POST /affiner`).
pub trait ExtracteurScenario: Send + Sync {
    /// Extrait un `Scenario` structure d'une demande textuelle (transcription
    /// et, pour l'affinage, scenario actuel et consigne utilisateur).
    fn extraire(&self, demande: String) -> FuturScenario<'_>;
}

impl<M: CompletionModel> ExtracteurScenario for Extractor<M, Scenario> {
    fn extraire(&self, demande: String) -> FuturScenario<'_> {
        Box::pin(async move {
            self.extract(demande)
                .await
                .map_err(|e| Error::Llm(format!("extraction du scenario : {e}")))
        })
    }
}

/// Genere un scenario a partir d'une transcription STT.
///
/// Le texte integral est transmis au modele ; les segments horodates ne sont
/// pas utilises en phase 2 (le decoupage en scenes est confie au Scenariste).
///
/// # Erreurs
/// `Error::Llm` si l'extraction echoue apres les tentatives accordees.
pub async fn generer_scenario(
    extracteur: &dyn ExtracteurScenario,
    transcription: &Transcription,
) -> Result<Scenario, Error> {
    let demande = format!(
        "Voici la transcription de la note dictee :\n\n{}",
        transcription.texte
    );
    extracteur.extraire(demande).await
}

/// Regenere un scenario en integrant une consigne d'affinage de
/// l'utilisateur (phase 7, `POST /affiner`).
///
/// La transcription d'origine et le scenario actuel (en JSON) sont fournis au
/// modele avec la consigne ; le scenario corrige est extrait avec le meme
/// structured output et les memes tentatives qu'a la generation initiale.
///
/// # Erreurs
/// `Error::Llm` si la serialisation du scenario actuel ou l'extraction
/// echoue.
pub async fn affiner_scenario(
    extracteur: &dyn ExtracteurScenario,
    transcription: &Transcription,
    actuel: &Scenario,
    consigne: &str,
) -> Result<Scenario, Error> {
    let actuel_json = serde_json::to_string_pretty(actuel)
        .map_err(|e| Error::Llm(format!("serialisation du scenario actuel : {e}")))?;
    let demande = format!(
        "Voici la transcription de la note dictee :\n\n{}\n\n\
         Voici le scenario actuellement produit a partir de cette transcription :\n\n\
         {actuel_json}\n\n\
         Consigne d'affinage de l'utilisateur : {consigne}\n\n\
         Produit le scenario complet corrige en integrant cette consigne, sans \
         trahir la transcription.",
        transcription.texte
    );
    extracteur.extraire(demande).await
}
