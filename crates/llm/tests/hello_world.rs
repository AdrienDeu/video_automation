//! Critère de sortie de la phase 0 : un agent rig minimal appelle un outil
//! factice, sans clé API ni réseau, grace a un modèle mocké.
//!
//! Le test `demo_mistral_reelle` permet en complément une vérification locale
//! contre la vraie API quand `MISTRAL_API_KEY` est presente.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use llm::client;
use llm::hello::{executer_hello_world, DireBonjour};
use rig_core::agent::AgentBuilder;
use rig_core::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
    Usage,
};
use rig_core::message::{ToolCall, ToolFunction};
use rig_core::streaming::StreamingCompletionResponse;
use rig_core::OneOrMany;

/// Réponses scriptées consommées une à une par le modèle mocké.
type ReponsesScriptees = Arc<Mutex<VecDeque<CompletionResponse<serde_json::Value>>>>;

/// Mock du trait `CompletionModel` de rig : aucune clé, aucun réseau, des
/// réponses prédéfinies consommées dans l'ordre, et les requêtes reçues sont
/// enregistrées pour vérification.
#[derive(Clone, Default)]
struct ModeleFactice {
    reponses: ReponsesScriptees,
    requetes: Arc<Mutex<Vec<CompletionRequest>>>,
}

impl ModeleFactice {
    fn avec_reponses(reponses: Vec<CompletionResponse<serde_json::Value>>) -> Self {
        Self {
            reponses: Arc::new(Mutex::new(reponses.into())),
            requetes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requetes(&self) -> MutexGuard<'_, Vec<CompletionRequest>> {
        self.requetes.lock().expect("mutex non empoisonne")
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
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        self.requetes().push(request);
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

/// 1er appel simulé : le modèle demande l'outil `dire_bonjour`.
fn reponse_appel_outil(
    nom_outil: &str,
    arguments: serde_json::Value,
) -> CompletionResponse<serde_json::Value> {
    CompletionResponse {
        choice: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
            "appel_1".to_string(),
            ToolFunction::new(nom_outil.to_string(), arguments),
        ))),
        usage: Usage::new(),
        raw_response: serde_json::json!({}),
        message_id: None,
    }
}

/// 2e appel simulé : le modèle rend sa réponse finale en texte.
fn reponse_texte(texte: &str) -> CompletionResponse<serde_json::Value> {
    CompletionResponse {
        choice: OneOrMany::one(AssistantContent::text(texte)),
        usage: Usage::new(),
        raw_response: serde_json::json!({}),
        message_id: None,
    }
}

/// Critère de sortie de la phase 0 : la boucle agent/outil de rig fonctionne
/// de bout en bout, sans clé API ni réseau.
#[tokio::test]
async fn hello_world_avec_mock() {
    // Scenario : 1er appel → tool call `dire_bonjour` { "nom": "Léa" } ;
    // 2e appel → texte final rapportant la sortie réelle de l'outil.
    let modele = ModeleFactice::avec_reponses(vec![
        reponse_appel_outil("dire_bonjour", serde_json::json!({ "nom": "Léa" })),
        reponse_texte("L'outil a répondu : Bonjour, Léa !"),
    ]);
    let enregistrement = modele.clone();

    let agent = AgentBuilder::new(modele)
        .preamble("Tu salues toujours via l'outil dire_bonjour.")
        .tool(DireBonjour)
        .build();

    let reponse = executer_hello_world(&agent)
        .await
        .expect("l'agent doit produire une reponse finale");

    // La réponse finale rapporte la salutation construite par l'outil.
    assert!(
        reponse.contains("Bonjour, Léa"),
        "la reponse finale doit contenir la salutation : {reponse}"
    );

    // L'outil a réellement été exécuté : le 2e appel modèle a reçu le
    // résultat de l'outil (« Bonjour, Léa ! ») dans son historique.
    let requetes = enregistrement.requetes();
    assert_eq!(requetes.len(), 2, "un appel outil + une reponse finale");
    let historique = serde_json::to_string(&requetes[1].chat_history)
        .expect("l'historique doit etre serialisable");
    assert!(
        historique.contains("Bonjour, Léa !"),
        "le resultat de l'outil doit figurer dans l'historique du 2e appel : {historique}"
    );
}

/// Vérification locale contre la vraie API Mistral : ignorée silencieusement
/// tant que `MISTRAL_API_KEY` n'est pas définie (donc en CI).
#[tokio::test]
async fn demo_mistral_reelle() {
    dotenvy::dotenv().ok();
    let Some(cle) = video_core::config::cle_api_mistral() else {
        eprintln!("MISTRAL_API_KEY absente : demo_mistral_reelle ignore.");
        return;
    };

    let agent = client::construire_agent_mistral(&cle, "mistral-large-latest")
        .expect("le client Mistral doit se construire")
        .preamble("Tu réponds toujours en français.")
        .tool(DireBonjour)
        .build();

    let reponse = executer_hello_world(&agent)
        .await
        .expect("l'appel reel a l'API Mistral doit aboutir");
    assert!(
        reponse.contains("Bonjour, Léa"),
        "la reponse reelle doit rapporter la salutation : {reponse}"
    );
}
