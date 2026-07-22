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

/// Resume leger d'un projet, pour la liste affichee par l'interface web.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjetResume {
    /// Identifiant du projet.
    pub id: String,
    /// Etiquette d'etat (ex. `audio_recu`, `scenario_genere`, `erreur`).
    pub etat: String,
    /// Date de derniere mise a jour (`datetime('now')` SQLite, UTC).
    pub maj: String,
}

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
        // Compteur d'uploads YouTube par jour (garde-fou quota, phase 6) :
        // la date UTC du jour sert de cle, le compteur repart donc de zero
        // chaque jour.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS quota_uploads (
                jour    TEXT PRIMARY KEY,
                uploads INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(erreur_sql)?;
        Ok(())
    }

    /// Nombre d'uploads YouTube comptabilises pour le jour courant (UTC).
    pub async fn uploads_du_jour(&self) -> Result<u32, Error> {
        let ligne = sqlx::query("SELECT uploads FROM quota_uploads WHERE jour = date('now')")
            .fetch_optional(&self.pool)
            .await
            .map_err(erreur_sql)?;
        match ligne {
            Some(ligne) => {
                let uploads: i64 = ligne
                    .try_get("uploads")
                    .map_err(|e| Error::Persistance(e.to_string()))?;
                Ok(uploads.max(0) as u32)
            }
            None => Ok(0),
        }
    }

    /// Comptabilise un upload YouTube et renvoie le nouveau total du jour.
    pub async fn incrementer_uploads(&self) -> Result<u32, Error> {
        let ligne = sqlx::query(
            "INSERT INTO quota_uploads (jour, uploads) VALUES (date('now'), 1)
             ON CONFLICT (jour) DO UPDATE SET uploads = uploads + 1
             RETURNING uploads",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(erreur_sql)?;
        let uploads: i64 = ligne
            .try_get("uploads")
            .map_err(|e| Error::Persistance(e.to_string()))?;
        Ok(uploads.max(0) as u32)
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

    /// Liste les projets connus, du plus recemment mis a jour au plus ancien.
    pub async fn lister(&self) -> Result<Vec<ProjetResume>, Error> {
        let lignes = sqlx::query("SELECT id, etat, maj FROM projets ORDER BY maj DESC, id")
            .fetch_all(&self.pool)
            .await
            .map_err(erreur_sql)?;
        let mut resumes = Vec::with_capacity(lignes.len());
        for ligne in lignes {
            let champ = |nom: &str| {
                ligne
                    .try_get::<String, _>(nom)
                    .map_err(|e| Error::Persistance(e.to_string()))
            };
            resumes.push(ProjetResume {
                id: champ("id")?,
                etat: etiquette_etat(&champ("etat")?),
                maj: champ("maj")?,
            });
        }
        Ok(resumes)
    }
}

/// La colonne `etat` stocke la valeur serde JSON de `EtatPipeline`
/// (`"audio_recu"` ou `{"erreur":"..."}`) ; on la reduit a une etiquette
/// lisible pour la liste (le detail de l'erreur reste dans `donnees`).
fn etiquette_etat(brut: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(brut) {
        Ok(serde_json::Value::String(etat)) => etat,
        Ok(serde_json::Value::Object(variante)) => {
            variante.keys().next().cloned().unwrap_or_default()
        }
        _ => brut.to_string(),
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

    #[tokio::test]
    async fn liste_les_projets_du_plus_recent_au_plus_ancien() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");

        stockage
            .sauvegarder(&Projet::nouveau("ancien"))
            .await
            .expect("sauvegarde");
        // Les deux insertions peuvent tomber dans la meme seconde : on fige
        // la date du premier pour rendre l'ordre deterministe.
        sqlx::query("UPDATE projets SET maj = '2020-01-01 00:00:00' WHERE id = 'ancien'")
            .execute(&stockage.pool)
            .await
            .expect("mise a jour de la date");
        let mut recent = Projet::nouveau("recent");
        recent.etat = EtatPipeline::Erreur("STT injoignable".to_string());
        stockage.sauvegarder(&recent).await.expect("sauvegarde");

        let resumes = stockage.lister().await.expect("liste");
        assert_eq!(resumes.len(), 2);
        assert_eq!(resumes[0].id, "recent");
        assert_eq!(resumes[0].etat, "erreur");
        assert_eq!(resumes[1].id, "ancien");
        assert_eq!(resumes[1].etat, "audio_recu");
    }

    #[tokio::test]
    async fn compte_les_uploads_du_jour() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");

        assert_eq!(stockage.uploads_du_jour().await.expect("compteur"), 0);
        assert_eq!(stockage.incrementer_uploads().await.expect("increment"), 1);
        assert_eq!(stockage.incrementer_uploads().await.expect("increment"), 2);
        assert_eq!(stockage.uploads_du_jour().await.expect("compteur"), 2);
    }

    #[tokio::test]
    async fn le_compteur_repart_a_zero_chaque_jour() {
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let stockage = Stockage::ouvrir(temp.path()).await.expect("ouverture");

        // Un compteur eleve date d'hier : il ne doit pas compter pour
        // aujourd'hui.
        sqlx::query("INSERT INTO quota_uploads (jour, uploads) VALUES (date('now', '-1 day'), 42)")
            .execute(&stockage.pool)
            .await
            .expect("insertion d'un compteur ancien");
        assert_eq!(stockage.uploads_du_jour().await.expect("compteur"), 0);
        assert_eq!(stockage.incrementer_uploads().await.expect("increment"), 1);

        // Le compteur d'hier est conserve tel quel.
        let hier: i64 =
            sqlx::query("SELECT uploads FROM quota_uploads WHERE jour = date('now', '-1 day')")
                .fetch_one(&stockage.pool)
                .await
                .expect("lecture du compteur ancien")
                .try_get("uploads")
                .expect("colonne uploads");
        assert_eq!(hier, 42);
    }
}
