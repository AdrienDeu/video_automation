//! Controle de duree des fichiers audio via `ffprobe` (livre avec ffmpeg,
//! brique centrale du projet).

use std::path::Path;

/// Mesure la duree d'un fichier audio en secondes via ffprobe.
///
/// Retourne `None` si ffprobe est indisponible ou ne parvient pas a lire le
/// fichier : le controle est alors simplement saute, la validation definitive
/// ayant de toute facon lieu cote API de transcription.
pub async fn duree_secondes(chemin: &Path) -> Option<f64> {
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
    extraire_duree(std::str::from_utf8(&sortie.stdout).ok()?)
}

/// Extrait `format.duration` de la sortie JSON de ffprobe.
fn extraire_duree(json: &str) -> Option<f64> {
    let valeur: serde_json::Value = serde_json::from_str(json).ok()?;
    valeur
        .get("format")?
        .get("duration")?
        .as_str()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrait_la_duree_de_la_sortie_ffprobe() {
        let json = r#"{ "format": { "duration": "123.456", "size": "1000" } }"#;
        assert_eq!(extraire_duree(json), Some(123.456));
    }

    #[test]
    fn ignore_une_sortie_inexploitable() {
        assert_eq!(extraire_duree("pas du json"), None);
        assert_eq!(extraire_duree(r#"{ "format": {} }"#), None);
        assert_eq!(
            extraire_duree(r#"{ "format": { "duration": "N/A" } }"#),
            None
        );
    }
}
