// Detail d'un projet : avancement, livrables de chaque etape et decisions
// humaines. Toutes les actions passent par ici ; chacune renvoie le projet a
// jour, affiche immediatement sans attendre le flux SSE.

import { useState } from 'react';

import { BlocDecision } from './BlocDecision';
import { Progression } from './Progression';
import { Section } from './Section';
import { SectionMontage, SectionPublication, SectionVoix } from './SectionMedias';
import { SectionScenario } from './SectionScenario';
import { SectionVisuels } from './SectionVisuels';
import * as api from '../api';
import { Alert, Badge, Card } from '../design-system';
import { etapeEnAttente, libelleEtat, normaliserEtat, tonEtat } from '../etats';
import type { DecisionValidation, EtapeValidation, Projet } from '../types';

interface Props {
  projet: Projet;
  onProjet: (projet: Projet) => void;
  onChangement: () => void;
}

/**
 * Une tache tourne-t-elle en arriere-plan ? Le serveur ne l'expose pas : on le
 * deduit de l'etat, qui ne bouge qu'en cas de succes complet d'une etape. Ni
 * terminal, ni en attente d'une decision humaine : le pipeline avance.
 */
function tacheEnCours(projet: Projet, code: string): boolean {
  if (code === 'publie' || code === 'annule' || code === 'erreur') return false;
  return etapeEnAttente(projet, code) === null;
}

export function DetailProjet({ projet, onProjet, onChangement }: Props) {
  const [enVol, setEnVol] = useState(false);
  const [erreur, setErreur] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const etat = normaliserEtat(projet.etat);
  const enAttente = etapeEnAttente(projet, etat.code);
  const enCours = tacheEnCours(projet, etat.code);

  /** Execute une action, affiche le projet renvoye et remonte l'echec. */
  async function agir(action: () => Promise<Projet>, echec: string, succes?: string) {
    setEnVol(true);
    setErreur(null);
    setInfo(null);
    try {
      onProjet(await action());
      if (succes) setInfo(succes);
      onChangement();
    } catch (e) {
      setErreur(`${echec} : ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setEnVol(false);
    }
  }

  const valider = (etape: EtapeValidation, decision: DecisionValidation) =>
    agir(() => api.valider(projet.id, etape, decision), 'La validation a échoué');

  const affiner = (etape: EtapeValidation, consigne: string) =>
    agir(
      () => api.affiner(projet.id, etape, consigne),
      "L'affinage a échoué",
      'Pipeline relancé : les étapes en aval sont régénérées.',
    );

  const remplacer = (scene: number, requete: string) =>
    agir(
      () => api.remplacerVisuel(projet.id, scene, requete),
      'Le remplacement a échoué',
      'Image remplacée : les visuels sont à revalider.',
    );

  const annuler = () =>
    agir(
      () => api.annuler(projet.id),
      "L'annulation a échoué",
      "Annulation demandée : le traitement s'arrête au prochain point de contrôle.",
    );

  const reprendre = () =>
    agir(
      () => api.reprendre(projet.id),
      'La reprise a échoué',
      'Pipeline relancé depuis le dernier point stable.',
    );

  return (
    <div className="detail">
      <Card>
        <div className="detail-entete">
          <h2 className="carte-titre">
            Projet <span className="mono">{projet.id.slice(0, 8)}</span>
          </h2>
          <Badge tone={tonEtat(etat.code)}>{libelleEtat(etat.code)}</Badge>
        </div>
        <Progression code={etat.code} enCours={enCours} />
        {etat.code === 'erreur' && etat.message && (
          <Alert tone="danger" title="L'étape a échoué">
            {etat.message} — corrigez la cause puis relancez l'étape avec « Affiner », ou reprenez
            le projet.
          </Alert>
        )}
        {erreur && <Alert tone="danger" title="Action impossible">{erreur}</Alert>}
        {info && <Alert tone="info">{info}</Alert>}
      </Card>

      <Card>
        <Section titre="Transcription" visible={projet.transcription !== null}>
          {projet.transcription && (
            <>
              {projet.transcription.langue && (
                <p className="meta">Langue : {projet.transcription.langue}</p>
              )}
              <p className="transcription">{projet.transcription.texte}</p>
            </>
          )}
        </Section>
        <SectionScenario scenario={projet.scenario} enCours={enVol} onAffiner={affiner} />
        <SectionVisuels
          id={projet.id}
          visuels={projet.visuels}
          enCours={enVol}
          onAffiner={affiner}
          onRemplacer={remplacer}
        />
        <SectionVoix projet={projet} enCours={enVol} onAffiner={affiner} />
        <SectionMontage projet={projet} enCours={enVol} onAffiner={affiner} />
        <SectionPublication projet={projet} />
      </Card>

      <BlocDecision
        projet={projet}
        code={etat.code}
        enAttente={enAttente}
        enCours={enVol}
        onValider={valider}
        onAnnuler={annuler}
        onReprendre={reprendre}
      />
    </div>
  );
}
