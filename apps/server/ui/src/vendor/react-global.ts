// Le bundle du design system est un script classique : il lit `window.React`
// au moment ou il s'execute, puis publie `window.VideoAutomation`. Ce module
// installe le global AVANT lui — l'ordre d'evaluation des imports ES (en
// profondeur, dans l'ordre d'apparition) le garantit, cf. `design-system.ts`.

import * as React from 'react';

declare global {
  interface Window {
    React: typeof React;
  }
}

window.React = React;
