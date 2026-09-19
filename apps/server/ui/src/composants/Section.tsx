// Titre de section du detail, avec son contenu. Rien n'est rendu quand la
// section n'a pas (encore) de livrable a montrer.

import type { ReactNode } from 'react';

interface Props {
  titre: string;
  children: ReactNode;
  /** Rendue seulement si vrai : evite un titre isole sur une etape a venir. */
  visible?: boolean;
}

export function Section({ titre, children, visible = true }: Props) {
  if (!visible) return null;
  return (
    <section className="section">
      <h3 className="section-titre">{titre}</h3>
      {children}
    </section>
  );
}
