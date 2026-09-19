// Point d'entree de l'interface : monte l'application React dans la page
// servie par le binaire serveur (`apps/server/src/ui.rs`).

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import './style.css';

const racine = document.getElementById('racine');
if (!racine) throw new Error("l'element #racine est absent de la page");

createRoot(racine).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
