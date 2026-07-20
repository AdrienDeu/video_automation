//! Construction des agents rig a partir de la configuration du projet.

use rig_core::agent::AgentBuilder;
use rig_core::client::CompletionClient;
use rig_core::providers::mistral;
use video_core::config::{self, LlmConfig, Provider};
use video_core::error::Error;

/// Agent rig Mistral en cours de construction.
///
/// L'appelant complete la construction (`.preamble(...)`, `.tool(...)`) puis
/// termine par `.build()`.
pub type ConstructeurAgentMistral = AgentBuilder<mistral::CompletionModel>;

/// Construit un agent rig adosse a l'API Mistral, a partir d'une cle API et
/// d'un nom de modele (ex. `mistral-large-latest`).
///
/// Retourne le constructeur d'agent : a l'appelant d'y ajouter les outils et
/// le preamble (prompt systeme) avant `.build()`.
pub fn construire_agent_mistral(
    cle_api: &str,
    modele: &str,
) -> Result<ConstructeurAgentMistral, Error> {
    let client = mistral::Client::new(cle_api).map_err(|e| Error::Llm(e.to_string()))?;
    Ok(client.agent(modele))
}

/// Construit un agent a partir de la configuration LLM du projet.
///
/// La cle API est lue dans l'environnement (`MISTRAL_API_KEY`), jamais dans
/// le fichier de configuration.
///
/// # Erreurs
/// - `Error::Llm` si `MISTRAL_API_KEY` est absente de l'environnement.
/// - `Error::Config` si le provider configure n'est pas supporte en phase 0
///   (Ollama).
pub fn construire_agent_depuis_config(
    config_llm: &LlmConfig,
) -> Result<ConstructeurAgentMistral, Error> {
    match config_llm.provider {
        Provider::Mistral => {
            let cle = config::cle_api_mistral().ok_or_else(|| {
                Error::Llm("MISTRAL_API_KEY absente de l'environnement".to_string())
            })?;
            construire_agent_mistral(&cle, &config_llm.model)
        }
        Provider::Ollama => Err(Error::config("provider Ollama non supporte en phase 0")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_refuse_en_phase_0() {
        let config_llm = LlmConfig {
            provider: Provider::Ollama,
            model: "llama3.1".to_string(),
            ollama_url: None,
        };
        // Pas d'`expect_err` : le constructeur d'agent n'implemente pas Debug.
        let resultat = construire_agent_depuis_config(&config_llm);
        match resultat {
            Err(erreur) => {
                assert!(matches!(erreur, Error::Config(_)));
                assert!(erreur.to_string().contains("non supporte en phase 0"));
            }
            Ok(_) => panic!("Ollama doit etre refuse en phase 0"),
        }
    }
}
