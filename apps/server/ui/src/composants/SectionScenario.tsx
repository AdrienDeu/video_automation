// Scenario produit par le Scenariste, scene par scene.

import { FormulaireAffinage } from './FormulaireAffinage';
import { Section } from './Section';
import type { EtapeValidation, Scenario } from '../types';

interface Props {
  scenario: Scenario | null;
  enCours: boolean;
  onAffiner: (etape: EtapeValidation, consigne: string) => void;
}

export function SectionScenario({ scenario, enCours, onAffiner }: Props) {
  if (!scenario) return null;

  return (
    <Section titre="Scénario">
      <h4 className="scenario-titre">{scenario.titre}</h4>
      <p className="meta">Public : {scenario.public}</p>
      <p className="meta">Style visuel : {scenario.style_images}</p>
      <ol className="scenes">
        {scenario.scenes.map((scene, index) => (
          <li className="scene" key={index}>
            <h5 className="scene-titre">
              Scène {index + 1}
              <span className="mono"> {scene.duree_cible} s</span>
            </h5>
            <p className="scene-narration">{scene.narration}</p>
            {scene.dialogues.map((dialogue, rang) => (
              <p className="scene-dialogue" key={rang}>
                <strong>{dialogue.personnage} : </strong>
                {dialogue.replique}
              </p>
            ))}
            <p className="meta">À l'écran : {scene.description_visuelle}</p>
          </li>
        ))}
      </ol>
      <FormulaireAffinage etape="scenario" enCours={enCours} onAffiner={onAffiner} />
    </Section>
  );
}
