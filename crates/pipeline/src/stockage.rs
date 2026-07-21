//! Persistance SQLite des projets (`<data_dir>/pipeline.db`), phase 2.
//!
//! Un projet est stocke en JSON integral dans la colonne `donnees` : les
//! types serde de `video_core` restent la source de verite, le SQL ne sert
//! qu'a indexer (etat, date de mise a jour) pour les requetes futures
//! (file de taches, reprise apres crash). Les fichiers lourds (audio,
//! images, voix) restent sur disque dans `<data_dir>/<id>/`.
//!
//! Les requetes sont ecrites sans macros sqlx : aucune base reelle n'est
//! requise a la compilation.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use video_core::error::Error;
use video_core::projet::Projet;

/// Nom du fichier de base de donnees dans le dossier de donnees.
const FICHIER_DB: &str = "pipeline.db";

/// Acces a la base SQLite du pipeline.
#[derive(Clone)]
pub struct Stockage {
    pool: SqlitePool,
}

impl Stockage {
    /// Ouvre (ou cree) la base `<data_dir>/pipeline.db` et applique le schema.
    pub async fn ouvrir(data_dir: &Path) -> Result<Self, Error> {
        tokio::fs::create_dir_all(data_dir).await?;
        let options = SqliteConnectOptions::new()
            .filename(data_dir.join(FICHIER_DB))
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(erreur_sql)?;
        let stockage = Self { pool };
        stockage.initialiser().await?;
        Ok(stockage)
    }

    /// Cree la table des projets si elle n'existe pas.
    async fn initialiser(&self) -> Result<(), Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS projets (
                id     TEXT PRIMARY KEY,
                etat   TEXT NOT NULL,
                donnees TEXT NOT NULL,
                maj    TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(erreur_sql)?;
        Ok(())
    }

    /// Insere ou remplace l'etat d'un projet.
    pub async fn sauvegarder(&self, projet: &Projet) -> Result<(), Error> {
        let donnees =
            serde_json::to_string(projet).map_err(|e| Error::Persistance(e.to_string()))?;
        let etat =
            serde_json::to_string(&projet.etat).map_err(|e| Error::Persistance(e.to_string()))?;
        sqlx::query(
            "INSERT INTO projets (id, etat, donnees, maj)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT (id) DO UPDATE SET
                etat = excluded.etat,
                donnees = excluded.donnees,
                maj = excluded.maj",
        )
        .bind(&projet.id)
        .bind(etat)
        .bind(donnees)
        .execute(&self.pool)
        .await
        .map_err(erreur_sql)?;
        Ok(())
    }

    /// Recharge un projet par son identifiant ; `Ok(None)` s'il n'existe pas.
    pub async fn charger(&self, id: &str) -> Result<Option<Projet>, Error> {
        let ligne = sqlx::query("SELECT donnees FROM projets WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(erreur_sql)?;
        match ligne {
            Some(ligne) => {
                let donnees: &str = ligne
                    .try_get("donnees")
                    .map_err(|e| Error::Persistance(e.to_string()))?;
                let projet =
                    serde_json::from_str(donnees).map_err(|e| Error::Persistance(e.to_string()))?;
                Ok(Some(projet))
            }
            None => Ok(None),
        }
    }
}

/// Traduit une erreur sqlx en erreur centrale.
fn erreur_sql(e: sqlx::Error) -> Error {
    Error::Persistance(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_core::etat::EtatPipeline;

    #[tokio::test]
    async fn sauvegarde_puis_recharge_un_projet() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");
        assert!(temp.path().join(FICHIER_DB).exists());

        let mut projet = Projet::nouveau("abc123");
        projet.audio = Some("audio.wav".to_string());
        projet.etat = EtatPipeline::Transcrit;
        stockage.sauvegarder(&projet).await.expect("sauvegarde");

        let relu = stockage
            .charger("abc123")
            .await
            .expect("chargement")
            .expect("le projet existe");
        assert_eq!(relu, projet);

        let absent = stockage.charger("inconnu").await.expect("chargement");
        assert_eq!(absent, None);
    }

    #[tokio::test]
    async fn la_sauvegarde_remplace_l_etat_precedent() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");

        let mut projet = Projet::nouveau("abc123");
        stockage.sauvegarder(&projet).await.expect("sauvegarde");
        projet.etat = EtatPipeline::ScenarioGenere;
        stockage.sauvegarder(&projet).await.expect("mise a jour");

        let relu = stockage
            .charger("abc123")
            .await
            .expect("chargement")
            .expect("le projet existe");
        assert_eq!(relu.etat, EtatPipeline::ScenarioGenere);
    }
}
