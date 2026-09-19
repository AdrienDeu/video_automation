// `POST /affiner` : regenere une etape avec une consigne, et invalide l'aval.

import { useState } from 'react';

import { Button } from '../design-system';
import { LIBELLES_ETAPES } from '../etats';
import type { EtapeValidation } from '../types';

interface Props {
  etape: EtapeValidation;
  enCours: boolean;
  onAffiner: (etape: EtapeValidation, consigne: string) => void;
}

export function FormulaireAffinage({ etape, enCours, onAffiner }: Props) {
  const [consigne, setConsigne] = useState('');

  function soumettre(evenement: React.FormEvent) {
    evenement.preventDefault();
    const propre = consigne.trim();
    if (!propre) return;
    onAffiner(etape, propre);
    setConsigne('');
  }

  return (
    <form className="affinage" onSubmit={soumettre}>
      <input
        className="va-input"
        type="text"
        required
        value={consigne}
        placeholder={`Affiner ${LIBELLES_ETAPES[etape]}…`}
        aria-label={`Consigne pour affiner ${LIBELLES_ETAPES[etape]}`}
        onChange={(e) => setConsigne(e.target.value)}
      />
      <Button type="submit" variant="secondary" size="sm" disabled={enCours}>
        Affiner
      </Button>
    </form>
  );
}
