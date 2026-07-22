//! Agent Monteur : assemble les assets valides d'un projet (images, voix
//! off, sous-titres) en une video finale 1080p et une preview de validation,
//! via l'outil `ffmpeg` et sa whitelist de templates (phase 5, voir
//! `docs/architecture.md` §6-7).
//!
//! Comme le Conteur, le Monteur n'est pas un LLM : il orchestre deux rendus
//! ffmpeg (preview 480p puis video 1080p) a partir des livrables des etapes
//! amont. Aucune commande libre n'est jamais construite.

use video_core::config::Config;
use video_core::error::Error;
use video_core::etat::{EtatPipeline, ModeTransition};
use video_core::projet::{DecisionValidation, Projet};

use tools::ffmpeg::{self, SceneMontage, PROFIL_FINAL, PROFIL_PREVIEW};

/// Nom du fichier de la video finale dans le dossier du projet.
const NOM_VIDEO: &str = "video.mp4";
/// Nom du fichier de preview basse resolution (validation humaine).
const NOM_PREVIEW: &str = "preview.mp4";

/// Fait passer un projet de `VoixPretes` (voix acceptees) a `MontagePret` :
/// une preview 480p puis la video finale 1080p sont rendues dans le dossier
/// du projet (ken-burns, fondus enchaines, voix normalisee, sous-titres
/// incrustes).
///
/// En mode `auto`, la transition sortante est validee d'office ; en mode
/// `validation`, `validation_montage` reste `None` et le pipeline bloque
/// jusqu'a `POST /valider`.
///
/// # Erreurs
/// - `Error::Pipeline` si le projet n'est pas dans l'etat attendu, si les
///   voix n'ont pas ete acceptees, ou si une scene n'a pas de visuel, de
///   voix ou de sous-titres.
/// - `Error::Tool` si ffmpeg est introuvable ou si un rendu echoue.
pub async fn produire_montage(
    projet: &mut Projet,
    config: &Config,
    mode: ModeTransition,
) -> Result<(), Error> {
    if projet.etat != EtatPipeline::VoixPretes {
        return Err(Error::Pipeline(format!(
            "montage demande sur un projet en etat {:?} (attendu : VoixPretes)",
            projet.etat
        )));
    }
    if projet.validation_voix != Some(DecisionValidation::Accepte) {
        return Err(Error::Pipeline(
            "montage demande avant acceptation des voix".to_string(),
        ));
    }
    let scenario = projet
        .scenario
        .clone()
        .ok_or_else(|| Error::Pipeline("projet sans scenario".to_string()))?;
    if !ffmpeg::ffmpeg_disponible().await {
        return Err(Error::Tool(
            "ffmpeg est introuvable : installez-le pour activer le montage".to_string(),
        ));
    }

    // Langue principale : celle de la premiere voix (la langue detectee a la
    // transcription, cf. Conteur) ; le premier .srt lui correspond.
    let langue = projet
        .voix
        .first()
        .map(|voix| voix.langue.clone())
        .ok_or_else(|| Error::Pipeline("projet sans voix off".to_string()))?;
    let srt = projet
        .sous_titres
        .first()
        .cloned()
        .ok_or_else(|| Error::Pipeline("projet sans sous-titres".to_string()))?;

    let mut scenes: Vec<SceneMontage> = Vec::with_capacity(scenario.scenes.len());
    for index in 0..scenario.scenes.len() {
        let visuel = projet
            .visuels
            .iter()
            .find(|asset| asset.scene == index)
            .ok_or_else(|| Error::Pipeline(format!("scene {index} sans visuel")))?;
        let voix = projet
            .voix
            .iter()
            .find(|voix| voix.scene == index && voix.langue == langue)
            .ok_or_else(|| Error::Pipeline(format!("scene {index} sans voix off ({langue})")))?;
        scenes.push(SceneMontage {
            image: visuel.fichier.clone(),
            voix: voix.fichier.clone(),
            duree: voix.duree,
        });
    }

    let dossier = config.data_dir.join(&projet.id);
    // Preview d'abord : en cas d'echec du rendu final, une preview reste
    // disponible pour diagnostiquer.
    ffmpeg::monter(&dossier, &scenes, Some(&srt), &PROFIL_PREVIEW, NOM_PREVIEW).await?;
    ffmpeg::monter(&dossier, &scenes, Some(&srt), &PROFIL_FINAL, NOM_VIDEO).await?;

    projet.preview = Some(NOM_PREVIEW.to_string());
    projet.video = Some(NOM_VIDEO.to_string());
    projet.etat = EtatPipeline::MontagePret;
    if mode == ModeTransition::Auto {
        projet.validation_montage = Some(DecisionValidation::Accepte);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use video_core::asset::{Asset, SourceImage};
    use video_core::config::{
        AudioConfig, LlmConfig, PipelineConfig, Provider, VoixConfig, YoutubeConfig,
    };
    use video_core::scenario::{Scenario, Scene};
    use video_core::voix::VoixScene;

    /// PNG 1x1 valide (rouge), fixture visuelle des scenes.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc,
        0xcf, 0xc0, 0x50, 0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

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
            voix: VoixConfig::default(),
            youtube: YoutubeConfig::default(),
        }
    }

    /// Projet en etat `VoixPretes`, voix acceptees, deux scenes.
    fn projet_voix_acceptees() -> Projet {
        let mut projet = Projet::nouveau("abc123");
        projet.etat = EtatPipeline::VoixPretes;
        projet.validation_scenario = Some(DecisionValidation::Accepte);
        projet.validation_visuels = Some(DecisionValidation::Accepte);
        projet.validation_voix = Some(DecisionValidation::Accepte);
        projet.scenario = Some(Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![
                Scene {
                    narration: "Bonjour le monde.".to_string(),
                    dialogues: vec![],
                    description_visuelle: "Visuel 0".to_string(),
                    duree_cible: 2.0,
                },
                Scene {
                    narration: "Fin de la video.".to_string(),
                    dialogues: vec![],
                    description_visuelle: "Visuel 1".to_string(),
                    duree_cible: 2.0,
                },
            ],
        });
        let visuel = |scene: usize| Asset {
            scene,
            fichier: format!("scene-{scene}.png"),
            source: SourceImage::Openverse,
            titre: None,
            auteur: None,
            url_page: "https://example.org/oeuvre".to_string(),
            url_fichier: "https://example.org/oeuvre.png".to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur: Some(1),
            hauteur: Some(1),
        };
        projet.visuels = vec![visuel(0), visuel(1)];
        let voix = |scene: usize| VoixScene {
            scene,
            langue: "fr".to_string(),
            fichier: format!("voix-{scene}.wav"),
            duree: 1.5,
        };
        projet.voix = vec![voix(0), voix(1)];
        projet.sous_titres = vec!["sous-titres-fr.srt".to_string()];
        projet
    }

    /// Genere un WAV valide (tonalite 440 Hz, PCM 16 bits mono, 8 kHz).
    ///
    /// Une tonalite et non un silence numerique : `loudnorm` produit des
    /// valeurs NaN/Inf sur un silence parfait et fait echouer l'encodage aac.
    fn wav_tonalite(duree_ms: u32) -> Vec<u8> {
        let taille_donnees = duree_ms * 16;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + taille_donnees).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&taille_donnees.to_le_bytes());
        for i in 0..(duree_ms * 8) {
            let echantillon = (f64::sin(2.0 * std::f64::consts::PI * 440.0 * f64::from(i) / 8000.0)
                * 8000.0) as i16;
            wav.extend_from_slice(&echantillon.to_le_bytes());
        }
        wav
    }

    /// Ecrit les fixtures (images PNG, voix WAV, srt) dans le dossier projet.
    fn semer_fichiers(data_dir: &Path, projet: &Projet) {
        let dossier = data_dir.join(&projet.id);
        std::fs::create_dir_all(&dossier).expect("dossier projet");
        for visuel in &projet.visuels {
            std::fs::write(dossier.join(&visuel.fichier), PNG_1X1).expect("image");
        }
        for voix in &projet.voix {
            std::fs::write(dossier.join(&voix.fichier), wav_tonalite(1500)).expect("voix");
        }
        std::fs::write(
            dossier.join("sous-titres-fr.srt"),
            "1\n00:00:00,000 --> 00:00:01,500\nBonjour.\n\n2\n00:00:01,500 --> 00:00:03,000\nFin.\n",
        )
        .expect("srt");
    }

    #[tokio::test]
    async fn refuse_un_projet_hors_voix_pretes() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_voix_acceptees();
        projet.etat = EtatPipeline::VisuelsPrets;

        let resultat = produire_montage(&mut projet, &config, ModeTransition::Validation).await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("VoixPretes"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    #[tokio::test]
    async fn refuse_des_voix_non_acceptees() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_voix_acceptees();
        projet.validation_voix = None; // voix pas encore tranchees

        let resultat = produire_montage(&mut projet, &config, ModeTransition::Validation).await;
        match resultat {
            Err(Error::Pipeline(message)) => {
                assert!(message.contains("acceptation des voix"), "{message}")
            }
            autre => panic!("une erreur Pipeline est attendue, pas {autre:?}"),
        }
    }

    /// Montage reel complet (preview + video) : saute si ffmpeg est absent de
    /// l'environnement (pattern `VIDEO_TEST_RESEAU`).
    #[tokio::test]
    async fn produit_preview_et_video_en_mode_validation() {
        if !ffmpeg::ffmpeg_disponible().await {
            eprintln!("ffmpeg absent : produit_preview_et_video_en_mode_validation ignore.");
            return;
        }
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_voix_acceptees();
        semer_fichiers(temp.path(), &projet);

        produire_montage(&mut projet, &config, ModeTransition::Validation)
            .await
            .expect("le montage doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::MontagePret);
        assert_eq!(projet.video.as_deref(), Some("video.mp4"));
        assert_eq!(projet.preview.as_deref(), Some("preview.mp4"));
        // Mode validation : la decision humaine reste attendue.
        assert_eq!(projet.validation_montage, None);

        let dossier = temp.path().join("abc123");
        for nom in ["video.mp4", "preview.mp4"] {
            let fichier = dossier.join(nom);
            assert!(fichier.exists(), "{nom} absent");
            assert!(std::fs::metadata(&fichier).expect("metadonnees").len() > 0);
        }
        // Duree attendue : 1.5 + 1.5 - fondu 0.5 = 2.5 s (tolerance x264).
        let duree = tools::voix::duree_audio_secondes(&dossier.join("video.mp4"))
            .await
            .expect("duree mesurable via ffprobe");
        assert!((duree - 2.5).abs() < 0.3, "duree inattendue : {duree} s");
    }

    #[tokio::test]
    async fn produit_le_montage_valide_d_office_en_mode_auto() {
        if !ffmpeg::ffmpeg_disponible().await {
            eprintln!("ffmpeg absent : produit_le_montage_valide_d_office_en_mode_auto ignore.");
            return;
        }
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = config_de_test(temp.path());
        let mut projet = projet_voix_acceptees();
        semer_fichiers(temp.path(), &projet);

        produire_montage(&mut projet, &config, ModeTransition::Auto)
            .await
            .expect("le montage doit aboutir");

        assert_eq!(projet.etat, EtatPipeline::MontagePret);
        assert_eq!(projet.validation_montage, Some(DecisionValidation::Accepte));
    }
}
