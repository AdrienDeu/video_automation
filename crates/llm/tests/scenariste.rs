//! Test de l'extraction structuree du Scenariste : sans cle API ni reseau,
//! grace a un modele mocke qui appelle l'outil `submit` avec un scenario JSON.
//!
//! Le test `scenario_mistral_reel` permet en complement une verification
//! locale contre la vraie API quand `MISTRAL_API_KEY` est presente.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use llm::scenariste;
use rig_core::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
    Usage,
};
use rig_core::message::{ToolCall, ToolFunction};
use rig_core::streaming::StreamingCompletionResponse;
use rig_core::OneOrMany;
use video_core::projet::Transcription;

/// Mock du trait `CompletionModel` de rig (meme principe que
/// `tests/hello_world.rs`) : reponses predefinies consommees dans l'ordre.
#[derive(Clone, Default)]
struct ModeleFactice {
    reponses: Arc<Mutex<VecDeque<CompletionResponse<serde_json::Value>>>>,
}

impl ModeleFactice {
    fn avec_reponses(reponses: Vec<CompletionResponse<serde_json::Value>>) -> Self {
        Self {
            reponses: Arc::new(Mutex::new(reponses.into())),
        }
    }
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
            .ok_or_else(|| CompletionError::ProviderError("plus de reponse scriptee".to_string()))
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

/// Reponse simulee : le modele soumet un scenario via l'outil `submit` de
/// l'extracteur.
fn reponse_submit(arguments: serde_json::Value) -> CompletionResponse<serde_json::Value> {
    CompletionResponse {
        choice: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
            "appel_1".to_string(),
            ToolFunction::new("submit".to_string(), arguments),
        ))),
        usage: Usage::new(),
        raw_response: serde_json::json!({}),
        message_id: None,
    }
}

fn scenario_json() -> serde_json::Value {
    serde_json::json!({
        "titre": "La photosynthese",
        "public": "collegiens",
        "style_images": "photos macro, tons verts",
        "scenes": [
            {
                "narration": "Les plantes fabriquent leur propre nourriture.",
                "dialogues": [],
                "description_visuelle": "Gros plan sur une feuille verte au soleil",
                "duree_cible": 12.0
            },
            {
                "narration": "Ce processus s'appelle la photosynthese.",
                "dialogues": [
                    { "personnage": "Prof", "replique": "Retenez ce mot !" }
                ],
                "description_visuelle": "Schema d'une feuille avec fleches soleil et CO2",
                "duree_cible": 10.0
            }
        ]
    })
}

fn transcription_fixture() -> Transcription {
    Transcription {
        texte: "Je veux expliquer la photosynthese a des collegiens : les plantes \
                fabriquent leur nourriture grace a la lumiere."
            .to_string(),
        langue: Some("fr".to_string()),
        segments: vec![],
    }
}

#[tokio::test]
async fn extraction_scenario_avec_mock() {
    let modele = ModeleFactice::avec_reponses(vec![reponse_submit(scenario_json())]);
    let extracteur = scenariste::extracteur_sur_modele(modele);

    let scenario = scenariste::generer_scenario(&extracteur, &transcription_fixture())
        .await
        .expect("l'extraction doit aboutir");

    assert_eq!(scenario.titre, "La photosynthese");
    assert_eq!(scenario.scenes.len(), 2);
    assert_eq!(scenario.scenes[0].duree_cible, 12.0);
    assert_eq!(scenario.scenes[1].dialogues[0].personnage, "Prof");
}

#[tokio::test]
async fn extraction_echoue_sans_appel_submit() {
    // Le modele repond en texte libre deux fois (2 tentatives accordees) :
    // l'extraction doit echouer proprement en `Error::Llm`.
    let reponse_texte = || CompletionResponse {
        choice: OneOrMany::one(AssistantContent::text("Voici un scenario.")),
        usage: Usage::new(),
        raw_response: serde_json::json!({}),
        message_id: None,
    };
    let modele = ModeleFactice::avec_reponses(vec![reponse_texte(), reponse_texte()]);
    let extracteur = scenariste::extracteur_sur_modele(modele);

    let resultat = scenariste::generer_scenario(&extracteur, &transcription_fixture()).await;
    match resultat {
        Err(erreur) => assert!(
            erreur.to_string().contains("generation du scenario"),
            "l'erreur doit etre contextualisee : {erreur}"
        ),
        Ok(_) => panic!("l'extraction sans appel a submit doit echouer"),
    }
}

/// Verification locale contre la vraie API Mistral : ignoree silencieusement
/// tant que `MISTRAL_API_KEY` n'est pas definie (donc en CI).
#[tokio::test]
async fn scenario_mistral_reel() {
    dotenvy::dotenv().ok();
    let Some(cle) = video_core::config::cle_api_mistral() else {
        eprintln!("MISTRAL_API_KEY absente : scenario_mistral_reel ignore.");
        return;
    };

    let extracteur = scenariste::construire_extracteur_scenario(&cle, "mistral-large-latest")
        .expect("l'extracteur Mistral doit se construire");
    let scenario = scenariste::generer_scenario(&extracteur, &transcription_fixture())
        .await
        .expect("l'appel reel a l'API Mistral doit aboutir");

    assert!(!scenario.titre.is_empty(), "le titre ne doit pas etre vide");
    assert!(
        !scenario.scenes.is_empty(),
        "le scenario doit contenir au moins une scene"
    );
    for scene in &scenario.scenes {
        assert!(
            !scene.description_visuelle.is_empty(),
            "chaque scene doit decrire son visuel"
        );
        assert!(scene.duree_cible > 0.0, "duree cible positive");
    }
}
