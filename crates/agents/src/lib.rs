//! Agents specialises (Realisateur, Scenariste, Visuel, Conteur, Monteur).
//!
//! Phase 2 : le Realisateur v1 orchestre l'enchainement transcription →
//! scenario. Le Scenariste vit dans la facade `llm` (`llm::scenariste`) :
//! ce n'est qu'une extraction structuree, sans boucle d'outils propre.
//! Les agents Visuel, Conteur et Monteur arrivent en phases 3 a 5.

pub mod realisateur;
