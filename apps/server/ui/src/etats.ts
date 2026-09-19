// Lecture de la machine a etats du pipeline cote interface : libelles, ordre
// des etapes, et traduction d'un etat en ton de badge du design system.

import type { EtapeValidation, EtatPipeline, Projet } from './types';

/** Etat normalise : un code et, pour `erreur`, le message du serveur. */
export interface EtatNormalise {
  code: string;
  message: string | null;
}

export function normaliserEtat(etat: EtatPipeline): EtatNormalise {
  if (typeof etat === 'string') return { code: etat, message: null };
  if (etat && typeof etat.erreur === 'string') return { code: 'erreur', message: etat.erreur };
  return { code: 'inconnu', message: null };
}

const LIBELLES_ETATS: Record<string, string> = {
  audio_recu: 'Audio reçu',
  transcrit: 'Transcrit',
  scenario_genere: 'Scénario généré',
  visuels_prets: 'Visuels prêts',
  voix_pretes: 'Voix prêtes',
  montage_pret: 'Montage prêt',
  publie: 'Publié',
  annule: 'Annulé',
  erreur: 'Erreur',
};

export function libelleEtat(code: string): string {
  return LIBELLES_ETATS[code] ?? code;
}

/** Etapes du pipeline dans l'ordre, pour la progression du detail. */
export const ETAPES_PIPELINE: ReadonlyArray<readonly [string, string]> = [
  ['audio_recu', 'Audio'],
  ['transcrit', 'Transcription'],
  ['scenario_genere', 'Scénario'],
  ['visuels_prets', 'Visuels'],
  ['voix_pretes', 'Voix'],
  ['montage_pret', 'Montage'],
  ['publie', 'Publié'],
];

export const LIBELLES_ETAPES: Record<EtapeValidation, string> = {
  scenario: 'le scénario',
  visuels: 'les visuels',
  voix: 'les voix',
  montage: 'le montage',
};

export type TonBadge = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

/**
 * Ton du badge d'un etat. Le design system reserve la couleur au sens : `accent`
 * pour une tache en cours, `success` / `warning` / `danger` pour une fin de
 * tache, `neutral` pour un etat de repos.
 */
export function tonEtat(code: string): TonBadge {
  switch (code) {
    case 'publie':
      return 'success';
    case 'erreur':
      return 'danger';
    case 'annule':
      return 'warning';
    case 'audio_recu':
      return 'neutral';
    default:
      return 'accent';
  }
}

/** Avancement du pipeline en pourcentage, pour la barre de progression. */
export function progression(code: string): number {
  const index = ETAPES_PIPELINE.findIndex(([etat]) => etat === code);
  if (index < 0) return 0;
  return Math.round(((index + 1) / ETAPES_PIPELINE.length) * 100);
}

/**
 * Etape en attente d'une decision humaine, s'il y en a une : seule celle qui
 * correspond a l'etat courant et n'a pas encore ete tranchee.
 */
export function etapeEnAttente(projet: Projet, code: string): EtapeValidation | null {
  if (code === 'scenario_genere' && projet.validation_scenario == null) return 'scenario';
  if (code === 'visuels_prets' && projet.validation_visuels == null) return 'visuels';
  if (code === 'voix_pretes' && projet.validation_voix == null) return 'voix';
  if (code === 'montage_pret' && projet.validation_montage == null) return 'montage';
  return null;
}
