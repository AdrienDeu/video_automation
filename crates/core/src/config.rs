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
use crate::etat::ModeTransition;

/// Configuration racine de l'application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Dossier de donnees (un sous-dossier par video).
    pub data_dir: PathBuf,
    /// Adresse d'ecoute du serveur HTTP, ex. `127.0.0.1:8080`.
    pub server_addr: String,
    /// Configuration du fournisseur LLM.
    pub llm: LlmConfig,
    /// Configuration de l'ingestion audio (phase 1).
    #[serde(default)]
    pub audio: AudioConfig,
    /// Modes de transition du pipeline (phase 2).
    #[serde(default)]
    pub pipeline: PipelineConfig,
    /// Configuration du TTS (phase 4).
    #[serde(default)]
    pub voix: VoixConfig,
    /// Configuration de la publication YouTube (phase 6).
    #[serde(default)]
    pub youtube: YoutubeConfig,
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

/// Configuration de l'ingestion audio (`POST /audio`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Duree maximale acceptee pour un audio envoye, en secondes.
    pub duree_max_secondes: u64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            duree_max_secondes: 1800,
        }
    }
}

/// Configuration du TTS (outil `generer_voix`, phase 4).
///
/// L'endpoint est configurable : la forme exacte de l'API Voxtral TTS
/// (`voxtral-mini-tts`) n'etait pas figee publiquement au moment de la phase
/// 4 ; le client suppose un endpoint compatible OpenAI `POST
/// /v1/audio/speech` (JSON en entree, octets audio en sortie).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoixConfig {
    /// URL de l'endpoint TTS.
    pub url: String,
    /// Modele TTS, ex. `voxtral-mini-tts`.
    pub modele: String,
    /// Voix utilisee pour la narration.
    pub voix: String,
}

impl Default for VoixConfig {
    fn default() -> Self {
        Self {
            url: "https://api.mistral.ai/v1/audio/speech".to_string(),
            modele: "voxtral-mini-tts".to_string(),
            voix: "default".to_string(),
        }
    }
}

/// Configuration de la publication YouTube (outil `publier_youtube`, phase 6).
///
/// Aucun secret ici : les identifiants OAuth (client ID/secret, refresh
/// token) passent par l'environnement ou par `data/youtube_token.json`
/// (voir [`secrets_youtube`]), jamais par `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YoutubeConfig {
    /// Visibilite des videos publiees : `private` (defaut prudent),
    /// `unlisted` ou `public`.
    pub visibilite: String,
    /// Tags ajoutes a chaque video publiee.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Nombre maximal d'uploads autorises par jour (garde-fou quota : le
    /// quota API par defaut est de 10 000 unites/jour, un upload coute
    /// 1 600 unites, soit ~6 videos/jour).
    pub quota_uploads_jour: u32,
}

impl Default for YoutubeConfig {
    fn default() -> Self {
        Self {
            visibilite: "private".to_string(),
            tags: Vec::new(),
            quota_uploads_jour: 6,
        }
    }
}

/// Modes de transition du pipeline (voir `docs/architecture.md` §8).
///
/// Chaque etape sensible peut etre `auto` (le pipeline enchaine) ou
/// `validation` (le pipeline bloque jusqu'a une decision via `POST /valider`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Mode de la transition qui suit la generation du scenario.
    #[serde(default)]
    pub scenario: ModeTransition,
    /// Mode de la transition qui suit le choix des visuels.
    #[serde(default)]
    pub visuels: ModeTransition,
    /// Mode de la transition qui suit la generation des voix.
    #[serde(default)]
    pub voix: ModeTransition,
    /// Mode de la transition qui suit le montage ffmpeg (phase 5).
    #[serde(default)]
    pub montage: ModeTransition,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        // Defaut prudent : scenario, visuels, voix et montage sont relus par
        // un humain.
        Self {
            scenario: ModeTransition::Validation,
            visuels: ModeTransition::Validation,
            voix: ModeTransition::Validation,
            montage: ModeTransition::Validation,
        }
    }
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
            audio: AudioConfig::default(),
            pipeline: PipelineConfig::default(),
            voix: VoixConfig::default(),
            youtube: YoutubeConfig::default(),
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

/// Nom du fichier de jeton OAuth YouTube dans le dossier de donnees (ecrit
/// par `cli youtube-auth`, jamais commite : `data/` est gitignore).
pub const FICHIER_JETON_YOUTUBE: &str = "youtube_token.json";

/// Identifiants OAuth YouTube necessaires a la publication (phase 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsYoutube {
    /// Client ID de l'application Google (variable `YOUTUBE_CLIENT_ID`).
    pub client_id: String,
    /// Client secret de l'application Google (variable `YOUTUBE_CLIENT_SECRET`).
    pub client_secret: String,
    /// Refresh token obtenu via `cli youtube-auth`.
    pub refresh_token: String,
}

/// Resout les identifiants OAuth YouTube.
///
/// Le client ID et le client secret viennent des variables
/// `YOUTUBE_CLIENT_ID` / `YOUTUBE_CLIENT_SECRET`. Le refresh token vient de
/// `YOUTUBE_REFRESH_TOKEN` si definie, sinon du fichier
/// `<data_dir>/youtube_token.json` ecrit par `cli youtube-auth`. Retourne
/// `None` si l'un des trois manque : la publication est alors desactivee.
pub fn secrets_youtube(data_dir: &std::path::Path) -> Option<SecretsYoutube> {
    let variable = |nom: &str| std::env::var(nom).ok().filter(|v| !v.is_empty());
    let client_id = variable("YOUTUBE_CLIENT_ID")?;
    let client_secret = variable("YOUTUBE_CLIENT_SECRET")?;
    let refresh_token = variable("YOUTUBE_REFRESH_TOKEN").or_else(|| {
        let contenu = std::fs::read_to_string(data_dir.join(FICHIER_JETON_YOUTUBE)).ok()?;
        serde_json::from_str::<serde_json::Value>(&contenu)
            .ok()?
            .get("refresh_token")?
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    })?;
    Some(SecretsYoutube {
        client_id,
        client_secret,
        refresh_token,
    })
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

    #[test]
    fn section_audio_optionnelle() {
        // Sans section [audio] : les valeurs par defaut s'appliquent.
        let toml = r#"
            data_dir = "data"
            server_addr = "127.0.0.1:8080"

            [llm]
            provider = "mistral"
            model = "mistral-large-latest"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML sans [audio] doit etre valide");
        assert_eq!(config.audio, AudioConfig::default());

        // Avec une section [audio] explicite.
        let toml = format!("{toml}\n[audio]\nduree_max_secondes = 600\n");
        let config: Config = Figment::from(Toml::string(&toml))
            .extract()
            .expect("le TOML avec [audio] doit etre valide");
        assert_eq!(config.audio.duree_max_secondes, 600);
    }

    #[test]
    fn section_pipeline_optionnelle() {
        // Sans section [pipeline] : validation du scenario par defaut.
        let toml = r#"
            data_dir = "data"
            server_addr = "127.0.0.1:8080"

            [llm]
            provider = "mistral"
            model = "mistral-large-latest"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML sans [pipeline] doit etre valide");
        assert_eq!(config.pipeline, PipelineConfig::default());
        assert_eq!(config.pipeline.scenario, ModeTransition::Validation);
        assert_eq!(config.pipeline.voix, ModeTransition::Validation);
        assert_eq!(config.pipeline.montage, ModeTransition::Validation);

        // Avec une section [pipeline] explicite.
        let toml = format!("{toml}\n[pipeline]\nscenario = \"auto\"\nmontage = \"auto\"\n");
        let config: Config = Figment::from(Toml::string(&toml))
            .extract()
            .expect("le TOML avec [pipeline] doit etre valide");
        assert_eq!(config.pipeline.scenario, ModeTransition::Auto);
        assert_eq!(config.pipeline.montage, ModeTransition::Auto);
    }

    #[test]
    fn section_youtube_optionnelle() {
        // Sans section [youtube] : visibilite private et quota de 6 uploads.
        let toml = r#"
            data_dir = "data"
            server_addr = "127.0.0.1:8080"

            [llm]
            provider = "mistral"
            model = "mistral-large-latest"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML sans [youtube] doit etre valide");
        assert_eq!(config.youtube, YoutubeConfig::default());
        assert_eq!(config.youtube.visibilite, "private");
        assert_eq!(config.youtube.quota_uploads_jour, 6);

        // Avec une section [youtube] explicite.
        let toml = format!(
            "{toml}\n[youtube]\nvisibilite = \"unlisted\"\ntags = [\"education\", \"test\"]\nquota_uploads_jour = 3\n"
        );
        let config: Config = Figment::from(Toml::string(&toml))
            .extract()
            .expect("le TOML avec [youtube] doit etre valide");
        assert_eq!(config.youtube.visibilite, "unlisted");
        assert_eq!(config.youtube.tags, vec!["education", "test"]);
        assert_eq!(config.youtube.quota_uploads_jour, 3);
    }

    #[test]
    fn section_voix_optionnelle() {
        // Sans section [voix] : endpoint, modele et voix par defaut.
        let toml = r#"
            data_dir = "data"
            server_addr = "127.0.0.1:8080"

            [llm]
            provider = "mistral"
            model = "mistral-large-latest"
        "#;
        let config: Config = Figment::from(Toml::string(toml))
            .extract()
            .expect("le TOML sans [voix] doit etre valide");
        assert_eq!(config.voix, VoixConfig::default());

        // Avec une section [voix] explicite (endpoint TTS interchangeable).
        let toml = format!(
            "{toml}\n[voix]\nurl = \"http://127.0.0.1:5000/tts\"\nmodele = \"piper\"\nvoix = \"alice\"\n"
        );
        let config: Config = Figment::from(Toml::string(&toml))
            .extract()
            .expect("le TOML avec [voix] doit etre valide");
        assert_eq!(config.voix.url, "http://127.0.0.1:5000/tts");
        assert_eq!(config.voix.modele, "piper");
        assert_eq!(config.voix.voix, "alice");
    }
}
