// Suivi temps reel d'un projet : flux SSE `GET /projet/{id}/events`, avec
// repli sur un polling de 3 s si EventSource est indisponible ou si la
// connexion se ferme definitivement.

import { useEffect, useState } from 'react';

import { chargerProjet } from '../api';
import type { Projet } from '../types';

const PERIODE_POLLING_MS = 3000;

/**
 * Retourne le projet suivi et un setter : les actions (valider, annuler...)
 * renvoient deja le projet a jour, autant l'afficher sans attendre le flux.
 */
export function useSuiviProjet(
  id: string | null,
): [Projet | null, (projet: Projet) => void] {
  const [projet, setProjet] = useState<Projet | null>(null);

  useEffect(() => {
    if (!id) {
      setProjet(null);
      return;
    }
    // Ne pas laisser le projet precedent a l'ecran pendant le chargement.
    setProjet(null);

    let vivant = true;
    let source: EventSource | null = null;
    let minuteur: number | null = null;

    const recharger = async () => {
      try {
        const frais = await chargerProjet(id);
        if (vivant) setProjet(frais);
      } catch {
        // Un rafraichissement rate n'interrompt pas l'interface.
      }
    };

    const demarrerPolling = () => {
      if (minuteur === null) minuteur = window.setInterval(recharger, PERIODE_POLLING_MS);
      void recharger();
    };

    if (typeof EventSource === 'undefined') {
      demarrerPolling();
    } else {
      source = new EventSource(`/projet/${encodeURIComponent(id)}/events`);
      source.addEventListener('projet', (evenement) => {
        try {
          const recu = JSON.parse((evenement as MessageEvent<string>).data) as Projet;
          if (vivant) setProjet(recu);
        } catch {
          // Evenement illisible : le suivant fera foi.
        }
      });
      source.onerror = () => {
        // EventSource retente seul tant que readyState n'est pas CLOSED ; une
        // fermeture definitive (ex. reponse non-SSE) bascule sur le polling.
        if (source && source.readyState === EventSource.CLOSED) {
          source.close();
          source = null;
          demarrerPolling();
        }
      };
    }

    return () => {
      vivant = false;
      source?.close();
      if (minuteur !== null) window.clearInterval(minuteur);
    };
  }, [id]);

  return [projet, setProjet];
}
