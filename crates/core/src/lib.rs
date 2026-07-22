//! Types partages du projet : configuration, erreurs, etats du pipeline,
//! projets video, transcriptions et voix off.
//!
//! Ce crate ne depend d'aucun autre crate du workspace ; tous les autres
//! (llm, agents, tools, pipeline, apps) en dependent.

pub mod asset;
pub mod config;
pub mod error;
pub mod etat;
pub mod projet;
pub mod scenario;
pub mod voix;
