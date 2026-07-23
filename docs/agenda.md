# Agenda — Planification du projet

Hypothèse : 1 développeur à temps plein (ajuster les durées sinon).
Découpage : **8 phases / 12 semaines**, jalons à la fin.

## Vue synthétique

| Phase | Semaines | Objectif | Livrable |
|---|---|---|---|
| 0. Fondations | S1 | Socle technique | Workspace compilable, CI verte |
| 1. Ingestion & STT | S2 | Audio → texte | Endpoint upload + transcription |
| 2. Scénariste & orchestrateur | S3–S4 | Texte → scénario validé | Pipeline à états + 1er agent |
| 3. Visuel | S5–S6 | Scènes → images licenciées | Outil `choisir_image` |
| 4. Conteur | S7 | Répliques → voix + sous-titres | Outil `generer_voix`, `.srt` |
| 5. Monteur | S8–S9 | Assets → vidéo finale | Outil `ffmpeg`, rendu complet |
| 6. Publication YouTube | S10 | Vidéo → YouTube | Outil `publier_youtube` |
| 7. Affinage & UX | S11 | Boucle de validation complète | Endpoint `/affiner`, suivi temps réel |
| 8. Durcissement | S12 | Qualité production | Tests, observabilité, déploiement |

## Détail des phases

### Phase 0 — Fondations (S1)
- Workspace Cargo (`core`, `llm`, `agents`, `tools`, `pipeline`, `apps/*`),
  CI (fmt, clippy, tests), gestion de config + secrets.
- Façade `llm` : provider Mistral via `rig-core`, hello-world tool calling.
- **Critère de sortie** : un agent rig minimal appelle un outil factice en CI.

### Phase 1 — Ingestion & STT (S2)
- `POST /audio` (multipart, formats courants, durée max), stockage dans
  `data/<projet>/`.
- Outil `transcrire_audio` (Voxtral Transcribe 2), persistance de la
  transcription + timestamps.
- **Critère de sortie** : un audio dicté depuis un téléphone produit une
  transcription consultable via `GET /projet/:id`.

### Phase 2 — Scénariste & orchestrateur (S3–S4)
- Machine à états du pipeline (SQLite), transitions `auto`/`validation`.
- Agent Scénariste avec structured output `Scenario` (JSON schema strict).
- Agent Réalisateur v1 : enchaîne transcription → scénario, expose
  `POST /valider`.
- **Critère de sortie** : scénario complet généré, validé ou rejeté via API. **→ Jalon M1**

### Phase 3 — Visuel (S5–S6)
- Clients Openverse + Wikimedia Commons (recherche, métadonnées de licence).
- Outil `choisir_image` : requêtes par scène, téléchargement, scoring de
  pertinence, registre des attributions.
- Mode validation : galerie des choix + remplacement par prompt.
- **Critère de sortie** : chaque scène a une image licenciée + attribution. **→ Jalon M2**

### Phase 4 — Conteur (S7)
- Outil `generer_voix` (Voxtral TTS), multi-langue, cache par hash.
- Génération des sous-titres à partir du scénario + durées audio réelles.
- **Critère de sortie** : un audio par scène/langue + `.srt` synchronisés.

### Phase 5 — Monteur (S8–S9)
- Templates ffmpeg (concat, ken-burns, fondus, `loudnorm`, `drawtext`/ASS),
  rendu via whitelist — pas de commande libre.
- Outil `ffmpeg` branché sur l'agent Monteur ; génération d'une preview
  basse résolution pour validation.
- **Critère de sortie** : vidéo 1080p complète avec voix et sous-titres. **→ Jalon M3 (MVP end-to-end hors publication)**

### Phase 6 — Publication YouTube (S10)
- OAuth2 (`yup-oauth2`, installed flow + refresh token), upload reprenable
  (`google-youtube3`).
- Métadonnées (titre, description avec attributions, tags, langue),
  visibilité `private` par défaut, contrôle de quota.
- **Critère de sortie** : vidéo publiée en privé sur une chaîne de test. **→ Jalon M4 (pipeline complet)**

### Phase 7 — Affinage & UX (S11)
- `POST /affiner { etape, prompt }` : régénération ciblée d'une étape et
  propagation en aval.
- Suivi temps réel (SSE/WebSocket) pour le client téléphone/PC.
- **Critère de sortie** : on peut corriger le scénario puis relancer uniquement
  les étapes impactées.

### Phase 8 — Durcissement (S12)
- **Annulation à n'importe quelle étape** : pipeline en tâche de fond
  (token d'annulation par projet), `POST /annuler` (interruption propre
  entre deux scènes/rendus/chunks, ffmpeg tué via `kill_on_drop`) et
  `POST /reprendre` (reprise au dernier livrable stable) — fait.
- Tests d'intégration avec LLM mocké (cassettes), tests des tools sur
  fixtures (audio/images).
- Observabilité (tracing, métriques de coûts API), rotation des journaux.
- Packaging (systemd, conteneur), documentation d'exploitation, option
  provider Ollama/GPU vérifiée.
- **Critère de sortie** : déploiement reproductible, pipeline exécuté sans
  intervention sur une série de 5 vidéos. **→ Jalon M5 (v1.0)**

## Risques et parades

| Risque | Impact | Parade |
|---|---|---|
| Évolutions de l'API `rig` | Refactos aux montées de version | Façade `llm` isolante ; versions épinglées |
| Quota YouTube (10 000 unités/j, upload = 1 600) | ~6 vidéos/jour max | File d'attente de publication, demande d'extension de quota |
| Images « libres » mal licenciées | Strike / retrait | Filtre strict CC0/CC-BY/DP + attribution systématique + registre |
| Commandes ffmpeg générées dangereuses | Sécurité / corruption | Whitelist de templates, chemins confinés à `data/` |
| Dérive des coûts API (STT+LLM+TTS) | Budget | Caches par hash, modèles Medium par défaut, métriques par vidéo |
| Qualité TTS insuffisante dans une langue | Refonte audio | Étape `VoixPretes` en mode validation tant que non stabilisé |

## Post-MVP (backlog)

- Génération d'images locale (SDXL/Flux sur GPU) en alternative au scraping.
- Shorts/verticals (découpe 9:16 via templates ffmpeg dédiés).
- Miniatures YouTube générées.
- Interface web légère de validation (au-delà de l'API).
- Multi-voix / dialogues à plusieurs personnages.
