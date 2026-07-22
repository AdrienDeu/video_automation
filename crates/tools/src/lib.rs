//! Outils LLM exposes aux agents (`choisir_image`, `ffmpeg`, `transcrire_audio`,
//! `generer_voix`, `publier_youtube`, `demander_validation`).
//!
//! Phase 1 : `transcrire_audio`. Phase 3 : `images` (choix d'images
//! licenciees). Phase 4 : `voix` (TTS avec cache par hash) et `sous_titres`
//! (`.srt` synchronises). Phase 5 : `ffmpeg` (rendu de la video via whitelist
//! de templates). Phase 6 : `youtube` (upload reprenable via la Data API
//! v3). Les outils sont de simples fonctions testables
//! independamment du LLM ; leur declaration a rig vit dans la facade `llm`.

pub mod ffmpeg;
pub mod images;
pub mod sous_titres;
pub mod transcrire;
pub mod voix;
pub mod youtube;
