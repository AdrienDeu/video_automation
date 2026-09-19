// Types de l'API du serveur : miroir des types serde de `video_core`
// (`projet.rs`, `etat.rs`, `scenario.rs`, `asset.rs`, `voix.rs`).

/** `EtatPipeline` : une chaine snake_case, ou `{ erreur: "..." }`. */
export type EtatPipeline =
  | 'audio_recu'
  | 'transcrit'
  | 'scenario_genere'
  | 'visuels_prets'
  | 'voix_pretes'
  | 'montage_pret'
  | 'publie'
  | 'annule'
  | { erreur: string };

export type DecisionValidation = 'accepte' | 'rejete';

export type EtapeValidation = 'scenario' | 'visuels' | 'voix' | 'montage';

export interface Segment {
  debut: number;
  fin: number;
  texte: string;
}

export interface Transcription {
  texte: string;
  langue: string | null;
  segments: Segment[];
}

export interface Dialogue {
  personnage: string;
  replique: string;
}

export interface Scene {
  narration: string;
  dialogues: Dialogue[];
  description_visuelle: string;
  duree_cible: number;
}

export interface Scenario {
  titre: string;
  public: string;
  style_images: string;
  scenes: Scene[];
}

export interface Asset {
  scene: number;
  fichier: string;
  source: 'openverse' | 'wikimedia_commons';
  titre: string | null;
  auteur: string | null;
  url_page: string;
  url_fichier: string;
  licence: string;
  licence_url: string | null;
  largeur: number | null;
  hauteur: number | null;
}

export interface VoixScene {
  scene: number;
  langue: string;
  fichier: string;
  duree: number;
}

export interface PublicationYoutube {
  id_video: string;
  url: string;
}

export interface Projet {
  id: string;
  etat: EtatPipeline;
  audio: string | null;
  transcription: Transcription | null;
  scenario: Scenario | null;
  validation_scenario: DecisionValidation | null;
  visuels: Asset[];
  validation_visuels: DecisionValidation | null;
  voix: VoixScene[];
  sous_titres: string[];
  validation_voix: DecisionValidation | null;
  video: string | null;
  preview: string | null;
  validation_montage: DecisionValidation | null;
  youtube: PublicationYoutube | null;
}

/** Ligne de `GET /projets` : l'etat y est deja reduit a une etiquette. */
export interface ProjetResume {
  id: string;
  etat: string;
  maj: string;
}
