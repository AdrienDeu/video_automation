"use strict";

// Interface web du pipeline video : une page, trois zones (upload, liste,
// detail). Tout contenu venant du serveur est injecte via textContent ou des
// attributs sur des elements crees, jamais via innerHTML.

// --- Etat local -------------------------------------------------------------

let projetCourant = null; // id du projet affiche dans la zone detail
let minuteurDetail = null;

// Etats dans lesquels le pipeline peut progresser sans action humaine :
// seuls ceux-la justifient un rafraichissement automatique du detail.
const ETATS_EN_COURS = new Set(["audio_recu", "transcrit"]);

const LIBELLES_ETATS = {
  audio_recu: "Audio recu",
  transcrit: "Transcrit",
  scenario_genere: "Scenario genere",
  visuels_prets: "Visuels prets",
  voix_pretes: "Voix pretes",
  montage_pret: "Montage pret",
  publie: "Publie",
  erreur: "Erreur",
};

// Etapes du pipeline dans l'ordre, pour le stepper du detail.
const ETAPES_PIPELINE = [
  ["audio_recu", "Audio"],
  ["transcrit", "Transcription"],
  ["scenario_genere", "Scenario"],
  ["visuels_prets", "Visuels"],
  ["voix_pretes", "Voix"],
  ["montage_pret", "Montage"],
  ["publie", "Publie"],
];

// --- Petits utilitaires -----------------------------------------------------

// Cree un element avec classe et texte optionnels (texte via textContent).
function el(balise, classe, texte) {
  const noeud = document.createElement(balise);
  if (classe) noeud.className = classe;
  if (texte !== undefined && texte !== null) noeud.textContent = texte;
  return noeud;
}

// `etat` serde : une chaine snake_case, ou {"erreur": "message"}.
function normaliserEtat(etat) {
  if (typeof etat === "string") return { code: etat, message: null };
  if (etat && typeof etat === "object" && typeof etat.erreur === "string") {
    return { code: "erreur", message: etat.erreur };
  }
  return { code: "inconnu", message: null };
}

function libelleEtat(code) {
  return LIBELLES_ETATS[code] || code;
}

function urlFichier(id, nom) {
  return "/projet/" + encodeURIComponent(id) + "/fichier/" + encodeURIComponent(nom);
}

// GET JSON ; leve une erreur avec le message du serveur si !ok.
async function getJSON(url) {
  const reponse = await fetch(url);
  if (!reponse.ok) throw new Error((await reponse.text()) || ("HTTP " + reponse.status));
  return reponse.json();
}

// POST JSON ; renvoie le corps JSON (un projet meme en cas d'erreur pipeline
// cote serveur) ou leve une erreur avec le message texte du serveur.
async function postJSON(url, corps) {
  const reponse = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(corps),
  });
  const type = reponse.headers.get("content-type") || "";
  if (type.includes("application/json")) return reponse.json();
  if (!reponse.ok) throw new Error((await reponse.text()) || ("HTTP " + reponse.status));
  return null;
}

// --- Zone upload ------------------------------------------------------------

async function envoyerAudio(evenement) {
  evenement.preventDefault();
  const champAudio = document.getElementById("champ-audio");
  const statut = document.getElementById("statut-upload");
  if (!champAudio.files.length) return;

  const donnees = new FormData();
  donnees.append("audio", champAudio.files[0]);
  const langue = document.getElementById("champ-langue").value.trim();
  if (langue) donnees.append("langue", langue);

  statut.hidden = false;
  statut.textContent = "Envoi en cours...";
  document.getElementById("bouton-upload").disabled = true;
  try {
    const reponse = await fetch("/audio", { method: "POST", body: donnees });
    const type = reponse.headers.get("content-type") || "";
    if (!type.includes("application/json")) {
      throw new Error((await reponse.text()) || ("HTTP " + reponse.status));
    }
    const projet = await reponse.json();
    const info = normaliserEtat(projet.etat);
    statut.textContent = "Projet cree : " + projet.id + " — " + libelleEtat(info.code) +
      (info.message ? " (" + info.message + ")" : "");
    champAudio.value = "";
    rafraichirListe();
    selectionnerProjet(projet.id);
  } catch (erreur) {
    statut.textContent = "Echec de l'envoi : " + erreur.message;
  } finally {
    document.getElementById("bouton-upload").disabled = false;
  }
}

// --- Zone liste -------------------------------------------------------------

async function rafraichirListe() {
  try {
    const projets = await getJSON("/projets");
    renderListe(projets);
  } catch (erreur) {
    // Un echec de rafraichissement n'interrompt pas l'interface.
    console.warn("liste des projets indisponible :", erreur);
  }
}

function renderListe(projets) {
  const liste = document.getElementById("liste-projets");
  liste.textContent = "";
  document.getElementById("liste-vide").hidden = projets.length > 0;

  for (const resume of projets) {
    const info = normaliserEtat(resume.etat);
    const item = el("li", resume.id === projetCourant ? "projet actif" : "projet");
    const bouton = el("button", "projet-bouton");
    bouton.type = "button";
    bouton.append(el("span", "projet-id", resume.id.slice(0, 8)));
    const etat = el("span", "projet-etat");
    etat.append(el("span", "badge badge-" + info.code, libelleEtat(info.code)));
    bouton.append(etat);
    bouton.append(el("span", "projet-date", resume.maj));
    bouton.addEventListener("click", () => selectionnerProjet(resume.id));
    item.append(bouton);
    liste.append(item);
  }
}

// --- Zone detail ------------------------------------------------------------

function selectionnerProjet(id) {
  projetCourant = id;
  chargerDetail();
}

async function chargerDetail() {
  if (!projetCourant) return;
  try {
    const projet = await getJSON("/projet/" + encodeURIComponent(projetCourant));
    renderDetail(projet);
  } catch (erreur) {
    console.warn("detail indisponible :", erreur);
  }
}

// Rafraichissement automatique : 3 s tant que le pipeline avance seul.
function gererMinuteur(codeEtat) {
  if (ETATS_EN_COURS.has(codeEtat) && minuteurDetail === null) {
    minuteurDetail = setInterval(chargerDetail, 3000);
  } else if (!ETATS_EN_COURS.has(codeEtat) && minuteurDetail !== null) {
    clearInterval(minuteurDetail);
    minuteurDetail = null;
  }
}

function renderDetail(projet) {
  const info = normaliserEtat(projet.etat);
  gererMinuteur(info.code);

  document.getElementById("zone-detail").hidden = false;
  document.getElementById("detail-titre").textContent = "Projet " + projet.id;

  renderEtapes(info);
  renderEtat(projet, info);
  renderTranscription(projet);
  renderScenario(projet);
  renderVisuels(projet);
  renderVoix(projet);
  renderMontage(projet);
  renderPublication(projet);
  renderValidation(projet, info);
}

// Stepper des etapes du pipeline : faites, active, ou neutres si erreur
// (on ne sait pas quelle etape a echoue).
function renderEtapes(info) {
  const zone = document.getElementById("detail-etapes");
  zone.textContent = "";
  const indexActif = ETAPES_PIPELINE.findIndex(([code]) => code === info.code);
  const liste = el("ol", "etapes");
  ETAPES_PIPELINE.forEach(([, libelle], index) => {
    let classe = "etape";
    if (info.code !== "erreur" && indexActif >= 0) {
      if (index < indexActif) classe += " faite";
      else if (index === indexActif) classe += " active";
    }
    const item = el("li", classe);
    item.append(el("span", "etape-point"));
    item.append(el("span", "etape-libelle", libelle));
    liste.append(item);
  });
  zone.append(liste);
}

function renderEtat(projet, info) {
  const zone = document.getElementById("detail-etat");
  zone.textContent = "";
  const ligne = el("p", "etat-courant");
  ligne.append(el("strong", null, "Etat : "));
  ligne.append(el("span", "badge badge-" + info.code, libelleEtat(info.code)));
  zone.append(ligne);
  if (info.code === "erreur" && info.message) {
    zone.append(el("p", "message-erreur", info.message));
  }
}

function renderTranscription(projet) {
  const zone = document.getElementById("detail-transcription");
  zone.textContent = "";
  if (!projet.transcription) return;

  zone.append(el("h3", null, "Transcription"));
  if (projet.transcription.langue) {
    zone.append(el("p", "meta", "Langue : " + projet.transcription.langue));
  }
  zone.append(el("p", "transcription-texte", projet.transcription.texte));
}

function renderScenario(projet) {
  const zone = document.getElementById("detail-scenario");
  zone.textContent = "";
  if (!projet.scenario) return;

  const scenario = projet.scenario;
  zone.append(el("h3", null, "Scenario"));
  zone.append(el("h4", null, scenario.titre));
  zone.append(el("p", "meta", "Public : " + scenario.public));
  zone.append(el("p", "meta", "Style visuel : " + scenario.style_images));

  const liste = el("ol", "scenes");
  scenario.scenes.forEach((scene, index) => {
    const item = el("li", "scene");
    item.append(el("h5", null, "Scene " + (index + 1) +
      " (" + scene.duree_cible + " s)"));
    item.append(el("p", "scene-narration", scene.narration));
    for (const dialogue of scene.dialogues || []) {
      const replique = el("p", "scene-dialogue");
      replique.append(el("strong", null, dialogue.personnage + " : "));
      replique.append(document.createTextNode(dialogue.replique));
      item.append(replique);
    }
    item.append(el("p", "meta", "A l'ecran : " + scene.description_visuelle));
    liste.append(item);
  });
  zone.append(liste);
}

function renderVisuels(projet) {
  const zone = document.getElementById("detail-visuels");
  zone.textContent = "";
  if (!projet.visuels || projet.visuels.length === 0) return;

  zone.append(el("h3", null, "Visuels"));
  const galerie = el("div", "galerie");
  for (const visuel of projet.visuels) {
    const figure = el("figure", "visuel");

    const image = document.createElement("img");
    image.src = urlFichier(projet.id, visuel.fichier);
    image.alt = visuel.titre || ("Scene " + (visuel.scene + 1));
    figure.append(image);

    const legende = el("figcaption", null);
    legende.append(el("span", "visuel-titre",
      "Scene " + (visuel.scene + 1) + " — " + (visuel.titre || "Sans titre")));
    const attribution = el("span", "visuel-attribution");
    attribution.append(document.createTextNode(
      (visuel.auteur || "auteur inconnu") + ", " + visuel.licence + " — "));
    const lien = document.createElement("a");
    lien.href = visuel.url_page;
    lien.target = "_blank";
    lien.rel = "noopener noreferrer";
    lien.textContent = "page de l'oeuvre";
    attribution.append(lien);
    legende.append(attribution);

    // Remplacement de l'image par une nouvelle recherche.
    const formulaire = el("form", "visuel-remplacer");
    const champ = document.createElement("input");
    champ.type = "text";
    champ.placeholder = "Remplacer par...";
    champ.required = true;
    const bouton = el("button", null, "Remplacer");
    bouton.type = "submit";
    formulaire.append(champ, bouton);
    formulaire.addEventListener("submit", (e) => {
      e.preventDefault();
      remplacerVisuel(projet.id, visuel.scene, champ.value.trim());
    });
    legende.append(formulaire);

    figure.append(legende);
    galerie.append(figure);
  }
  zone.append(galerie);
}

async function remplacerVisuel(id, scene, requete) {
  if (!requete) return;
  try {
    const projet = await postJSON("/visuel/remplacer", { id, scene, requete });
    if (projet) renderDetail(projet);
  } catch (erreur) {
    afficherErreurAction("Remplacement impossible : " + erreur.message);
  }
}

function renderVoix(projet) {
  const zone = document.getElementById("detail-voix");
  zone.textContent = "";
  const aDesVoix = projet.voix && projet.voix.length > 0;
  const aDesSousTitres = projet.sous_titres && projet.sous_titres.length > 0;
  if (!aDesVoix && !aDesSousTitres) return;

  zone.append(el("h3", null, "Voix et sous-titres"));
  for (const voix of projet.voix || []) {
    const ligne = el("div", "voix");
    ligne.append(el("span", "voix-meta",
      "Scene " + (voix.scene + 1) + " (" + voix.langue + ", " + voix.duree + " s)"));
    const lecteur = document.createElement("audio");
    lecteur.controls = true;
    lecteur.src = urlFichier(projet.id, voix.fichier);
    ligne.append(lecteur);
    zone.append(ligne);
  }
  for (const srt of projet.sous_titres || []) {
    const lien = document.createElement("a");
    lien.href = urlFichier(projet.id, srt);
    lien.download = srt;
    lien.textContent = "Telecharger " + srt;
    const ligne = el("p", "srt");
    ligne.append(lien);
    zone.append(ligne);
  }
}

// --- Montage ----------------------------------------------------------------

// Preview (ou video finale a defaut) en lecteur video, avec lien de
// telechargement de la video finale 1080p.
function renderMontage(projet) {
  const zone = document.getElementById("detail-montage");
  zone.textContent = "";
  if (!projet.preview && !projet.video) return;

  zone.append(el("h3", null, "Montage"));
  const lecteur = document.createElement("video");
  lecteur.controls = true;
  lecteur.src = urlFichier(projet.id, projet.preview || projet.video);
  zone.append(lecteur);
  if (projet.video) {
    const lien = document.createElement("a");
    lien.href = urlFichier(projet.id, projet.video);
    lien.download = projet.video;
    lien.textContent = "Telecharger la video finale";
    const ligne = el("p", "srt");
    ligne.append(lien);
    zone.append(ligne);
  }
}

// --- Publication ------------------------------------------------------------

// Lien YouTube une fois la video publiee.
function renderPublication(projet) {
  const zone = document.getElementById("detail-publication");
  zone.textContent = "";
  if (!projet.youtube) return;

  zone.append(el("h3", null, "Publication"));
  const ligne = el("p", "srt");
  const lien = document.createElement("a");
  lien.href = projet.youtube.url;
  lien.target = "_blank";
  lien.rel = "noopener noreferrer";
  lien.textContent = "Voir la video sur YouTube";
  ligne.append(lien);
  zone.append(ligne);
}

// --- Validation humaine -----------------------------------------------------

// Etape en attente de decision, s'il y en a une : on n'affiche les boutons
// que pour l'etape correspondant a l'etat courant et non encore tranchee.
function etapeEnAttente(projet, codeEtat) {
  if (codeEtat === "scenario_genere" && projet.validation_scenario == null) return "scenario";
  if (codeEtat === "visuels_prets" && projet.validation_visuels == null) return "visuels";
  if (codeEtat === "voix_pretes" && projet.validation_voix == null) return "voix";
  if (codeEtat === "montage_pret" && projet.validation_montage == null) return "montage";
  return null;
}

const LIBELLES_ETAPES = {
  scenario: "le scenario",
  visuels: "les visuels",
  voix: "les voix",
  montage: "le montage",
};

function renderValidation(projet, info) {
  const zone = document.getElementById("detail-validation");
  zone.textContent = "";
  const etape = etapeEnAttente(projet, info.code);
  if (!etape) return;

  zone.append(el("h3", null, "Validation"));
  zone.append(el("p", null, "Voulez-vous valider " + LIBELLES_ETAPES[etape] + " ?"));
  const actions = el("div", "actions-validation");

  const accepter = el("button", "bouton-accepter", "Accepter");
  accepter.type = "button";
  accepter.addEventListener("click", () => valider(projet.id, etape, "accepte"));

  const rejeter = el("button", "bouton-rejeter", "Rejeter");
  rejeter.type = "button";
  rejeter.addEventListener("click", () => valider(projet.id, etape, "rejete"));

  actions.append(accepter, rejeter);
  zone.append(actions);
}

async function valider(id, etape, decision) {
  try {
    // L'affichage est mis a jour directement depuis le projet renvoye.
    const projet = await postJSON("/valider", { id, decision, etape });
    if (projet) {
      renderDetail(projet);
      rafraichirListe();
    }
  } catch (erreur) {
    afficherErreurAction("Validation impossible : " + erreur.message);
  }
}

function afficherErreurAction(message) {
  const zone = document.getElementById("detail-validation");
  zone.querySelectorAll(".message-erreur").forEach((n) => n.remove());
  zone.append(el("p", "message-erreur", message));
}

// --- Demarrage --------------------------------------------------------------

document.getElementById("form-upload").addEventListener("submit", envoyerAudio);
rafraichirListe();
setInterval(rafraichirListe, 5000);
