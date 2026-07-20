# Architecture

## 1. Exigences

- Serveur Linux écrit en Rust, recevant un fichier audio dicté depuis un
  téléphone ou un ordinateur.
- Chaîne d'agents LLM spécialisés : Réalisateur (orchestrateur), Scénariste,
  Visuel, Conteur, Monteur.
- Système de fonctions-outils (*tool calling*), notamment les outils
  « Choisir une image » et « ffmpeg ».
- Étapes automatiques **ou** bloquées en attente de validation utilisateur,
  avec affinage possible via un prompt par étape.
- LLM interchangeable : API hébergée (Mistral) ou LLM local sur serveur GPU.

## 2. État de l'art : les trois options Rust

| Critère | `rig` (rig-core) | `zeroclaw` | Sans lib (reqwest + serde) |
|---|---|---|---|
| Nature | **Bibliothèque** à intégrer dans notre serveur | **Runtime autonome** (binaire unique, assistant personnel multi-canaux) | Appels HTTP écrits à la main |
| Tool calling | Oui, **type-safe** via serde : les arguments JSON du modèle sont désérialisés dans des types Rust, les erreurs sont détectées avant l'appel de l'outil | Oui, mais orienté outils système du runtime (shell, fichiers, canaux) | À réimplémenter (boucle outil, parsing, retries) |
| Multi-providers | 20+ providers dont **Mistral** et **Ollama** → bascule API ↔ GPU local sans changer les agents | Providers interchangeables aussi, mais pilotés par sa configuration propre | Un seul provider sauf si l'on réécrit une couche d'abstraction |
| Maturité | v0.39 (juin 2026), ~8k étoiles, MIT, API stabilisée fin 2025/début 2026, utilisé en production ([dev.co](https://dev.co/ai/frameworks/rig), [rustify.rs](https://rustify.rs/articles/building-ai-apps-with-rust-rig-2026)) | Jeune (bêta), très léger (~4 Mo RAM, démarrage <10 ms) mais conçu comme un produit à déployer, pas comme une brique logicielle ([github.com/zeroclaw-labs/zeroclaw](https://github.com/zeroclaw-labs/zeroclaw)) | Aucune dette d'API externe, contrôle total |
| Adéquation au besoin | Agents + outils + streaming + structured output : exactement notre cas | Doublon avec notre orchestrateur maison ; on hériterait de ses canaux (Telegram…) dont on n'a pas besoin | Pertinent seulement si l'on reste à **un seul** appel LLM sans agents |

## 3. Décision : `rig-core`

**Choix retenu : [`rig`](https://github.com/0xPlaygrounds/rig).**

Raisons succinctes :

1. **C'est une bibliothèque, pas un runtime.** zeroclaw est un binaire
   autonome à configurer ; notre besoin est un pipeline sur mesure intégré à
   notre serveur HTTP, pas un assistant généraliste.
2. **Tool calling type-safe.** Les outils « choisir_image » et « ffmpeg »
   sont déclarés avec des types d'arguments serde ; rig valide le JSON du
   modèle avant l'exécution et gère la boucle agent/outil — c'est le cœur du
   projet, inutile de le réécrire.
3. **Provider interchangeable.** rig supporte Mistral et Ollama : l'option
   « serveur GPU avec LLM local » reste ouverte sans toucher au code des
   agents, conformément au cahier des charges.
4. **Maturité suffisante** : API stabilisée, MIT, adopteurs en production.

Compromis accepté : une dépendance plus lourde qu'un client HTTP nu, et un
suivi des montées de version de rig à prévoir (API encore évolutive).

> Option de repli documentée : si rig devenait bloquant, l'isolation du client
> LLM dans le crate `llm` (voir §5) permet de retomber sur du reqwest + serde
> contre l'API Mistral sans toucher aux agents.

## 4. Stack complète

| Fonction | Choix principal | Alternative locale / GPU | Notes |
|---|---|---|---|
| STT (voix → texte) | Mistral **Voxtral Transcribe 2** (~0,003 $/min, 13 langues, timestamps mot à mot — [gend.co](https://www.gend.co/blog/voxtral-transcribe-2), [Simon Willison](https://simonwillison.net/2026/Feb/4/voxtral-2/)) | whisper.cpp (Whisper large-v3) sur GPU | Voxtral couvre les langues cibles courantes ; Whisper si besoin de 99+ langues |
| LLM agents | Mistral (Large/Medium) via rig | Ollama / vLLM sur GPU via rig | Même code d'agents dans les deux cas |
| TTS (voix off) | Mistral **Voxtral TTS** (`voxtral-mini-tts`, mars 2026 — [qwe.edu.pl](https://www.qwe.edu.pl/tutorial/voxtral-tts-open-weight-voice-ai/)) | Piper (local, léger) | Une seule clé API Mistral couvre STT + LLM + TTS |
| Images libres de droits | API **Openverse** (agrégateur CC) + **Wikimedia Commons** ; Pexels/Unsplash en complément | Génération locale SDXL/Flux sur GPU | Licence et attribution stockées par asset (§9) |
| Montage | **ffmpeg en CLI** via `std::process::Command` | — | Pas de binding `ffmpeg-next` : la CLI est plus stable, scriptable et déboguable |
| Publication | Crates [`google-youtube3`](https://docs.rs/google-youtube3) + `yup-oauth2` (upload reprenable) | — | Quota par défaut 10 000 unités/jour, un upload coûte 1 600 unités (~6 vidéos/jour) |
| Serveur HTTP | `axum` + `tokio` | — | Upload audio, WebSocket ou SSE pour le suivi et les validations |
| Persistance | `sqlx` + SQLite | JSON sur disque pour un MVP | État du pipeline, projets, artefacts |

## 5. Arborescence du workspace Cargo

```
video_automation/
├── Cargo.toml                  # [workspace]
├── docs/                       # ce dossier
├── crates/
│   ├── core/                   # Types partagés : Projet, Scene, Scenario, Asset,
│   │                           #   erreurs (thiserror), config (figment/toml), états du pipeline
│   ├── llm/                    # Façade rig : construction des providers (Mistral/Ollama),
│   │                           #   prompts système versionnés, structured output (JSON schema)
│   ├── agents/                 # Un module par agent (voir §6)
│   │   └── src/{realisateur,scenariste,visuel,conteur,monteur}.rs
│   ├── tools/                  # Outils LLM exposés aux agents (voir §7)
│   │   └── src/{choisir_image,ffmpeg,youtube,stt,tts}.rs
│   └── pipeline/               # Machine à états, file de tâches, validations humaines,
│                               #   régénération ciblée après prompt d'affinage
├── apps/
│   ├── server/                 # axum : POST /audio, GET /projet/:id, POST /valider, POST /affiner
│   └── cli/                    # Pilotage terminal (debug, rejouer une étape)
├── assets/                     # Templates ffmpeg, polices, charte graphique, prompts
└── data/                       # Un sous-dossier par vidéo : audio, images, voix, srt, rendu
```

Principes :

- **`core` ne dépend de rien**, tout le monde dépend de `core`.
- Le LLM n'est visible que derrière la façade `llm` → provider interchangeable.
- Les agents ne font que du raisonnement ; les effets de bord (fichiers,
  ffmpeg, réseau) vivent dans `tools`, testables indépendamment du LLM.

## 6. Les cinq agents

| Agent | Entrée | Sortie (structured output JSON) | Modèle suggéré |
|---|---|---|---|
| **Réalisateur** (orchestrateur) | Transcription + demande utilisateur | Plan d'exécution, découpage en tâches, dialogue de validation | Mistral Large (raisonnement) |
| **Scénariste** | Brief du Réalisateur | `Scenario { titre, public, style_images, scenes[] { narration, dialogues, description_visuelle, duree_cible } }` | Mistral Large |
| **Visuel** | `description_visuelle` + `style_images` par scène | Un `Asset` par scène (fichier, source, licence, attribution) via l'outil `choisir_image` | Mistral Medium |
| **Conteur** | Répliques + langues cibles | Un fichier audio par scène et par langue + durées réelles, via l'outil `tts` | — (TTS, pas un LLM ; l'agent orchestre les appels) |
| **Monteur** | Assets + audios + sous-titres | Commande ffmpeg validée via l'outil `ffmpeg`, vidéo finale + `.srt`/`.ass` | Mistral Medium |

Le Réalisateur est le seul agent « conversationnel » ; les quatre autres sont
des étapes spécialisées pilotées par la machine à états, ce qui borne les
coûts et les dérives.

## 7. Les outils LLM (tool calling)

Chaque outil est une fonction Rust déclarée à rig avec des arguments typés
serde. Outils prévus :

| Outil | Arguments | Effet | Garde-fous |
|---|---|---|---|
| `choisir_image` | `requete: String, scene_id: u32, style: String` | Interroge Openverse/Wikimedia, télécharge, note la pertinence, retourne le meilleur `Asset` | Licence vérifiée (CC-BY/CC0…), attribution enregistrée, taille min. exigée |
| `ffmpeg` | `template: String, params: HashMap<String, String>` | Exécute un **template prédéfini** (concat, ken-burns, fondu, drawtext, loudnorm) rendu avec les paramètres | **Jamais de ligne de commande libre** : whitelist de templates dans `assets/ffmpeg/`, chemins confinés à `data/` |
| `transcrire_audio` | `fichier: PathBuf, langue: Option<String>` | Appel Voxtral Transcribe, retourne texte + timestamps | Format/durée max validés |
| `generer_voix` | `texte: String, langue: String, voix: String` | Appel TTS, retourne le fichier audio + sa durée | Longueur max par segment, cache par hash |
| `publier_youtube` | `video: PathBuf, titre, description, tags, visibilite` | Upload reprenable via Data API v3 | `visibilite = private` par défaut ; quota journalier vérifié |
| `demander_validation` | `etape, resume, apercu` | Bloque le pipeline jusqu'à réponse utilisateur | Timeout configurable → poursuite auto optionnelle |

L'outil `ffmpeg` est le point le plus sensible : le LLM **choisit un template
et ses paramètres**, il ne rédige jamais de commande shell arbitraire.

## 8. Machine à états du pipeline

```
AudioRecu → Transcrit → ScenarioGenere → VisuelsPrets → VoixPretes
          → MontagePret → Publie
```

- Chaque transition est marquée `auto` ou `validation` dans la config du
  projet (ex. : validation obligatoire du scénario, publication automatique).
- L'état est persisté (SQLite) → reprise après crash, régénération d'une
  seule étape (`POST /affiner { etape, prompt }`) sans tout relancer.
- Un échec d'étape (API down, asset introuvable) passe l'étape en `Erreur`
  avec le détail ; le Réalisateur propose une correction ou une nouvelle
  tentative.

## 9. Sécurité, licences, coûts

- **Secrets** : variables d'environnement (`MISTRAL_API_KEY`, OAuth Google),
  jamais dans le dépôt.
- **Images** : seules des licences compatibles (CC0, CC-BY, domaine public)
  sont acceptées ; l'attribution est insérée dans la description YouTube.
- **ffmpeg** : cf. §7, pas de commande libre ; les chemins sont vérifiés
  appartenir au dossier du projet.
- **YouTube** : publication en `private` par défaut, passage en `public`
  explicite côté utilisateur.
- **Coûts** : cache des appels STT/TTS par hash, modèles Medium pour les
  agents simples, Large réservé au Scénariste/Réalisateur.
