// `POST /audio` : depot d'une dictee, qui cree le projet et lance le pipeline.

import { useRef, useState } from 'react';

import { envoyerAudio } from '../api';
import { Alert, Button, Card, Input } from '../design-system';
import type { Projet } from '../types';

interface Props {
  onProjetCree: (projet: Projet) => void;
  /**
   * `accent` ne vaut que pour l'action principale de la vue, au plus une fois
   * (design system, usage du token `accent`) : le depot cede la vedette quand
   * une decision est attendue sur le projet ouvert.
   */
  principale: boolean;
}

export function FormulaireUpload({ onProjetCree, principale }: Props) {
  const champFichier = useRef<HTMLInputElement>(null);
  const [langue, setLangue] = useState('');
  const [envoi, setEnvoi] = useState(false);
  const [erreur, setErreur] = useState<string | null>(null);

  async function soumettre(evenement: React.FormEvent) {
    evenement.preventDefault();
    const fichier = champFichier.current?.files?.[0];
    if (!fichier) return;

    setEnvoi(true);
    setErreur(null);
    try {
      const projet = await envoyerAudio(fichier, langue.trim());
      if (champFichier.current) champFichier.current.value = '';
      setLangue('');
      onProjetCree(projet);
    } catch (e) {
      setErreur(e instanceof Error ? e.message : String(e));
    } finally {
      setEnvoi(false);
    }
  }

  return (
    <Card>
      <h2 className="carte-titre">Nouveau projet</h2>
      <form className="formulaire" onSubmit={soumettre}>
        <div className="va-field">
          <label className="va-input-label" htmlFor="champ-audio">
            Fichier audio
          </label>
          <input
            className="va-input va-input-fichier"
            id="champ-audio"
            ref={champFichier}
            type="file"
            accept="audio/*"
            required
          />
          <p className="va-input-helper">mp3, wav, m4a, flac, ogg ou webm — 30 min au maximum.</p>
        </div>
        <Input
          label="Langue"
          helperText="Optionnel : code ISO, par exemple fr. Détectée automatiquement si vide."
          placeholder="fr"
          maxLength={12}
          value={langue}
          onChange={(e) => setLangue(e.target.value)}
        />
        <div className="formulaire-actions">
          <Button type="submit" variant={principale ? 'primary' : 'secondary'} disabled={envoi}>
            {envoi ? 'Envoi en cours…' : 'Envoyer la dictée'}
          </Button>
        </div>
      </form>
      {erreur && (
        <Alert tone="danger" title="L'envoi a échoué">
          {erreur} — vérifiez le format et la taille du fichier, puis réessayez.
        </Alert>
      )}
    </Card>
  );
}
