//! Chargement de la configuration (fichier TOML + variables d'environnement).
//!
//! Les valeurs **non secretes** vivent dans `config.toml` a la racine du
//! projet et peuvent etre surchargees par des variables d'environnement
//! prefixees `VIDEO_AUTOMATION_`. Les **secrets** (cles API) ne passent que
//! par l'environnement, jamais par le fichier de configuration.

use std::path::PathBuf;

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Configuration racine de l'application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Dossier de donnees (un sous-dossier par video).
    pub data_dir: PathBuf,
    /// Adresse d'ecoute du serveur HTTP, ex. `127.0.0.1:8080`.
    pub server_addr: String,
    /// Configuration du fournisseur LLM.
    pub llm: LlmConfig,
}

/// Configuration du fournisseur LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Fournisseur retenu (Mistral en phase 0).
    pub provider: Provider,
    /// Nom du modele, ex. `mistral-large-latest`.
    pub model: String,
    /// URL du serveur Ollama local (inutilise en phase 0).
    pub ollama_url: Option<String>,
}

/// Fournisseur de LLM interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// API hebergee Mistral.
    Mistral,
    /// LLM local via Ollama (non supporte en phase 0).
    Ollama,
}

impl Config {
    /// Valeurs par defaut, identiques au `config.toml` du depot.
    fn defaut() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            server_addr: "127.0.0.1:8080".to_string(),
            llm: LlmConfig {
                provider: Provider::Mistral,
                model: "mistral-large-latest".to_string(),
                ollama_url: None,
            },
        }
    }

    /// Charge la configuration : defauts, puis `config.toml`, puis les
    /// variables d'environnement `VIDEO_AUTOMATION_` (les dernieres gagnent).
    pub fn load() -> Result<Self, Error> {
        let config = Figment::from(Serialized::defaults(Self::defaut()))
            .merge(Toml::file("config.toml"))
            .merge(Env::prefixed("VIDEO_AUTOMATION_"))
            .extract()
            .map_err(|e| Error::Config(Box::new(e)))?;
        Ok(config)
    }
}

/// Lit la cle API Mistral dans l'environnement.
///
/// La cle ne figure **jamais** dans `config.toml` : elle vient de la variable
/// `MISTRAL_API_KEY` (chargee depuis un `.env` local par les binaires).
pub fn cle_api_mistral() -> Option<String> {
    std::env::var("MISTRAL_API_KEY")
        .ok()
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_un_toml_complet() {
        // Parsing depuis une chaine : pas de dependance au fichier reel.
        let toml = r#"
            data_dir = "donnees_test"
            server_addr = "0.0.0.0:9000"

            [llm]
            provider = "mistral"
            model = "mistral-medium-latest"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML de test doit etre valide");
        assert_eq!(config.data_dir, PathBuf::from("donnees_test"));
        assert_eq!(config.server_addr, "0.0.0.0:9000");
        assert_eq!(config.llm.provider, Provider::Mistral);
        assert_eq!(config.llm.model, "mistral-medium-latest");
        assert_eq!(config.llm.ollama_url, None);
    }

    #[test]
    fn parse_provider_ollama() {
        let toml = r#"
            data_dir = "data"
            server_addr = "127.0.0.1:8080"

            [llm]
            provider = "ollama"
            model = "llama3.1"
            ollama_url = "http://127.0.0.1:11434"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML de test doit etre valide");
        assert_eq!(config.llm.provider, Provider::Ollama);
        assert_eq!(
            config.llm.ollama_url.as_deref(),
            Some("http://127.0.0.1:11434")
        );
    }
}
