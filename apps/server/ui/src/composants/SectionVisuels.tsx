// Images licenciees choisies par le Visuel : apercu, attribution (exigee par
// les licences CC) et remplacement par une nouvelle recherche.

import { useState } from 'react';

import { FormulaireAffinage } from './FormulaireAffinage';
import { Section } from './Section';
import { urlFichier } from '../api';
import { Button } from '../design-system';
import type { Asset, EtapeValidation } from '../types';

interface Props {
  id: string;
  visuels: Asset[];
  enCours: boolean;
  onAffiner: (etape: EtapeValidation, consigne: string) => void;
  onRemplacer: (scene: number, requete: string) => void;
}

function Remplacement({
  scene,
  enCours,
  onRemplacer,
}: {
  scene: number;
  enCours: boolean;
  onRemplacer: (scene: number, requete: string) => void;
}) {
  const [requete, setRequete] = useState('');

  return (
    <form
      className="affinage affinage-compact"
      onSubmit={(evenement) => {
        evenement.preventDefault();
        const propre = requete.trim();
        if (!propre) return;
        onRemplacer(scene, propre);
        setRequete('');
      }}
    >
      <input
        className="va-input"
        type="text"
        required
        value={requete}
        placeholder="Remplacer par…"
        aria-label={`Nouvelle recherche d'image pour la scène ${scene + 1}`}
        onChange={(e) => setRequete(e.target.value)}
      />
      <Button type="submit" variant="secondary" size="sm" disabled={enCours}>
        Remplacer
      </Button>
    </form>
  );
}

export function SectionVisuels({ id, visuels, enCours, onAffiner, onRemplacer }: Props) {
  if (visuels.length === 0) return null;

  return (
    <Section titre="Visuels">
      <div className="galerie">
        {visuels.map((visuel) => (
          <figure className="visuel" key={`${visuel.scene}-${visuel.fichier}`}>
            <img
              src={urlFichier(id, visuel.fichier)}
              alt={visuel.titre ?? `Scène ${visuel.scene + 1}`}
              loading="lazy"
            />
            <figcaption>
              <span
                className="visuel-titre"
                title={`Scène ${visuel.scene + 1} — ${visuel.titre ?? 'Sans titre'}`}
              >
                Scène {visuel.scene + 1} — {visuel.titre ?? 'Sans titre'}
              </span>
              <span className="visuel-attribution">
                {visuel.auteur ?? 'auteur inconnu'}, {visuel.licence} —{' '}
                <a href={visuel.url_page} target="_blank" rel="noopener noreferrer">
                  page de l'œuvre
                </a>
              </span>
              <Remplacement scene={visuel.scene} enCours={enCours} onRemplacer={onRemplacer} />
            </figcaption>
          </figure>
        ))}
      </div>
      <FormulaireAffinage etape="visuels" enCours={enCours} onAffiner={onAffiner} />
    </Section>
  );
}
