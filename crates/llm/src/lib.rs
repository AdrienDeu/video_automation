//! Facade LLM du projet, basee sur `rig-core`.
//!
//! Tout le code du workspace passe par cette facade : le provider (Mistral en
//! phase 0, Ollama plus tard) reste interchangeable sans toucher aux agents.

pub mod client;
pub mod hello;
pub mod scenariste;
pub mod visuel;

// Re-exports necessaires aux signatures des crates clients (agents, server) :
// ils manipulent agents, extracteurs et modeles sans dependre de rig-core.
pub use rig_core::agent::Agent;
pub use rig_core::completion::{CompletionModel, Prompt};
pub use rig_core::extractor::Extractor;
