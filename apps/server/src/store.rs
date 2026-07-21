//! Rangement des fichiers des projets sur disque.
//!
//! Chaque projet vit dans `<data_dir>/<id>/` : le fichier audio source et,
//! dans les phases suivantes, les images, voix et rendus. L'etat du projet
//! lui-meme est persiste en SQLite par `pipeline::stockage` (phase 2).

use std::path::{Path, PathBuf};

/// Dossier des donnees d'un projet : `<data_dir>/<id>/`.
pub fn dossier_projet(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join(id)
}

/// Un identifiant valide est court et alphanumerique : jamais de separateur
/// de chemin, donc aucune traversee de repertoire possible.
pub fn id_valide(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valide_les_identifiants() {
        assert!(id_valide("abc123"));
        assert!(!id_valide(""));
        assert!(!id_valide("../secret"));
        assert!(!id_valide("a/b"));
        assert!(!id_valide(&"x".repeat(65)));
    }
}
