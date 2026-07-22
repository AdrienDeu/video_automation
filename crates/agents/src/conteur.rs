//! Agent Conteur : double chaque scene d'un scenario valide avec une voix
//! off synthetisee, via l'outil `generer_voix` (phase 4, voir
//! `docs/architecture.md` §6).
//!
//! Le Conteur n'est pas un LLM : il orchestre les appels TTS (un audio par
//! scene et par langue, cache par hash cote outil), mesure les durees
//! reelles via ffprobe et ecrit les sous-titres `.srt` synchronises.

use std::path::Path;

use video_core::annulation::{point_de_controle, CancellationToken};
use video_core::config::{self, Config};
use video_core::error::Error;
use video_core::etat::{EtatPipeline, ModeTransition};
use video_core::projet::{DecisionValidation, Projet};
use video_core::scenario::Scene;
use video_core::voix::VoixScene;

/// Langues cibles du doublage : la langue detectee a la transcription, `fr`
/// a defaut. Le multi-langue se generalisera avec la configuration projet.
fn langues_du_projet(projet: &Projet) -> Vec<String> {
    let langue = projet
        .transcription
        .as_ref()
        .and_then(|t| t.langue.clone())
        .unwrap_or_else(|| "fr".to_string());
    vec![langue]
}

/// Texte integral prononce pour une scene : la narration suivie des
/// repliques, dans l'ordre du scenario.
fn texte_scene(scene: &Scene) -> String {
    let mut texte = scene.narration.trim().to_string();
    for dialogue in &scene.dialogues {
        let replique = dialogue.replique.trim();
        if !replique.is_empty() {
            if !texte.is_empty() {
                texte.push(' ');
            }
            texte.push_str(replique);
        }
    }
    texte
}

/// Fait passer un projet de `VisuelsPrets` (visuels acceptes) a `VoixPretes` :
/// une voix off est synthetisee pour chaque scene et chaque langue cible, et
/// un fichier `.srt` synchronise sur les durees reelles est ecrit par langue.
///
/// En mode `auto`, la transition sortante est validee d'office ; en mode
/// `validation`, `validation_voix` reste `None` et le pipeline bloque
/// jusqu'a `POST /valider`.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas dans l'etat attendu, si les
///   visuels n'ont pas ete acceptes, ou si une scene n'a rien a dire.
/// - `Error::Tool` si la synthese vocale echoue (cle API absente, appel
///   reseau, ecriture disque).
/// - `Error::Annulation` si l'annulation est demandee entre deux scenes.
pub async fn produire_voix(
    projet: &mut Projet,
    config: &Config,
    mode: ModeTransition,
    token: &CancellationToken,
) -> Result<(), Error> {
    let langues = langues_du_projet(projet);
    let cle = config::cle_api_mistral()
        .ok_or_else(|| Error::Tool("MISTRAL_API_KEY absente de l'environnement".to_string()))?;
    produire_voix_langues(projet, config, &langues, &cle, mode, token).await
}

/// Implementation de [`produire_voix`], parametree par les langues cibles et
/// la cle API : testable en multi-langue et sans variable d'environnement.
async fn produire_voix_langues(
    projet: &mut Projet,
    config: &Config,
    langues: &[String],
    cle_api: &str,
    mode: ModeTransition,
    token: &CancellationToken,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::VisuelsPrets {
        return Err(Error::Pipeline(format!(
            "voix demandees sur un projet en etat {:?} (attendu : VisuelsPrets)",
            projet.etat
        )));
    }
    if projet.validation_visuels != Some(DecisionValidation::Accepte) {
        return Err(Error::Pipeline(
            "voix demandees avant acceptation des visuels".to_string(),
        ));
    }
    if langues.is_empty() {
        return Err(Error::Pipeline(
            "aucune langue cible pour la voix off".to_string(),
        ));
    }
    let scenario = projet
        .scenario
        .clone()
        .ok_or_else(|| Error::Pipeline("projet sans scenario".to_string()))?;

    let http = tools::voix::client_http()?;
    let dossier = config.data_dir.join(&projet.id);

    let mut voix = Vec::new();
    let mut sous_titres = Vec::new();
    for langue in langues {
        let mut durees = Vec::with_capacity(scenario.scenes.len());
        for (index, scene) in scenario.scenes.iter().enumerate() {
            point_de_controle(token)?;
            let texte = texte_scene(scene);
            if texte.is_empty() {
                return Err(Error::Pipeline(format!(
                    "scene {index} sans narration ni dialogue a dire"
                )));
            }
            let generee =
                tools::voix::generer_voix(&http, &config.voix, &dossier, &texte, langue, cle_api)
                    .await?;
            // A defaut de mesure ffprobe, la duree cible de la scene fait foi.
            let duree = generee.duree.unwrap_or(scene.duree_cible);
            voix.push(VoixScene {
                scene: index,
                langue: langue.clone(),
                fichier: generee.fichier,
                duree,
            });
            durees.push(duree);
        }

        let nom_srt = format!("sous-titres-{langue}.srt");
        ecrire_srt(
            &dossier,
            &nom_srt,
            &tools::sous_titres::generer_srt(&scenario, &durees),
        )?;
        sous_titres.push(nom_srt);
    }

    projet.voix = voix;
    projet.sous_titres = sous_titres;
    projet.etat = EtatPipeline::VoixPretes;
    if mode == ModeTransition::Auto {
        projet.validation_voix = Some(DecisionValidation::Accepte);
    }
    Ok(())
}

/// Ecrit un fichier de sous-titres dans le dossier du projet.
fn ecrire_srt(dossier: &Path, nom: &str, contenu: &str) -> Result<(), Error> {
    std::fs::create_dir_all(dossier)?;
    std::fs::write(dossier.join(nom), contenu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use video_core::config::{
        AudioConfig, LlmConfig, PipelineConfig, Provider, VoixConfig, YoutubeConfig,
    };
    use video_core::projet::{Segment, Transcription};
    use video_core::scenario::Scenario;

    /// Cle factice : seuls les chemins en cache sont testes, jamais le reseau.
    const CLE: &str = "cle-factice";

    fn config_de_test(data_dir: &Path) -> Config {
        Config {
            data_dir: data_dir.to_path_buf(),
            server_addr: "127.0.0.1:0".to_string(),
            llm: LlmConfig {
                provider: Provider::Mistral,
                model: "mistral-large-latest".to_string(),
                ollama_url: None,
            },
            audio: AudioConfig::default(),
            pipeline: PipelineConfig::default(),
            voix: VoixConfig {
                // Endpoint impossible : tout appel reseau ferait echouer le
                // test, seul le cache par hash est exploite.
                url: "http://127.0.0.1:1/injoignable".to_string(),
                modele: "voxtral-mini-tts".to_string(),
                voix: "default".to_string(),
            },
            youtube: YoutubeConfig::default(),
        }
    }

    fn projet_visuels_acceptes() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::VisuelsPrets;
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.transcription = Some(Transcription {
            texte: "Sujet dicte.".to_string(),
            langue: Some("fr".to_string()),
            segments: vec![Segment {
                debut: 0.0,
                fin: 1.0,
                texte: "Sujet dicte.".to_string(),
            }],
        });
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![
                Scene {
                    narration: "Bonjour le monde.".to_string(),
                    dialogues: vec![video_core::scenario::Dialogue {
                        personnage: "Prof".to_string(),
                        replique: "Une question ?".to_string(),
                    }],
                    description_visuelle: "Visuel 0".to_string(),
                    duree_cible: 8.0,
                },
                Scene {
                    narration: "Fin de la video.".to_string(),
                    dialogues: vec![],
                    description_visuelle: "Visuel 1".to_string(),
                    duree_cible: 4.0,
                },
            ],
        });
        projet
    }

    /// Pre-remplit le cache TTS du projet : les fichiers faux-audio nommes
    /// d'apres le hash de chaque scene/langue.
    fn semer_cache(projet: &Projet, config: &Config, langues: &[&str]) {
        let scenario = projet.scenario.as_ref().expect("scenario");
        let dossier = config.data_dir.join(&projet.id);
        std::fs::create_dir_all(&dossier).expect("dossier projet");
        for langue in langues {
            for scene in &scenario.scenes {
                let nom = format!(
                    "voix-{}.mp3",
                    tools::voix::hash_voix(
                        &texte_scene(scene),
                        langue,
                        &config.voix.voix,
                        &config.voix.modele
                    )
                );
                std::fs::write(dossier.join(nom), b"faux audio").expect("cache");
            }
        }
    }

    #[tokio::test]
    async fn produit_les_voix_et_le_srt_en_mode_validation() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        semer_cache(&projet, &config, &["fr"]);

        produire_voix_langues(
            &mut projet,
            &config,
            &["fr".to_string()],
            CLE,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await
        .expect("la production doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::VoixPretes);
        assert_eq!(projet.voix.len(), 2);
        assert_eq!(projet.voix[0].scene, 0);
        assert_eq!(projet.voix[0].langue, "fr");
        // Faux audio illisible par ffprobe : repli sur la duree cible.
        assert_eq!(projet.voix[0].duree, 8.0);
        // Mode validation : la decision humaine reste attendue.
        assert_eq!(projet.validation_voix, None);

        // Le .srt est ecrit et synchronise sur les durees (cibles ici).
        assert_eq!(projet.sous_titres, vec!["sous-titres-fr.srt".to_string()]);
        let srt = std::fs::read_to_string(temp.path().join("abc123").join("sous-titres-fr.srt"))
            .expect("le .srt est ecrit");
        assert!(srt.starts_with("1\n00:00:00,000 --> "), "{srt}");
        assert!(
            srt.contains("00:00:08,000 --> 00:00:12,000\nFin de la video."),
            "{srt}"
        );
    }

    #[tokio::test]
    async fn produit_les_voix_en_mode_auto_et_multi_langue() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        semer_cache(&projet, &config, &["fr", "en"]);

        let langues = vec!["fr".to_string(), "en".to_string()];
        produire_voix_langues(
            &mut projet,
            &config,
            &langues,
            CLE,
            ModeTransition::Auto,
            &CancellationToken::new(),
        )
        .await
            .expect("la production doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::VoixPretes);
        assert_eq!(projet.voix.len(), 4); // 2 scenes x 2 langues
        assert!(projet.voix.iter().any(|v| v.langue == "en" && v.scene == 1));
        assert_eq!(
            projet.sous_titres,
            vec![
                "sous-titres-fr.srt".to_string(),
                "sous-titres-en.srt".to_string()
            ]
        );
        assert_eq!(projet.validation_voix, Some(DecisionValidation::Accepte));
    }

    #[tokio::test]
    async fn refuse_un_projet_hors_visuels_prets() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        projet.etat = EtatPipeline::ScenarioGenere;

        let resultat = produire_voix_langues(
            &mut projet,
            &config,
            &["fr".to_string()],
            CLE,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("VisuelsPrets"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_des_visuels_non_acceptes() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        projet.validation_visuels = None; // visuels pas encore tranches

        let resultat = produire_voix_langues(
            &mut projet,
            &config,
            &["fr".to_string()],
            CLE,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("acceptation des visuels"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn echoue_sans_cache_ni_reseau() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        // Pas de cache : l'appel TTS part vers l'endpoint impossible.
        let resultat = produire_voix_langues(
            &mut projet,
            &config,
            &["fr".to_string()],
            CLE,
            ModeTransition::Validation,
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(resultat, Err(Error::Tool(_))));
    }

    #[tokio::test]
    async fn s_interrompt_des_la_premiere_scene_si_annule() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_visuels_acceptes();
        semer_cache(&projet, &config, &["fr"]);
        let token = CancellationToken::new();
        token.cancel();

        let resultat = produire_voix_langues(
            &mut projet,
            &config,
            &["fr".to_string()],
            CLE,
            ModeTransition::Validation,
            &token,
        )
        .await;
        assert!(matches!(resultat, Err(Error::Annulation)));
        // Le projet reste dans son etat d'entree : reprise possible.
        assert_eq!(projet.etat, EtatPipeline::VisuelsPrets);
    }
}
