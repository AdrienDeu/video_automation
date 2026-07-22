//! Agents specialises (Realisateur, Scenariste, Visuel, Conteur, Monteur).
//!
//! Phase 2 : le Realisateur v1 orchestre l'enchainement transcription →
//! scenario. Phase 3 : le Visuel illustre chaque scene validee avec une image
//! licenciee. Phase 4 : le Conteur double chaque scene d'une voix off
//! synthetisee et ecrit les sous-titres. Phase 5 : le Monteur assemble les
//! assets en video finale 1080p et preview, via les templates ffmpeg. Le
//! Scenariste vit dans la facade `llm` (`llm::scenariste`) : ce n'est qu'une
//! extraction structuree, sans boucle d'outils propre.

pub mod conteur;
pub mod monteur;
pub mod realisateur;
pub mod visuel;
