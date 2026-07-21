//! Agent Realisateur (orchestrateur) — v1 : enchaine transcription → scenario
//! et expose la porte de validation du scenario (voir `docs/agenda.md`,
//! phase 2).
//!
//! Le Realisateur ne raisonne pas encore par LLM en v1 : il pilote la machine
//! a etats et delegue la generation au Scenariste (`llm::scenariste`). Les
//! versions conversationnelles arriveront avec les phases suivantes.

use llm::{scenariste, CompletionModel, Extractor};
use video_core::error::Error;
use video_core::etat::{EtatPipeline, ModeTransition};
use video_core::projet::{DecisionValidation, Projet};
use video_core::scenario::Scenario;

/// Fait passer un projet de `Transcrit` a `ScenarioGenere` en produisant le
/// scenario via le Scenariste.
///
/// En mode `auto`, la transition sortante est validee d'office ; en mode
/// `validation`, `validation_scenario` reste `None` et le pipeline bloque
/// jusqu'a une decision via `POST /valider`.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas en etat `Transcrit` ou n'a pas
///   de transcription.
/// - `Error::Llm` si la generation du scenario echoue.
pub async fn produire_scenario<M: CompletionModel>(
    projet: &mut Projet,
    extracteur: &Extractor<M, Scenario>,
    mode: ModeTransition,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::Transcrit {
        return Err(Error::Pipeline(format!(
            "scenario demande sur un projet en etat {:?} (attendu : Transcrit)",
            projet.etat
        )));
    }
    let transcription = projet.transcription.clone().ok_or_else(|| {
        Error::Pipeline("projet en etat Transcrit sans transcription".to_string())
    })?;

    let scenario = scenariste::generer_scenario(extracteur, &transcription).await?;

    projet.scenario = Some(scenario);
    projet.etat = EtatPipeline::ScenarioGenere;
    if mode == ModeTransition::Auto {
        projet.validation_scenario = Some(DecisionValidation::Accepte);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use rig_core::completion::{
        AssistantContent, CompletionError, CompletionRequest, CompletionResponse, Usage,
    };
    use rig_core::message::{ToolCall, ToolFunction};
    use rig_core::streaming::StreamingCompletionResponse;
    use rig_core::OneOrMany;
    use video_core::projet::Transcription;

    /// Mock minimal de `CompletionModel` : reponses predefinies consommees
    /// dans l'ordre (meme principe que `llm/tests/hello_world.rs`).
    #[derive(Clone, Default)]
    struct ModeleFactice {
        reponses: Arc<Mutex<VecDeque<CompletionResponse<serde_json::Value>>>>,
    }

    impl CompletionModel for ModeleFactice {
        type Response = serde_json::Value;
        type StreamingResponse = ();
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self::default()
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            self.reponses
                .lock()
                .expect("mutex non empoisonne")
                .pop_front()
                .ok_or_else(|| {
                    CompletionError::ProviderError("plus de reponse scriptee".to_string())
                })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "streaming non supporte par le mock".to_string(),
            ))
        }
    }

    fn extracteur_factice() -> Extractor<ModeleFactice, Scenario> {
        let reponse = CompletionResponse {
            choice: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
                "appel_1".to_string(),
                ToolFunction::new(
                    "submit".to_string(),
                    serde_json::json!({
                        "titre": "Sujet dicte",
                        "public": "tout public",
                        "style_images": "photos documentaires",
                        "scenes": [
                            {
                                "narration": "Voici le sujet.",
                                "dialogues": [],
                                "description_visuelle": "Une image d'illustration",
                                "duree_cible": 8.0
                            }
                        ]
                    }),
                ),
            ))),
            usage: Usage::new(),
            raw_response: serde_json::json!({}),
            message_id: None,
        };
        let modele = ModeleFactice {
            reponses: Arc::new(Mutex::new(vec![reponse].into())),
        };
        scenariste::extracteur_sur_modele(modele)
    }

    fn projet_transcrit() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::Transcrit;
        projet.transcription = Some(Transcription {
            texte: "Un sujet dicte au telephone.".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![],
        });
        projet
    }

    #[tokio::test]
    async fn produit_un_scenario_en_mode_validation() {
        let mut projet = projet_transcrit();
        produire_scenario(
            &mut projet,
            &extracteur_factice(),
            ModeTransition::Validation,
        )
        .await
        .expect("la generation doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
        assert_eq!(
            projet.scenario.as_ref().map(|s| s.titre.as_str()),
            Some("Sujet dicte")
        );
        // Mode validation : la decision humaine reste attendue.
        assert_eq!(projet.validation_scenario, None);
    }

    #[tokio::test]
    async fn produit_un_scenario_en_mode_auto() {
        let mut projet = projet_transcrit();
        produire_scenario(&mut projet, &extracteur_factice(), ModeTransition::Auto)
            .await
            .expect("la generation doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
        assert_eq!(
            projet.validation_scenario,
            Some(DecisionValidation::Accepte)
        );
    }

    #[tokio::test]
    async fn refuse_un_projet_non_transcrit() {
        let mut projet = Projet::nouveau("abc123"); // etat AudioRecu
        let resultat = produire_scenario(
            &mut projet,
            &extracteur_factice(),
            ModeTransition::Validation,
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("attendu : Transcrit"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_un_projet_transcrit_sans_transcription() {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::Transcrit; // transcription absente
        let resultat = produire_scenario(
            &mut projet,
            &extracteur_factice(),
            ModeTransition::Validation,
        )
        .await;
        assert!(matches!(resultat, Err(Error::Pipeline(_))));
    }
}
