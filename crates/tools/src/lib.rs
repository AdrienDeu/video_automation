//! Outils LLM exposes aux agents (`choisir_image`, `ffmpeg`, `transcrire_audio`,
//! `generer_voix`, `publier_youtube`, `demander_validation`).
//!
//! Phase 1 : `transcrire_audio`. Phase 3 : `images` (choix d'images
//! licenciees). Phase 4 : `voix` (TTS avec cache par hash) et `sous_titres`
//! (`.srt` synchronises). Les outils sont de simples fonctions testables
//! independamment du LLM ; leur declaration a rig vit dans la facade `llm`.

pub mod images;
pub mod sous_titres;
pub mod transcrire;
pub mod voix;
