// Decisions humaines sur un projet : validation de l'etape en attente
// (`POST /valider`), puis annulation (`POST /annuler`) ou reprise
// (`POST /reprendre`).

import { Alert, Button, Card } from '../design-system';
import { LIBELLES_ETAPES } from '../etats';
import type { DecisionValidation, EtapeValidation, Projet } from '../types';

interface Props {
  projet: Projet;
  code: string;
  enAttente: EtapeValidation | null;
  enCours: boolean;
  onValider: (etape: EtapeValidation, decision: DecisionValidation) => void;
  onAnnuler: () => void;
  onReprendre: () => void;
}

export function BlocDecision({
  projet,
  code,
  enAttente,
  enCours,
  onValider,
  onAnnuler,
  onReprendre,
}: Props) {
  const terminal = code === 'publie';
  if (!enAttente && terminal) return null;

  return (
    <Card className="decision">
      {enAttente && (
        <>
          <h3 className="section-titre">Validation</h3>
          <p>Voulez-vous valider {LIBELLES_ETAPES[enAttente]} ?</p>
          <div className="actions">
            <Button
              variant="primary"
              disabled={enCours}
              onClick={() => onValider(enAttente, 'accepte')}
            >
              Accepter
            </Button>
            <Button
              variant="secondary"
              disabled={enCours}
              onClick={() => onValider(enAttente, 'rejete')}
            >
              Rejeter
            </Button>
          </div>
        </>
      )}
      {/* L'annulation n'est pas `danger` : elle est reversible (« Reprendre »),
          et le design system reserve ce ton aux actions irreversibles. */}
      {!terminal && (
        <div className="actions actions-secondaires">
          {code === 'annule' ? (
            <Button variant="secondary" disabled={enCours} onClick={onReprendre}>
              Reprendre
            </Button>
          ) : (
            <Button variant="secondary" disabled={enCours} onClick={onAnnuler}>
              Annuler le traitement
            </Button>
          )}
        </div>
      )}
      {code === 'annule' && (
        <Alert tone="warning" title="Traitement interrompu">
          La reprise repart du dernier livrable produit : les étapes déjà abouties ne sont pas
          rejouées. Projet {projet.id.slice(0, 8)}.
        </Alert>
      )}
    </Card>
  );
}
