//! Facade LLM du projet, basee sur `rig-core`.
//!
//! Tout le code du workspace passe par cette facade : le provider (Mistral en
//! phase 0, Ollama plus tard) reste interchangeable sans toucher aux agents.

pub mod client;
pub mod hello;
