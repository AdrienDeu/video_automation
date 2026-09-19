// Interface du pipeline video : depot d'une dictee, liste des projets, et
// detail du projet selectionne suivi en temps reel.

import { useCallback, useEffect, useState } from 'react';

import { listerProjets } from './api';
import { DetailProjet } from './composants/DetailProjet';
import { FormulaireUpload } from './composants/FormulaireUpload';
import { ListeProjets } from './composants/ListeProjets';
import { etapeEnAttente, normaliserEtat } from './etats';
import { useSuiviProjet } from './hooks/useSuiviProjet';
import type { Projet, ProjetResume } from './types';

const PERIODE_LISTE_MS = 5000;

export function App() {
  const [projets, setProjets] = useState<ProjetResume[]>([]);
  const [selection, setSelection] = useState<string | null>(null);
  const [projet, setProjet] = useSuiviProjet(selection);

  const rafraichirListe = useCallback(async () => {
    try {
      setProjets(await listerProjets());
    } catch {
      // Un echec de rafraichissement n'interrompt pas l'interface.
    }
  }, []);

  useEffect(() => {
    void rafraichirListe();
    const minuteur = window.setInterval(rafraichirListe, PERIODE_LISTE_MS);
    return () => window.clearInterval(minuteur);
  }, [rafraichirListe]);

  // L'etat du projet suivi vient de changer : la liste le montre aussi.
  const etatSuivi = projet ? JSON.stringify(projet.etat) : null;
  useEffect(() => {
    if (etatSuivi !== null) void rafraichirListe();
  }, [etatSuivi, rafraichirListe]);

  function ouvrirProjetCree(cree: Projet) {
    void rafraichirListe();
    setSelection(cree.id);
  }

  // Une decision attendue est l'action principale de la vue : le depot d'une
  // dictee laisse alors sa couleur d'accent (un seul accent par ecran).
  const decisionAttendue =
    projet !== null && etapeEnAttente(projet, normaliserEtat(projet.etat).code) !== null;

  return (
    <>
      <header className="entete">
        <div className="entete-contenu">
          <h1>Studio vidéo</h1>
          <p className="accroche">De la dictée à la vidéo éducative</p>
        </div>
      </header>
      <main className="page">
        <div className="colonne-laterale">
          <FormulaireUpload onProjetCree={ouvrirProjetCree} principale={!decisionAttendue} />
          <ListeProjets projets={projets} selection={selection} onSelection={setSelection} />
        </div>
        <div className="colonne-principale">
          {projet ? (
            <DetailProjet projet={projet} onProjet={setProjet} onChangement={rafraichirListe} />
          ) : (
            <p className="vide vide-page">
              {selection ? 'Chargement du projet…' : 'Sélectionnez un projet ou envoyez une dictée.'}
            </p>
          )}
        </div>
      </main>
    </>
  );
}
