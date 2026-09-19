// Pont vers le design system « Video Automation ».
//
// `vendor/ds-bundle.js`, `vendor/ds-bundle.css` et `vendor/tokens.json` sont
// des copies conformes du systeme publie : on ne les edite jamais ici, on les
// re-synchronise. Le bundle etant un script classique sans export, ses
// composants sont recuperes sur `window.VideoAutomation` (type par
// `vendor/ds-bundle.d.ts`, lui aussi venu du systeme).

import './vendor/react-global';
import './vendor/tokens.css';
import './vendor/ds-bundle.css';
import './vendor/ds-bundle.js';

const systeme = window.VideoAutomation;

if (!systeme) {
  throw new Error(
    "le bundle du design system n'a pas publie window.VideoAutomation",
  );
}

export const { Button, Input, Badge, Card, Alert, ProgressBar } = systeme;
