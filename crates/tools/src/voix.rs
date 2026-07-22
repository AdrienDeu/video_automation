//! Outil `generer_voix` : synthese vocale (TTS) multi-langue via l'API
//! Mistral (Voxtral TTS, modele `voxtral-mini-tts`), phase 4.
//!
//! **Hypothese d'endpoint** : la forme exacte de l'API Voxtral TTS n'etant
//! pas figee publiquement au moment de la phase 4, le client suppose un
//! endpoint compatible OpenAI `POST /v1/audio/speech` : JSON
//! `{ model, input, voice, response_format }` en entree, octets audio en
//! sortie. L'URL est configurable (`[voix] url` dans `config.toml`) pour
//! s'ajuster sans toucher au code.
//!
//! **Cache par hash** : le nom du fichier audio est derive d'un hash stable
//! du texte, de la langue, de la voix et du modele (`voix-<hash>.mp3` dans le
//! dossier du projet). Un texte deja synthetise reutilise le fichier present,
//! sans nouvel appel reseau — y compris apres un crash ou une regeneration
//! des etapes amont.

use std::path::Path;

use serde::Serialize;
use video_core::config::VoixConfig;
use video_core::error::Error;

/// Longueur maximale d'un segment envoye au TTS (garde-fou, voir
/// `docs/architecture.md` §7).
const LONGUEUR_MAX_SEGMENT: usize = 500;

/// Resultat d'une synthese : fichier audio et duree reelle.
#[derive(Debug, Clone, PartialEq)]
pub struct VoixGeneree {
    /// Nom du fichier audio dans le dossier du projet.
    pub fichier: String,
    /// Duree reelle en secondes, mesuree via ffprobe (`None` si ffprobe est
    /// indisponible ou ne parvient pas a lire le fichier).
    pub duree: Option<f64>,
}

/// Construit le client HTTP des appels TTS (meme User-Agent que les
/// recherches d'images).
pub fn client_http() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .user_agent("video-automation/0.1 (pipeline de videos educatives)")
        .build()
        .map_err(|e| Error::Tool(format!("construction du client HTTP : {e}")))
}

/// Requete JSON de l'endpoint TTS (champs anglais, convention OpenAI).
#[derive(Debug, Serialize)]
struct RequeteTts<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    language: &'a str,
    response_format: &'a str,
}

/// Hash stable (FNV-1a 64 bits, hexadecimal) du quadruplet
/// texte/langue/voix/modele : sert de cle de cache et de nom de fichier.
///
/// Implemente a la main pour rester deterministe d'une version de Rust a
/// l'autre (`std::collections::hash_map::DefaultHasher` ne le garantit pas),
/// sans ajouter de dependance.
pub fn hash_voix(texte: &str, langue: &str, voix: &str, modele: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for octet in [modele, voix, langue, texte]
        .into_iter()
        .flat_map(|partie| partie.bytes().chain([0]))
    {
        hash ^= u64::from(octet);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Mesure la duree d'un fichier audio en secondes via ffprobe (meme approche
/// que `apps/server/src/audio.rs`, dupliquee ici car un crate bibliotheque ne
/// peut pas importer le binaire serveur).
///
/// Retourne `None` si ffprobe est indisponible ou ne parvient pas a lire le
/// fichier : l'appelant retombe alors sur la duree cible de la scene.
pub async fn duree_audio_secondes(chemin: &Path) -> Option<f64> {
    let sortie = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(chemin)
        .output()
        .await
        .ok()?;
    if !sortie.status.success() {
        return None;
    }
    let valeur: serde_json::Value = serde_json::from_slice(&sortie.stdout).ok()?;
    valeur
        .get("format")?
        .get("duration")?
        .as_str()?
        .parse()
        .ok()
}

/// Synthetise un texte en voix off et l'ecrit dans le dossier du projet sous
/// `voix-<hash>.mp3`, ou reutilise le fichier existant si ce meme texte a
/// deja ete synthetise (cache par hash : aucun appel reseau alors).
///
/// # Erreurs
/// `Error::Tool` si le texte est vide ou trop long, si l'appel HTTP echoue,
/// si l'API renvoie un statut d'erreur, ou si l'ecriture du fichier echoue.
pub async fn generer_voix(
    http: &reqwest::Client,
    config: &VoixConfig,
    dossier: &Path,
    texte: &str,
    langue: &str,
    cle_api: &str,
) -> Result<VoixGeneree, Error> {
    let texte = texte.trim();
    if texte.is_empty() {
        return Err(Error::Tool("texte vide envoye au TTS".to_string()));
    }
    if texte.chars().count() > LONGUEUR_MAX_SEGMENT {
        return Err(Error::Tool(format!(
            "segment trop long pour le TTS ({} caracteres, maximum {LONGUEUR_MAX_SEGMENT})",
            texte.chars().count()
        )));
    }

    let nom_fichier = format!(
        "voix-{}.mp3",
        hash_voix(texte, langue, &config.voix, &config.modele)
    );
    let chemin = dossier.join(&nom_fichier);

    // Cache : le fichier existe deja pour ce texte/langue/voix/modele.
    if !chemin.exists() {
        let requete = RequeteTts {
            model: &config.modele,
            input: texte,
            voice: &config.voix,
            language: langue,
            response_format: "mp3",
        };
        let reponse = http
            .post(&config.url)
            .bearer_auth(cle_api)
            .json(&requete)
            .send()
            .await
            .map_err(|e| Error::Tool(format!("appel a l'API de synthese vocale : {e}")))?;
        let statut = reponse.status();
        if !statut.is_success() {
            let detail = reponse.text().await.unwrap_or_default();
            return Err(Error::Tool(format!(
                "l'API de synthese vocale a repondu {statut} : {detail}"
            )));
        }
        let octets = reponse
            .bytes()
            .await
            .map_err(|e| Error::Tool(format!("lecture de l'audio synthetise : {e}")))?;
        if octets.is_empty() {
            return Err(Error::Tool(
                "l'API de synthese vocale a renvoye un audio vide".to_string(),
            ));
        }
        std::fs::create_dir_all(dossier)?;
        std::fs::write(&chemin, &octets)?;
    }

    let duree = duree_audio_secondes(&chemin).await;
    Ok(VoixGeneree {
        fichier: nom_fichier,
        duree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_hash_est_stable_et_sensible_aux_entrees() {
        let reference = hash_voix("Bonjour le monde.", "fr", "default", "voxtral-mini-tts");
        // Deterministe : meme entrees, meme hash.
        assert_eq!(
            hash_voix("Bonjour le monde.", "fr", "default", "voxtral-mini-tts"),
            reference
        );
        assert_eq!(reference.len(), 16);
        // Sensible a chaque entree.
        assert_ne!(
            hash_voix("Bonjour le monde!", "fr", "default", "voxtral-mini-tts"),
            reference
        );
        assert_ne!(
            hash_voix("Bonjour le monde.", "en", "default", "voxtral-mini-tts"),
            reference
        );
        assert_ne!(
            hash_voix("Bonjour le monde.", "fr", "autre", "voxtral-mini-tts"),
            reference
        );
        assert_ne!(
            hash_voix("Bonjour le monde.", "fr", "default", "autre-modele"),
            reference
        );
    }

    #[tokio::test]
    async fn reutilise_le_cache_sans_appel_reseau() {
        // Un fichier nomme d'apres le hash existe deja : generer_voix doit le
        // reutiliser. L'endpoint pointe vers une adresse impossible : tout
        // appel reseau ferait echouer le test.
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = VoixConfig {
            url: "http://127.0.0.1:1/injoignable".to_string(),
            modele: "voxtral-mini-tts".to_string(),
            voix: "default".to_string(),
        };
        let nom = format!(
            "voix-{}.mp3",
            hash_voix("Deja synthetise.", "fr", &config.voix, &config.modele)
        );
        std::fs::write(temp.path().join(&nom), b"faux audio").expect("ecriture du cache");

        let http = reqwest::Client::new();
        let generee = generer_voix(&http, &config, temp.path(), "Deja synthetise.", "fr", "cle")
            .await
            .expect("le cache doit etre reutilise sans reseau");
        assert_eq!(generee.fichier, nom);
        // Le faux fichier n'est pas un vrai audio : duree non mesurable.
        assert_eq!(generee.duree, None);
    }

    #[tokio::test]
    async fn refuse_un_texte_vide_ou_trop_long() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let config = VoixConfig::default();
        let http = reqwest::Client::new();

        let resultat = generer_voix(&http, &config, temp.path(), "   ", "fr", "cle").await;
        assert!(matches!(resultat, Err(Error::Tool(_))));

        let long = "a".repeat(LONGUEUR_MAX_SEGMENT + 1);
        let resultat = generer_voix(&http, &config, temp.path(), &long, "fr", "cle").await;
        match resultat {
            Err(Error::Tool(message)) => assert!(message.contains("trop long"), "{message}"),
            autre => panic!("une erreur Tool est attendue, pas {autre:?}"),
        }
    }

    /// Verification reelle contre l'API de synthese : ignoree tant que
    /// `VIDEO_TEST_RESEAU` n'est pas definie (donc en CI).
    #[tokio::test]
    async fn genere_une_voix_reelle() {
        if std::env::var("VIDEO_TEST_RESEAU").is_err() {
            eprintln!("VIDEO_TEST_RESEAU absente : genere_une_voix_reelle ignore.");
            return;
        }
        let cle = video_core::config::cle_api_mistral().expect("MISTRAL_API_KEY requise");
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let http = reqwest::Client::new();
        let config = VoixConfig::default();
        let generee = generer_voix(&http, &config, temp.path(), "Bonjour le monde.", "fr", &cle)
            .await
            .expect("la synthese doit aboutir");

        assert!(temp.path().join(&generee.fichier).exists());
        assert!(generee.duree.expect("duree mesurable via ffprobe") > 0.0);
    }
}
