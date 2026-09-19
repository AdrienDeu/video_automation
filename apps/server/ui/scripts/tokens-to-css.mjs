// Traduit `src/vendor/tokens.json` (copie conforme du design system) en
// proprietes personnalisees CSS, dans `src/vendor/tokens.css`.
//
// Le design system genere ce `tokens.css` de son cote mais ne le publie pas :
// on le derive donc ici, au build, pour qu'il ne puisse jamais diverger des
// tokens vendorises. Ne pas editer `tokens.css` a la main.

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const racine = join(dirname(fileURLToPath(import.meta.url)), '..');
const tokens = JSON.parse(readFileSync(join(racine, 'src/vendor/tokens.json'), 'utf8'));

/** Valeur d'un token de couleur pour un theme, avec repli sur le premier. */
function valeurCouleur(token, theme, themeParDefaut) {
  if (typeof token.value === 'string') return token.value;
  return token.value[theme] ?? token.value[themeParDefaut];
}

/** Resout les alias `{autre-token}` d'une table nom -> valeur. */
function resoudreAlias(table) {
  const resolu = {};
  for (const [nom, valeur] of Object.entries(table)) {
    let courant = valeur;
    const vus = new Set([nom]);
    while (typeof courant === 'string' && /^\{[^{}]+\}$/.test(courant)) {
      const cible = courant.slice(1, -1);
      if (vus.has(cible) || !(cible in table)) break;
      vus.add(cible);
      courant = table[cible];
    }
    resolu[nom] = courant;
  }
  return resolu;
}

const lignes = [];
const themes = tokens.color?.themes ?? [{ id: 'light' }];
const themeParDefaut = themes[0].id;

/** Bloc de declarations d'un theme de couleurs. */
function couleursDuTheme(theme) {
  const table = {};
  for (const token of tokens.color?.tokens ?? []) {
    table[token.name] = valeurCouleur(token, theme, themeParDefaut);
  }
  const resolu = resoudreAlias(table);
  return Object.entries(resolu)
    .filter(([, valeur]) => typeof valeur === 'string')
    .map(([nom, valeur]) => `  --${nom}: ${valeur};`);
}

lignes.push('/* Genere par scripts/tokens-to-css.mjs depuis src/vendor/tokens.json.');
lignes.push(`   Design system « ${tokens.name} » version ${tokens.version}. Ne pas editer. */`);
lignes.push('');
lignes.push(':root {');
lignes.push(...couleursDuTheme(themeParDefaut));

// Familles typographiques.
for (const [nom, pile] of Object.entries(tokens.type?.families ?? {})) {
  lignes.push(`  --font-${nom}: ${pile};`);
}
// Chaque style typographique en taille / interligne / graisse / interlettrage.
for (const groupe of tokens.type?.groups ?? []) {
  for (const style of groupe.styles ?? []) {
    lignes.push(`  --text-${style.name}-size: ${style.fontSize};`);
    lignes.push(`  --text-${style.name}-line: ${style.lineHeight};`);
    lignes.push(`  --text-${style.name}-weight: ${style.fontWeight};`);
    if (style.letterSpacing) {
      lignes.push(`  --text-${style.name}-spacing: ${style.letterSpacing};`);
    }
  }
}
// Les autres familles (espacement, rayons, ombres) partagent la meme forme.
for (const [famille, contenu] of Object.entries(tokens)) {
  if (['name', 'version', 'color', 'type', 'meta'].includes(famille)) continue;
  for (const token of contenu.tokens ?? []) {
    lignes.push(`  --${token.name}: ${token.value};`);
  }
}
lignes.push('}');

// Thèmes suivants : preference systeme, et forçage explicite par data-theme.
for (const theme of themes.slice(1)) {
  const declarations = couleursDuTheme(theme.id);
  lignes.push('');
  lignes.push('@media (prefers-color-scheme: dark) {');
  lignes.push(`  :root:not([data-theme="${themeParDefaut}"]) {`);
  lignes.push(...declarations.map((l) => `  ${l}`));
  lignes.push('  }');
  lignes.push('}');
  lignes.push('');
  lignes.push(`:root[data-theme="${theme.id}"] {`);
  lignes.push(...declarations);
  lignes.push('}');
}

writeFileSync(join(racine, 'src/vendor/tokens.css'), `${lignes.join('\n')}\n`);
console.log(`tokens.css genere (${themes.length} theme(s), ${tokens.color?.tokens?.length ?? 0} couleurs)`);
