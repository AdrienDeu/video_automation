// Avancement du projet : barre de progression du design system et suite des
// etapes franchies. En erreur, aucune etape n'est marquee active — on ne sait
// pas laquelle a echoue.

import { ProgressBar } from '../design-system';
import { ETAPES_PIPELINE, progression } from '../etats';

interface Props {
  code: string;
  /** Une tache tourne : la duree restante est inconnue. */
  enCours: boolean;
}

export function Progression({ code, enCours }: Props) {
  const indexActif = ETAPES_PIPELINE.findIndex(([etat]) => etat === code);
  const enErreur = code === 'erreur';

  return (
    <div className="progression">
      <ProgressBar
        label="Avancement du pipeline"
        value={progression(code)}
        indeterminate={enCours}
      />
      <ol className="etapes">
        {ETAPES_PIPELINE.map(([etat, libelle], index) => {
          let classe = 'etape';
          if (!enErreur && indexActif >= 0) {
            if (index < indexActif) classe += ' etape-faite';
            else if (index === indexActif) classe += ' etape-active';
          }
          return (
            <li key={etat} className={classe}>
              <span className="etape-point" aria-hidden="true" />
              <span className="etape-libelle">{libelle}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
