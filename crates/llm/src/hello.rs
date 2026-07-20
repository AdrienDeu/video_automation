//! Hello-world de tool calling : un outil factice `dire_bonjour` et la
//! fonction qui pilote un agent pour qu'il l'appelle.
//!
//! Sert de critère de sortie de la phase 0 : prouve que la boucle
//! agent/outil de rig fonctionne de bout en bout (voir `tests/hello_world.rs`).

use rig_core::agent::Agent;
use rig_core::completion::{CompletionModel, Prompt};
use rig_core::tool::Tool;
use schemars::JsonSchema;
use serde::Deserialize;

/// Prompt utilise pour la demonstration.
const PROMPT_HELLO: &str = "Salue Léa en appelant l'outil dire_bonjour, \
     puis rapporte sa réponse telle quelle.";

/// Arguments de l'outil `dire_bonjour`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsDireBonjour {
    /// Prénom de la personne à saluer.
    pub nom: String,
}

/// Outil factice de phase 0 : retourne une salutation, sans aucun effet de
/// bord.
#[derive(Debug, Clone, Copy, Default)]
pub struct DireBonjour;

impl Tool for DireBonjour {
    /// Nom sous lequel le modèle appelle l'outil.
    const NAME: &'static str = "dire_bonjour";

    type Error = std::convert::Infallible;
    type Args = ArgsDireBonjour;
    type Output = String;

    fn description(&self) -> String {
        "Dit bonjour à une personne à partir de son prénom.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(ArgsDireBonjour))
            .expect("le schema des arguments doit etre serialisable")
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(format!("Bonjour, {} !", args.nom))
    }
}

/// Demande a l'agent de saluer Léa via l'outil `dire_bonjour` et retourne sa
/// réponse finale.
///
/// L'agent est fourni par l'appelant (deja muni de l'outil et du preamble) :
/// cette fonction ne fait que piloter la boucle de tool calling. Le budget de
/// 4 appels modèle couvre : appel d'outil → réponse finale, avec de la marge.
pub async fn executer_hello_world<M>(agent: &Agent<M>) -> anyhow::Result<String>
where
    M: CompletionModel + 'static,
{
    let reponse = agent.prompt(PROMPT_HELLO).max_turns(4).await?;
    Ok(reponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn l_outil_formate_la_salutation() {
        let sortie = DireBonjour
            .call(ArgsDireBonjour {
                nom: "Léa".to_string(),
            })
            .await
            .expect("l'outil factice ne peut pas echouer");
        assert_eq!(sortie, "Bonjour, Léa !");
    }

    #[test]
    fn l_outil_expose_son_schema() {
        let schema = DireBonjour.parameters();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["nom"].is_object());
        assert_eq!(DireBonjour.name(), "dire_bonjour");
    }
}
