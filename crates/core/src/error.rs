//! Erreurs centrales du projet, derivees avec thiserror.

use thiserror::Error;

/// Erreur racine utilisee dans tout le workspace.
#[derive(Debug, Error)]
pub enum Error {
    /// Erreur de chargement ou de validation de la configuration.
    ///
    /// Boxee : `figment::Error` est volumineux (clippy::result_large_err).
    #[error("erreur de configuration : {0}")]
    Config(#[from] Box<figment::Error>),

    /// Erreur d'entree/sortie (fichiers, reseau local).
    #[error("erreur d'E/S : {0}")]
    Io(#[from] std::io::Error),

    /// Erreur remontee par la facade LLM.
    #[error("erreur LLM : {0}")]
    Llm(String),

    /// Erreur remontee par un outil (tool calling).
    #[error("erreur outil : {0}")]
    Tool(String),
}

impl Error {
    /// Construit une erreur de configuration a partir d'un simple message,
    /// sans exposer `figment` aux crates clients.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(Box::new(figment::Error::from(message.into())))
    }
}
