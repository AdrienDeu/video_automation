// Appels a l'API du serveur. Toute erreur remonte avec le message texte rendu
// par le serveur : les handlers repondent `(StatusCode, String)`.

import type { DecisionValidation, EtapeValidation, Projet, ProjetResume } from './types';

/** URL d'un fichier du dossier d'un projet (image, voix, .srt, video). */
export function urlFichier(id: string, nom: string): string {
  return `/projet/${encodeURIComponent(id)}/fichier/${encodeURIComponent(nom)}`;
}

async function messageDErreur(reponse: Response): Promise<string> {
  const texte = await reponse.text().catch(() => '');
  return texte || `HTTP ${reponse.status}`;
}

async function getJSON<T>(url: string): Promise<T> {
  const reponse = await fetch(url);
  if (!reponse.ok) throw new Error(await messageDErreur(reponse));
  return (await reponse.json()) as T;
}

// Les handlers renvoient le projet en JSON meme quand l'etape echoue ensuite
// (l'echec est porte par `etat`), d'ou la lecture du corps avant le statut.
async function postJSON<T>(url: string, corps: unknown): Promise<T> {
  const reponse = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(corps),
  });
  const type = reponse.headers.get('content-type') ?? '';
  if (type.includes('application/json')) return (await reponse.json()) as T;
  throw new Error(await messageDErreur(reponse));
}

export function listerProjets(): Promise<ProjetResume[]> {
  return getJSON<ProjetResume[]>('/projets');
}

export function chargerProjet(id: string): Promise<Projet> {
  return getJSON<Projet>(`/projet/${encodeURIComponent(id)}`);
}

/** `POST /audio` : multipart, champ `audio` et champ optionnel `langue`. */
export async function envoyerAudio(fichier: File, langue: string): Promise<Projet> {
  const donnees = new FormData();
  donnees.append('audio', fichier);
  if (langue) donnees.append('langue', langue);

  const reponse = await fetch('/audio', { method: 'POST', body: donnees });
  const type = reponse.headers.get('content-type') ?? '';
  if (!type.includes('application/json')) throw new Error(await messageDErreur(reponse));
  return (await reponse.json()) as Projet;
}

export function valider(
  id: string,
  etape: EtapeValidation,
  decision: DecisionValidation,
): Promise<Projet> {
  return postJSON<Projet>('/valider', { id, etape, decision });
}

export function affiner(id: string, etape: EtapeValidation, prompt: string): Promise<Projet> {
  return postJSON<Projet>('/affiner', { id, etape, prompt });
}

export function remplacerVisuel(id: string, scene: number, requete: string): Promise<Projet> {
  return postJSON<Projet>('/visuel/remplacer', { id, scene, requete });
}

export function annuler(id: string): Promise<Projet> {
  return postJSON<Projet>('/annuler', { id });
}

export function reprendre(id: string): Promise<Projet> {
  return postJSON<Projet>('/reprendre', { id });
}
