// Livrables audio et video : voix off et sous-titres du Conteur, puis preview
// et video finale du Monteur, enfin le lien de publication YouTube.

import { FormulaireAffinage } from './FormulaireAffinage';
import { Section } from './Section';
import { urlFichier } from '../api';
import type { EtapeValidation, Projet } from '../types';

interface Props {
  projet: Projet;
  enCours: boolean;
  onAffiner: (etape: EtapeValidation, consigne: string) => void;
}

export function SectionVoix({ projet, enCours, onAffiner }: Props) {
  if (projet.voix.length === 0 && projet.sous_titres.length === 0) return null;

  return (
    <Section titre="Voix et sous-titres">
      {projet.voix.map((voix) => (
        <div className="voix" key={`${voix.langue}-${voix.scene}`}>
          <span className="voix-meta">
            Scène {voix.scene + 1}
            <span className="mono">
              {' '}
              {voix.langue} · {voix.duree.toFixed(1)} s
            </span>
          </span>
          <audio controls src={urlFichier(projet.id, voix.fichier)} />
        </div>
      ))}
      {projet.sous_titres.map((srt) => (
        <p className="telechargement" key={srt}>
          <a href={urlFichier(projet.id, srt)} download={srt}>
            Télécharger {srt}
          </a>
        </p>
      ))}
      <FormulaireAffinage etape="voix" enCours={enCours} onAffiner={onAffiner} />
    </Section>
  );
}

export function SectionMontage({ projet, enCours, onAffiner }: Props) {
  const apercu = projet.preview ?? projet.video;
  if (!apercu) return null;

  return (
    <Section titre="Montage">
      <video className="lecteur" controls src={urlFichier(projet.id, apercu)} />
      {projet.video && (
        <p className="telechargement">
          <a href={urlFichier(projet.id, projet.video)} download={projet.video}>
            Télécharger la vidéo finale
          </a>
        </p>
      )}
      <FormulaireAffinage etape="montage" enCours={enCours} onAffiner={onAffiner} />
    </Section>
  );
}

export function SectionPublication({ projet }: { projet: Projet }) {
  if (!projet.youtube) return null;

  return (
    <Section titre="Publication">
      <p className="telechargement">
        <a href={projet.youtube.url} target="_blank" rel="noopener noreferrer">
          Voir la vidéo sur YouTube
        </a>
      </p>
    </Section>
  );
}
