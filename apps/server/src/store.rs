//! Persistance JSON des projets sur disque (MVP de la phase 1 ; la machine a
//! etats SQLite arrive en phase 2).
//!
//! Chaque projet vit dans `<data_dir>/<id>/` : `projet.json` pour l'etat, le
//! fichier audio source a cote.

use std::path::{Path, PathBuf};

use video_core::error::Error;
use video_core::projet::Projet;

/// Dossier des donnees d'un projet : `<data_dir>/<id>/`.
pub fn dossier_projet(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join(id)
}

/// Un identifiant valide est court et alphanumerique : jamais de separateur
/// de chemin, donc aucune traversee de repertoire possible.
pub fn id_valide(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Chemin du fichier d'etat d'un projet.
fn chemin_json(data_dir: &Path, id: &str) -> PathBuf {
    dossier_projet(data_dir, id).join("projet.json")
}

/// Ecrit l'etat du projet dans `data/<id>/projet.json`.
pub async fn sauvegarder(data_dir: &Path, projet: &Projet) -> Result<(), Error> {
    let json =
        serde_json::to_string_pretty(projet).map_err(|e| Error::Persistance(e.to_string()))?;
    tokio::fs::create_dir_all(dossier_projet(data_dir, &projet.id)).await?;
    tokio::fs::write(chemin_json(data_dir, &projet.id), json).await?;
    Ok(())
}

/// Recharge un projet depuis le disque ; `Ok(None)` s'il n'existe pas.
pub async fn charger(data_dir: &Path, id: &str) -> Result<Option<Projet>, Error> {
    match tokio::fs::read(chemin_json(data_dir, id)).await {
        Ok(octets) => {
            let projet =
                serde_json::from_slice(&octets).map_err(|e| Error::Persistance(e.to_string()))?;
            Ok(Some(projet))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_core::etat::EtatPipeline;

    #[test]
    fn valide_les_identifiants() {
        assert!(id_valide("abc123"));
        assert!(!id_valide(""));
        assert!(!id_valide("../secret"));
        assert!(!id_valide("a/b"));
        assert!(!id_valide(&"x".repeat(65)));
    }

    #[tokio::test]
    async fn sauvegarde_puis_recharge_un_projet() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let mut projet = Projet::nouveau("abc123");
        projet.audio = Some("audio.wav".to_string());
        projet.etat = EtatPipeline::Transcrit;

        sauvegarder(temp.path(), &projet).await.expect("sauvegarde");
        assert!(temp.path().join("abc123/projet.json").exists());

        let relu = charger(temp.path(), "abc123")
            .await
            .expect("chargement")
            .expect("le projet existe");
        assert_eq!(relu, projet);

        let absent = charger(temp.path(), "inconnu").await.expect("chargement");
        assert_eq!(absent, None);
    }
}
