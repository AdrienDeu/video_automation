//! Agent Visuel : illustre chaque scene d'un scenario valide avec une image
//! licenciee, via l'outil `choisir_image` (phase 3).
//!
//! L'agent raisonne par scene (une boucle agent/outil par scene) : les couts
//! sont bornes et une scene en echec est identifiable precisement.

use llm::visuel::ImagesChoisies;
use llm::{Agent, CompletionModel, Prompt};
use video_core::annulation::{point_de_controle, CancellationToken};
use video_core::config::Config;
use video_core::error::Error;
use video_core::etat::{EtatPipeline, ModeTransition};
use video_core::projet::{DecisionValidation, Projet};

/// Nombre maximal d'appels modele par scene (appel d'outil + cloture).
const TOURS_MAX_PAR_SCENE: u32 = 4;

/// Fait passer un projet de `ScenarioGenere` (scenario accepte) a
/// `VisuelsPrets` : une image licenciee est choisie pour chaque scene.
///
/// En mode `auto`, la transition sortante est validee d'office ; en mode
/// `validation`, `validation_visuels` reste `None` et le pipeline bloque
/// jusqu'a `POST /valider`.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas dans l'etat attendu, si le
///   scenario n'a pas ete accepte, ou si une scene reste sans image.
/// - `Error::Llm` si une boucle agent/outil echoue.
/// - `Error::Annulation` si l'annulation est demandee entre deux scenes.
pub async fn produire_visuels<M: CompletionModel + 'static>(
    projet: &mut Projet,
    agent: &Agent<M>,
    images_choisies: &ImagesChoisies,
    mode: ModeTransition,
    token: &CancellationToken,
) -> Result<(), Error> {
    produire_visuels_avec_consigne(projet, agent, images_choisies, mode, None, token).await
}

/// Implementation de [`produire_visuels`], avec en option une consigne
/// d'affinage de l'utilisateur integree a la consigne de chaque scene : elle
/// guide la regeneration des requetes de recherche d'images (phase 7,
/// `POST /affiner`).
pub async fn produire_visuels_avec_consigne<M: CompletionModel + 'static>(
    projet: &mut Projet,
    agent: &Agent<M>,
    images_choisies: &ImagesChoisies,
    mode: ModeTransition,
    consigne_affinage: Option<&str>,
    token: &CancellationToken,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::ScenarioGenere {
        return Err(Error::Pipeline(format!(
            "visuels demandes sur un projet en etat {:?} (attendu : ScenarioGenere)",
            projet.etat
        )));
    }
    if projet.validation_scenario != Some(DecisionValidation::Accepte) {
        return Err(Error::Pipeline(
            "visuels demandes avant acceptation du scenario".to_string(),
        ));
    }
    let scenario = projet
        .scenario
        .clone()
        .ok_or_else(|| Error::Pipeline("projet sans scenario".to_string()))?;

    for (index, scene) in scenario.scenes.iter().enumerate() {
        point_de_controle(token)?;
        let mut consigne = format!(
            "Scene {index} :\n\
             - description visuelle : {}\n\
             - narration : {}\n\
             Style commun de la video : {}\n",
            scene.description_visuelle, scene.narration, scenario.style_images
        );
        if let Some(affinage) = consigne_affinage {
            consigne.push_str(&format!(
                "Consigne d'affinage de l'utilisateur, a integrer au choix de \
                 l'image : {affinage}\n"
            ));
        }
        consigne.push_str("Appelle choisir_image pour illustrer cette scene.");
        agent
            .prompt(consigne)
            .max_turns(TOURS_MAX_PAR_SCENE as usize)
            .await
            .map_err(|e| Error::Llm(format!("choix de l'image de la scene {index} : {e}")))?;
    }

    // Une image par scene, dans l'ordre : la premiere image choisie pour une
    // scene fait foi, un appel en double est ignore.
    let choisies = images_choisies.lock().expect("mutex non empoisonne");
    let mut visuels = Vec::with_capacity(scenario.scenes.len());
    for index in 0..scenario.scenes.len() {
        let asset = choisies
            .iter()
            .find(|a| a.scene == index)
            .ok_or_else(|| Error::Pipeline(format!("scene {index} restee sans image")))?;
        visuels.push(asset.clone());
    }
    drop(choisies);

    projet.visuels = visuels;
    projet.etat = EtatPipeline::VisuelsPrets;
    if mode == ModeTransition::Auto {
        projet.validation_visuels = Some(DecisionValidation::Accepte);
    }
    Ok(())
}

/// Construit l'agent Visuel et son outil depuis la configuration du projet,
/// puis produit les visuels. Raccourci utilise par le serveur.
///
/// # Erreurs
/// Voir [`produire_visuels`] ; `Error::Llm` en plus si l'agent ne peut etre
/// construit (cle API absente).
pub async fn produire_visuels_depuis_config(
    projet: &mut Projet,
    config: &Config,
    mode: ModeTransition,
    token: &CancellationToken,
) -> Result<(), Error> {
    let (agent, images_choisies) = construire_agent(config, projet)?;
    produire_visuels(projet, &agent, &images_choisies, mode, token).await
}

/// Regenere tous les visuels en integrant une consigne d'affinage de
/// l'utilisateur dans la consigne de chaque scene (phase 7, `POST /affiner`).
///
/// # Erreurs
/// Voir [`produire_visuels_depuis_config`].
pub async fn affiner_visuels_depuis_config(
    projet: &mut Projet,
    config: &Config,
    mode: ModeTransition,
    consigne: &str,
    token: &CancellationToken,
) -> Result<(), Error> {
    let (agent, images_choisies) = construire_agent(config, projet)?;
    produire_visuels_avec_consigne(
        projet,
        &agent,
        &images_choisies,
        mode,
        Some(consigne),
        token,
    )
    .await
}

/// Construit l'agent Visuel et son collecteur d'images pour un projet.
fn construire_agent(
    config: &Config,
    projet: &Projet,
) -> Result<(llm::visuel::AgentVisuel, ImagesChoisies), Error> {
    let images_choisies: ImagesChoisies = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let dossier = config.data_dir.join(&projet.id);
    let outil = llm::visuel::ChoisirImage::nouveau(dossier, images_choisies.clone())?;
    let agent = llm::visuel::construire_agent_visuel_depuis_config(&config.llm, outil)?;
    Ok((agent, images_choisies))
}

/// Remplace l'image d'une scene par une nouvelle recherche (mode validation :
/// « remplacement par prompt », voir `docs/agenda.md` phase 3).
///
/// La requete est fournie par l'utilisateur, pas par le LLM : cette fonction
/// ne fait pas appel a un agent. Le style visuel du scenario sert au scoring.
/// Apres remplacement, `validation_visuels` est remise a `None` : le nouvel
/// ensemble de visuels doit etre re-valide.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas en etat `VisuelsPrets` ou si la
///   scene n'a pas d'image a remplacer.
/// - `Error::Tool` si la recherche ou le telechargement echoue.
pub async fn remplacer_image(
    projet: &mut Projet,
    config: &Config,
    scene: usize,
    requete: &str,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::VisuelsPrets {
        return Err(Error::Pipeline(format!(
            "remplacement demande sur un projet en etat {:?} (attendu : VisuelsPrets)",
            projet.etat
        )));
    }
    if !projet.visuels.iter().any(|a| a.scene == scene) {
        return Err(Error::Pipeline(format!(
            "aucune image a remplacer pour la scene {scene}"
        )));
    }
    let style = projet
        .scenario
        .as_ref()
        .map(|s| s.style_images.clone())
        .unwrap_or_default();

    let http = tools::images::client_http()?;
    let dossier = config.data_dir.join(&projet.id);
    let asset = tools::images::choisir_image(&http, &dossier, scene, requete, &style).await?;

    let emplacement = projet
        .visuels
        .iter_mut()
        .find(|a| a.scene == scene)
        .expect("verifie plus haut");
    *emplacement = asset;
    projet.validation_visuels = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use rig_core::agent::AgentBuilder;
    use rig_core::completion::{
        AssistantContent, CompletionError, CompletionRequest, CompletionResponse, Usage,
    };
    use rig_core::message::{ToolCall, ToolFunction};
    use rig_core::streaming::StreamingCompletionResponse;
    use rig_core::tool::Tool;
    use rig_core::OneOrMany;
    use video_core::asset::{Asset, SourceImage};
    use video_core::scenario::{Scenario, Scene};

    /// Mock de `CompletionModel` : reponses predefinies consommees dans
    /// l'ordre (meme principe que `llm/tests/hello_world.rs`). Les requetes
    /// recues sont capturees (format debug) pour verifier les consignes.
    #[derive(Clone, Default)]
    struct ModeleFactice {
        reponses: Arc<Mutex<VecDeque<CompletionResponse<serde_json::Value>>>>,
        requetes: Arc<Mutex<Vec<String>>>,
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
            self.requetes
                .lock()
                .expect("mutex non empoisonne")
                .push(format!("{_request:?}"));
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

    /// Outil factice : fabrique un `Asset` sans reseau et le consigne dans le
    /// collecteur partage, a la place du vrai `choisir_image`.
    struct ChoisirImageFactice {
        choisies: ImagesChoisies,
    }

    #[derive(serde::Deserialize)]
    struct ArgsFactice {
        scene_id: u32,
    }

    impl Tool for ChoisirImageFactice {
        const NAME: &'static str = "choisir_image";

        type Error = std::convert::Infallible;
        type Args = ArgsFactice;
        type Output = String;

        fn description(&self) -> String {
            "Choisit une image (factice).".to_string()
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": { "scene_id": { "type": "integer" } },
                "required": ["scene_id"]
            })
        }

        async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
            let asset = Asset {
                scene: args.scene_id as usize,
                fichier: format!("scene-{}.jpg", args.scene_id),
                source: SourceImage::Openverse,
                titre: Some(format!("Image scene {}", args.scene_id)),
                auteur: Some("Test".to_string()),
                url_page: "https://example.org/oeuvre".to_string(),
                url_fichier: "https://example.org/oeuvre.jpg".to_string(),
                licence: "CC0".to_string(),
                licence_url: None,
                largeur: Some(1024),
                hauteur: Some(768),
            };
            self.choisies
                .lock()
                .expect("mutex non empoisonne")
                .push(asset);
            Ok("image choisie".to_string())
        }
    }

    fn reponse_appel_outil(scene_id: u32) -> CompletionResponse<serde_json::Value> {
        CompletionResponse {
            choice: OneOrMany::one(AssistantContent::ToolCall(ToolCall::new(
                format!("appel_{scene_id}"),
                ToolFunction::new(
                    "choisir_image".to_string(),
                    serde_json::json!({ "scene_id": scene_id }),
                ),
            ))),
            usage: Usage::new(),
            raw_response: serde_json::json!({}),
            message_id: None,
        }
    }

    fn reponse_texte() -> CompletionResponse<serde_json::Value> {
        CompletionResponse {
            choice: OneOrMany::one(AssistantContent::text("scene illustree")),
            usage: Usage::new(),
            raw_response: serde_json::json!({}),
            message_id: None,
        }
    }

    fn projet_scenario_accepte(nb_scenes: usize) -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::ScenarioGenere;
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos documentaires".to_string(),
            scenes: (0..nb_scenes)
                .map(|i| Scene {
                    narration: format!("Narration {i}."),
                    dialogues: vec![],
                    description_visuelle: format!("Visuel {i}"),
                    duree_cible: 8.0,
                })
                .collect(),
        });
        projet
    }

    /// Construit un agent mocke qui appelle `choisir_image` pour chaque scene
    /// demandee (une boucle outil + cloture par scene).
    fn agent_factice(nb_scenes: u32) -> (Agent<ModeleFactice>, ImagesChoisies) {
        let mut reponses = VecDeque::new();
        for scene_id in 0..nb_scenes {
            reponses.push_back(reponse_appel_outil(scene_id));
            reponses.push_back(reponse_texte());
        }
        let choisies: ImagesChoisies = Arc::new(Mutex::new(vec![]));
        let agent = AgentBuilder::new(ModeleFactice {
            reponses: Arc::new(Mutex::new(reponses)),
            requetes: Arc::new(Mutex::new(vec![])),
        })
        .tool(ChoisirImageFactice {
            choisies: choisies.clone(),
        })
        .build();
        (agent, choisies)
    }

    #[tokio::test]
    async fn affine_les_visuels_en_integrant_la_consigne() {
        let mut projet = projet_scenario_accepte(2);
        // Meme montage que `agent_factice`, avec capture des requetes.
        let reponses = VecDeque::from(vec![
            reponse_appel_outil(0),
            reponse_texte(),
            reponse_appel_outil(1),
            reponse_texte(),
        ]);
        let requetes: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let choisies: ImagesChoisies = Arc::new(Mutex::new(vec![]));
        let agent = AgentBuilder::new(ModeleFactice {
            reponses: Arc::new(Mutex::new(reponses)),
            requetes: requetes.clone(),
        })
        .tool(ChoisirImageFactice {
            choisies: choisies.clone(),
        })
        .build();

        produire_visuels_avec_consigne(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Validation,
            Some("Plutot des photos de nuit"),
            &CancellationToken::new(),
        )
        .await
        .expect("l'affinage doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
        assert_eq!(projet.visuels.len(), 2);
        // Chaque consigne de scene integre la consigne d'affinage.
        let captures = requetes.lock().expect("mutex non empoisonne");
        assert!(
            captures
                .iter()
                .any(|r| r.contains("Plutot des photos de nuit")),
            "la consigne d'affinage doit etre transmise au modele : {captures:?}"
        );
    }

    #[tokio::test]
    async fn produit_une_image_par_scene_en_mode_validation() {
        let mut projet = projet_scenario_accepte(2);
        let (agent, choisies) = agent_factice(2);

        produire_visuels(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await
        .expect("la production doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
        assert_eq!(projet.visuels.len(), 2);
        assert_eq!(projet.visuels[0].scene, 0);
        assert_eq!(projet.visuels[1].fichier, "scene-1.jpg");
        // Mode validation : la decision humaine reste attendue.
        assert_eq!(projet.validation_visuels, None);
    }

    #[tokio::test]
    async fn produit_les_visuels_en_mode_auto() {
        let mut projet = projet_scenario_accepte(1);
        let (agent, choisies) = agent_factice(1);

        produire_visuels(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Auto,
            &CancellationToken::new(),
        )
        .await
        .expect("la production doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
        assert_eq!(projet.validation_visuels, Some(DecisionValidation::Accepte));
    }

    #[tokio::test]
    async fn refuse_un_scenario_non_accepte() {
        let mut projet = projet_scenario_accepte(1);
        projet.validation_scenario = None; // scenario pas encore tranche
        let (agent, choisies) = agent_factice(1);

        let resultat = produire_visuels(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("acceptation du scenario"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn echoue_si_une_scene_reste_sans_image() {
        let mut projet = projet_scenario_accepte(2);
        // Le modele repond bien aux deux consignes mais n'appelle l'outil que
        // pour la scene 0 : la scene 1 reste sans image.
        let reponses = VecDeque::from(vec![
            reponse_appel_outil(0),
            reponse_texte(),
            reponse_texte(), // scene 1 : pas d'appel d'outil
        ]);
        let choisies: ImagesChoisies = Arc::new(Mutex::new(vec![]));
        let agent = AgentBuilder::new(ModeleFactice {
            reponses: Arc::new(Mutex::new(reponses)),
            requetes: Arc::new(Mutex::new(vec![])),
        })
        .tool(ChoisirImageFactice {
            choisies: choisies.clone(),
        })
        .build();

        let resultat = produire_visuels(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("scene 1"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn s_interrompt_avant_la_premiere_scene_si_annule() {
        let mut projet = projet_scenario_accepte(2);
        let (agent, choisies) = agent_factice(2);
        let token = CancellationToken::new();
        token.cancel();

        let resultat = produire_visuels(
            &mut projet,
            &agent,
            &choisies,
            ModeTransition::Validation,
            &token,
        )
        .await;
        assert!(matches!(resultat, Err(Error::Annulation)));
        // Aucune image choisie, etat d'entree conserve : reprise possible.
        assert!(choisies.lock().expect("mutex non empoisonne").is_empty());
        assert_eq!(projet.etat, EtatPipeline::ScenarioGenere);
    }
}
