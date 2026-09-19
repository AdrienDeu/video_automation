// Liste des projets connus, du plus recent au plus ancien.

import { Badge, Card } from '../design-system';
import { libelleEtat, tonEtat } from '../etats';
import type { ProjetResume } from '../types';

interface Props {
  projets: ProjetResume[];
  selection: string | null;
  onSelection: (id: string) => void;
}

export function ListeProjets({ projets, selection, onSelection }: Props) {
  return (
    <Card>
      <h2 className="carte-titre">Projets</h2>
      {projets.length === 0 ? (
        <p className="vide">Aucun projet pour le moment.</p>
      ) : (
        <ul className="liste-projets">
          {projets.map((resume) => (
            <li key={resume.id}>
              <button
                type="button"
                className={`ligne-projet${resume.id === selection ? ' ligne-projet-active' : ''}`}
                aria-current={resume.id === selection}
                onClick={() => onSelection(resume.id)}
              >
                <span className="ligne-projet-id">{resume.id.slice(0, 8)}</span>
                <Badge tone={tonEtat(resume.etat)}>{libelleEtat(resume.etat)}</Badge>
                {/* `maj` est un `datetime('now')` SQLite, en UTC : la minute
                    suffit a l'ecran, la seconde reste dans l'infobulle. */}
                <span className="ligne-projet-date" title={`${resume.maj} UTC`}>
                  {resume.maj.slice(0, 16)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
