//! Outils LLM exposes aux agents (`choisir_image`, `ffmpeg`, `transcrire_audio`,
//! `generer_voix`, `publier_youtube`, `demander_validation`).
//!
//! Phase 1 : seul `transcrire_audio` est implemente (voir `docs/agenda.md`).
//! Les outils sont de simples fonctions testables independamment du LLM ; leur
//! declaration a rig arrive avec les agents (phase 2).

pub mod transcrire;
