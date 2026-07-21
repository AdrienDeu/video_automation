//! Types partages du projet : configuration, erreurs, etats du pipeline,
//! projets video et transcriptions.
//!
//! Ce crate ne depend d'aucun autre crate du workspace ; tous les autres
//! (llm, agents, tools, pipeline, apps) en dependent.

pub mod config;
pub mod error;
pub mod etat;
pub mod projet;
