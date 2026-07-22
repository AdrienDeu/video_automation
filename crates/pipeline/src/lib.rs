//! Machine a etats, persistance et validations humaines du pipeline.
//!
//! Phase 2 (voir `docs/agenda.md`) :
//! - `stockage` : persistance SQLite des projets (`data/pipeline.db`),
//!   remplace le JSON par projet de la phase 1 ;
//! - `validation` : porte de validation humaine du scenario
//!   (`POST /valider`).
//!
//! Phase 7 :
//! - `affiner` : regeneration ciblee d'une etape avec invalidation de l'aval
//!   (`POST /affiner`).
//!
//! Les types d'etat sont definis dans `video_core::etat`.

pub mod affiner;
pub mod stockage;
pub mod validation;
