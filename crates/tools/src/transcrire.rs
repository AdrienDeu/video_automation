//! Outil `transcrire_audio` : transcription d'un fichier audio via l'API
//! Mistral (Voxtral Transcribe 2, modele `voxtral-mini-latest`).
//!
//! L'API `POST /v1/audio/transcriptions` accepte un multipart avec le fichier
//! et retourne le texte integral plus des segments horodates lorsque
//! `timestamp_granularities = "segment"` est demande.

use std::path::Path;

use serde::Deserialize;
use video_core::error::Error;
use video_core::projet::{Segment, Transcription};

/// Endpoint de transcription de l'API Mistral.
const URL_TRANSCRIPTION: &str = "https://api.mistral.ai/v1/audio/transcriptions";

/// Modele Voxtral Transcribe 2 cote API hebergee.
const MODELE_STT: &str = "voxtral-mini-latest";

/// Reponse brute de l'API `/v1/audio/transcriptions`.
#[derive(Debug, Deserialize)]
struct ReponseApi {
    text: String,
    language: Option<String>,
    #[serde(default)]
    segments: Vec<SegmentApi>,
}

/// Segment brut retourne par l'API (champs anglais, convention Mistral).
#[derive(Debug, Deserialize)]
struct SegmentApi {
    start: f64,
    end: f64,
    text: String,
}

/// Convertit la reponse brute de l'API en type partage du projet.
fn en_transcription(api: ReponseApi) -> Transcription {
    Transcription {
        texte: api.text,
        langue: api.language,
        segments: api
            .segments
            .into_iter()
            .map(|s| Segment {
                debut: s.start,
                fin: s.end,
                texte: s.text,
            })
            .collect(),
    }
}

/// Type MIME a joindre au fichier dans le multipart, d'apres son extension.
fn mime_pour_extension(extension: &str) -> &'static str {
    match extension {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        "webm" => "audio/webm",
        _ => "application/octet-stream",
    }
}

/// Transcrit un fichier audio via l'API Mistral et retourne le texte
/// accompagne des segments horodates.
///
/// `langue` est un code ISO optionnel (ex. `"fr"`) ; sans indication, la
/// langue est detectee automatiquement.
///
/// # Erreurs
/// `Error::Tool` si le fichier est illisible, si l'appel HTTP echoue ou si
/// l'API renvoie un statut d'erreur (le detail de l'API est conserve).
pub async fn transcrire_audio(
    fichier: &Path,
    langue: Option<&str>,
    cle_api: &str,
) -> Result<Transcription, Error> {
    let octets = std::fs::read(fichier)
        .map_err(|e| Error::Tool(format!("lecture de {} : {e}", fichier.display())))?;

    let nom = fichier
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".to_string());
    let extension = fichier
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let partie = reqwest::multipart::Part::bytes(octets)
        .file_name(nom)
        .mime_str(mime_pour_extension(&extension))
        .map_err(|e| Error::Tool(format!("type MIME invalide : {e}")))?;

    let mut formulaire = reqwest::multipart::Form::new()
        .text("model", MODELE_STT)
        .text("timestamp_granularities", "segment")
        .part("file", partie);
    if let Some(code) = langue {
        formulaire = formulaire.text("language", code.to_string());
    }

    let reponse = reqwest::Client::new()
        .post(URL_TRANSCRIPTION)
        .bearer_auth(cle_api)
        .multipart(formulaire)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("appel a l'API de transcription : {e}")))?;

    let statut = reponse.status();
    if !statut.is_success() {
        let detail = reponse.text().await.unwrap_or_default();
        return Err(Error::Tool(format!(
            "l'API de transcription a repondu {statut} : {detail}"
        )));
    }

    let api: ReponseApi = reponse
        .json()
        .await
        .map_err(|e| Error::Tool(format!("reponse de transcription illisible : {e}")))?;
    Ok(en_transcription(api))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_la_reponse_de_l_api() {
        // Forme reelle de la reponse Mistral (segments horodates).
        let json = r#"{
            "model": "voxtral-mini-latest",
            "text": " Bonjour le monde.",
            "language": "fr",
            "segments": [
                { "start": 0.0, "end": 1.4, "text": " Bonjour" },
                { "start": 1.4, "end": 2.6, "text": " le monde." }
            ],
            "usage": { "prompt_audio_seconds": 3 }
        }"#;
        let api: ReponseApi = serde_json::from_str(json).expect("le JSON de test est valide");
        let transcription = en_transcription(api);

        assert_eq!(transcription.texte, " Bonjour le monde.");
        assert_eq!(transcription.langue.as_deref(), Some("fr"));
        assert_eq!(transcription.segments.len(), 2);
        assert_eq!(transcription.segments[0].debut, 0.0);
        assert_eq!(transcription.segments[1].fin, 2.6);
        assert_eq!(transcription.segments[1].texte, " le monde.");
    }

    #[test]
    fn parse_une_reponse_sans_segments() {
        // `segments` peut etre absent si les timestamps ne sont pas demandes.
        let json = r#"{ "model": "voxtral-mini-latest", "text": "Salut.", "language": null }"#;
        let api: ReponseApi = serde_json::from_str(json).expect("le JSON de test est valide");
        let transcription = en_transcription(api);

        assert_eq!(transcription.texte, "Salut.");
        assert_eq!(transcription.langue, None);
        assert!(transcription.segments.is_empty());
    }

    #[test]
    fn mime_selon_extension() {
        assert_eq!(mime_pour_extension("mp3"), "audio/mpeg");
        assert_eq!(mime_pour_extension("m4a"), "audio/mp4");
        assert_eq!(mime_pour_extension("ogg"), "audio/ogg");
        assert_eq!(mime_pour_extension("3gp"), "application/octet-stream");
    }
}
