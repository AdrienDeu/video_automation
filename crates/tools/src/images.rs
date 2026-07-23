//! Outil `choisir_image` : recherche et telecharge une image licenciee pour
//! une scene (phase 3, voir `docs/architecture.md` §7 et §9).
//!
//! Deux sources sont interrogees : l'agregateur Creative Commons **Openverse**
//! et **Wikimedia Commons**. Seules les licences compatibles (CC0, CC-BY,
//! domaine public) sont retenues ; le candidat le plus pertinent (recouvrement
//! de mots-cles avec la requete et le style) est telecharge dans le dossier
//! du projet et son attribution est conservee dans l'`Asset` retourne.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use video_core::asset::{Asset, SourceImage};
use video_core::error::Error;

/// Endpoint de recherche d'images d'Openverse.
const URL_OPENVERSE: &str = "https://api.openverse.org/v1/images/";

/// Endpoint de l'API MediaWiki de Wikimedia Commons.
const URL_COMMONS: &str = "https://commons.wikimedia.org/w/api.php";

/// Licences Openverse demandees cote serveur : CC0, CC-BY, domaine public.
const LICENCES_OPENVERSE: &str = "cc0,by,pdm";

/// Largeur minimale exigee pour un visuel 1080p confortable (en pixels).
const LARGEUR_MIN: u32 = 500;

/// Taille maximale d'un telechargement d'image (20 Mio).
const TAILLE_MAX_TELECHARGEMENT: usize = 20 * 1024 * 1024;

/// Extensions d'images exploitables par ffmpeg en phase 5 (pas de SVG/GIF).
const EXTENSIONS_IMAGE: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Mots vides usuels du francais, ignores a l'extraction de mots-cles.
const MOTS_VIDES: &[&str] = &[
    "dans", "avec", "sans", "sont", "cote", "côte", "sous", "pour", "plus", "tres", "entre",
    "chaque", "cette", "leurs", "etre", "par", "les", "des", "une", "aux", "sur", "est", "ces",
    "ses", "trois", "aussi", "ainsi",
];

/// Extrait jusqu'a `n` mots-cles discriminants d'une description : mots
/// alphabetiques d'au moins 4 caracteres, hors mots vides, tries par longueur
/// decroissante (les plus specifiques d'abord), en minuscules et sans doublon.
pub fn mots_cles(description: &str, n: usize) -> Vec<String> {
    let mut mots: Vec<String> = description
        .split(|c: char| !c.is_alphabetic())
        .filter(|m| m.len() >= 4)
        .map(str::to_lowercase)
        .filter(|m| !MOTS_VIDES.contains(&m.as_str()))
        .collect();
    mots.sort_by_key(|m| std::cmp::Reverse(m.len()));
    mots.dedup();
    mots.truncate(n);
    mots
}

/// Recherche d'image en mode degrade (repli sans LLM, quand la boucle
/// agent/outil n'a produit aucune image) : essaie des requetes de la plus
/// precise a la plus generique — mots-cles extraits de la description de la
/// scene, puis le style — et retourne la premiere image trouvee.
///
/// Les descriptions sont en francais alors que les sources sont indexees
/// majoritairement en anglais : des requetes courtes maximisent le rappel.
///
/// # Erreurs
/// `Error::Tool` si aucune requete de la cascade ne produit d'image.
pub async fn choisir_image_degrade(
    http: &reqwest::Client,
    dossier: &Path,
    scene: usize,
    description: &str,
    style: &str,
) -> Result<Asset, Error> {
    let cles = mots_cles(description, 4);
    let mut requetes: Vec<String> = Vec::new();
    if cles.len() >= 3 {
        requetes.push(cles.join(" "));
    }
    if let Some(deux) = cles.get(..2) {
        requetes.push(deux.join(" "));
    }
    if let Some(premier) = cles.first() {
        requetes.push(premier.clone());
    }
    let style = style.trim();
    if !style.is_empty() {
        requetes.push(style.to_string());
    }
    requetes.dedup();

    let mut derniere_erreur = None;
    for requete in &requetes {
        match choisir_image(http, dossier, scene, requete, style).await {
            Ok(asset) => return Ok(asset),
            Err(e) => derniere_erreur = Some(format!("« {requete} » : {e}")),
        }
    }
    Err(Error::Tool(format!(
        "repli degrade sans resultat ({})",
        derniere_erreur.unwrap_or_else(|| "aucune requete exploitable".to_string())
    )))
}

/// Un candidat image issu d'une des deux sources, avant telechargement.
#[derive(Debug, Clone)]
pub struct CandidatImage {
    /// Titre de l'oeuvre.
    pub titre: Option<String>,
    /// Auteur de l'oeuvre.
    pub auteur: Option<String>,
    /// URL de la page de l'oeuvre.
    pub url_page: String,
    /// URL directe du fichier image.
    pub url_fichier: String,
    /// Licence lisible (ex. `CC BY 2.0`).
    pub licence: String,
    /// URL du texte de licence.
    pub licence_url: Option<String>,
    /// Largeur en pixels.
    pub largeur: Option<u32>,
    /// Hauteur en pixels.
    pub hauteur: Option<u32>,
    /// Source du candidat.
    pub source: SourceImage,
    /// Mots-cles associes a l'image (titre + tags), pour le scoring.
    pub mots_cles: Vec<String>,
}

/// Construit le client HTTP partage des recherches d'images, avec un
/// User-Agent explicite (requis par Wikimedia).
pub fn client_http() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .user_agent("video-automation/0.1 (pipeline de videos educatives)")
        .build()
        .map_err(|e| Error::Tool(format!("construction du client HTTP : {e}")))
}

/// Une licence est acceptee si elle est CC0, CC-BY (toutes versions) ou
/// domaine public — jamais CC-BY-SA ni NC/ND (voir `docs/architecture.md` §9).
pub fn licence_acceptee(licence: &str) -> bool {
    let licence = licence.trim().to_lowercase();
    if matches!(licence.as_str(), "cc0" | "by" | "pdm" | "public domain") {
        return true;
    }
    // Forme Wikimedia : « CC BY 2.0 », « CC BY 3.0 »... (jamais « CC BY-SA »).
    licence
        .strip_prefix("cc by ")
        .is_some_and(|reste| reste.starts_with(|c: char| c.is_ascii_digit()))
}

/// Note de pertinence d'un candidat : nombre de mots distincts de la requete
/// et du style presents dans les mots-cles de l'image (titre + tags).
pub fn score_pertinence(requete: &str, style: &str, candidat: &CandidatImage) -> u32 {
    let mots_image: std::collections::HashSet<String> = candidat
        .mots_cles
        .iter()
        .map(|m| m.to_lowercase())
        .collect();
    let mots_recherche = format!("{requete} {style}");
    mots_recherche
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|m| m.len() >= 3)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .filter(|mot| mots_image.iter().any(|mi| mi.contains(mot.as_str())))
        .count() as u32
}

/// Extrait l'extension d'une URL d'image, si elle est exploitable.
fn extension_image(url: &str) -> Option<&'static str> {
    let chemin = url.split(['?', '#']).next()?;
    let extension = chemin.rsplit('.').next()?.to_lowercase();
    EXTENSIONS_IMAGE
        .iter()
        .copied()
        .find(|ext| *ext == extension)
}

// --- Openverse ---

#[derive(Debug, Deserialize)]
struct ReponseOpenverse {
    #[serde(default)]
    results: Vec<ResultatOpenverse>,
}

#[derive(Debug, Deserialize)]
struct ResultatOpenverse {
    title: Option<String>,
    url: String,
    foreign_landing_url: Option<String>,
    creator: Option<String>,
    license: String,
    license_version: Option<String>,
    license_url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default)]
    tags: Vec<TagOpenverse>,
}

#[derive(Debug, Deserialize)]
struct TagOpenverse {
    name: String,
}

/// Rend la licence Openverse lisible : `by` + `2.0` → `CC BY 2.0`.
fn licence_lisible_openverse(code: &str, version: Option<&str>) -> String {
    match code {
        "cc0" => "CC0".to_string(),
        "pdm" => "Public domain".to_string(),
        autre => format!(
            "CC {}{}",
            autre.to_uppercase(),
            version.map(|v| format!(" {v}")).unwrap_or_default()
        ),
    }
}

fn en_candidat_openverse(r: ResultatOpenverse) -> Option<CandidatImage> {
    if !licence_acceptee(&r.license) {
        return None;
    }
    let mut mots_cles: Vec<String> = r.tags.into_iter().map(|t| t.name).collect();
    if let Some(titre) = &r.title {
        mots_cles.push(titre.clone());
    }
    Some(CandidatImage {
        titre: r.title,
        auteur: r.creator,
        url_page: r.foreign_landing_url.unwrap_or_else(|| r.url.clone()),
        url_fichier: r.url,
        licence: licence_lisible_openverse(&r.license, r.license_version.as_deref()),
        licence_url: r.license_url,
        largeur: r.width,
        hauteur: r.height,
        source: SourceImage::Openverse,
        mots_cles,
    })
}

/// Recherche des candidats sur Openverse (licences filtrees cote serveur).
async fn rechercher_openverse(
    http: &reqwest::Client,
    requete: &str,
) -> Result<Vec<CandidatImage>, Error> {
    let reponse = http
        .get(URL_OPENVERSE)
        .query(&[
            ("q", requete),
            ("license", LICENCES_OPENVERSE),
            ("page_size", "20"),
        ])
        .send()
        .await
        .map_err(|e| Error::Tool(format!("recherche Openverse : {e}")))?;
    let statut = reponse.status();
    if !statut.is_success() {
        return Err(Error::Tool(format!(
            "Openverse a repondu {statut} : {}",
            reponse.text().await.unwrap_or_default()
        )));
    }
    let brute: ReponseOpenverse = reponse
        .json()
        .await
        .map_err(|e| Error::Tool(format!("reponse Openverse illisible : {e}")))?;
    Ok(brute
        .results
        .into_iter()
        .filter_map(en_candidat_openverse)
        .collect())
}

// --- Wikimedia Commons ---

#[derive(Debug, Deserialize)]
struct ReponseCommons {
    query: Option<RequeteCommons>,
}

#[derive(Debug, Deserialize)]
struct RequeteCommons {
    #[serde(default)]
    pages: HashMap<String, PageCommons>,
}

#[derive(Debug, Deserialize)]
struct PageCommons {
    title: String,
    imageinfo: Option<Vec<InfoCommons>>,
}

#[derive(Debug, Deserialize)]
struct InfoCommons {
    url: String,
    descriptionurl: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    extmetadata: Option<MetadonneesCommons>,
}

#[derive(Debug, Deserialize)]
struct MetadonneesCommons {
    #[serde(rename = "LicenseShortName")]
    licence: Option<ChampCommons>,
    #[serde(rename = "Artist")]
    auteur: Option<ChampCommons>,
}

#[derive(Debug, Deserialize)]
struct ChampCommons {
    value: String,
}

/// Retire les balises HTML d'un champ Wikimedia (ex. `<a ...>Nom</a>`).
fn sans_html(brut: &str) -> String {
    let mut propre = String::with_capacity(brut.len());
    let mut dans_balise = false;
    for c in brut.chars() {
        match c {
            '<' => dans_balise = true,
            '>' => dans_balise = false,
            _ if !dans_balise => propre.push(c),
            _ => {}
        }
    }
    propre.trim().to_string()
}

fn en_candidat_commons(page: PageCommons) -> Option<CandidatImage> {
    let info = page.imageinfo?.into_iter().next()?;
    let meta = info.extmetadata?;
    let licence = meta.licence?.value;
    if !licence_acceptee(&licence) {
        return None;
    }
    // Titre « File:Nom du fichier.jpg » → nom lisible + mots-cles.
    let titre = page
        .title
        .strip_prefix("File:")
        .unwrap_or(&page.title)
        .to_string();
    Some(CandidatImage {
        titre: Some(titre.clone()),
        auteur: meta
            .auteur
            .map(|a| sans_html(&a.value))
            .filter(|a| !a.is_empty()),
        url_page: info.descriptionurl.unwrap_or_else(|| info.url.clone()),
        url_fichier: info.url,
        licence,
        licence_url: None,
        largeur: info.width,
        hauteur: info.height,
        source: SourceImage::WikimediaCommons,
        mots_cles: vec![titre],
    })
}

/// Recherche des candidats sur Wikimedia Commons.
async fn rechercher_commons(
    http: &reqwest::Client,
    requete: &str,
) -> Result<Vec<CandidatImage>, Error> {
    let reponse = http
        .get(URL_COMMONS)
        .query(&[
            ("action", "query"),
            ("format", "json"),
            ("generator", "search"),
            ("gsrsearch", requete),
            ("gsrnamespace", "6"), // espace « File: »
            ("gsrlimit", "20"),
            ("prop", "imageinfo"),
            ("iiprop", "url|size|extmetadata"),
        ])
        .send()
        .await
        .map_err(|e| Error::Tool(format!("recherche Wikimedia Commons : {e}")))?;
    let statut = reponse.status();
    if !statut.is_success() {
        return Err(Error::Tool(format!(
            "Wikimedia Commons a repondu {statut} : {}",
            reponse.text().await.unwrap_or_default()
        )));
    }
    let brute: ReponseCommons = reponse
        .json()
        .await
        .map_err(|e| Error::Tool(format!("reponse Wikimedia illisible : {e}")))?;
    let pages = brute
        .query
        .map(|q| q.pages.into_values().collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(pages.into_iter().filter_map(en_candidat_commons).collect())
}

/// Retient les candidats exploitables : extension supportee et largeur
/// suffisante (ou inconnue, la verification aura lieu au telechargement).
fn exploitable(candidat: &CandidatImage) -> bool {
    extension_image(&candidat.url_fichier).is_some()
        && candidat.largeur.is_none_or(|l| l >= LARGEUR_MIN)
}

/// Choisit la meilleure image licenciee pour une scene et la telecharge dans
/// le dossier du projet sous `scene-<n>.<ext>`.
///
/// `requete` est la recherche (idealement en anglais, les tags des sources le
/// sont majoritairement) ; `style` ne sert qu'au scoring de pertinence.
///
/// # Erreurs
/// `Error::Tool` si les deux sources echouent, si aucun candidat licencie
/// exploitable n'est trouve, ou si le telechargement echoue.
pub async fn choisir_image(
    http: &reqwest::Client,
    dossier: &Path,
    scene: usize,
    requete: &str,
    style: &str,
) -> Result<Asset, Error> {
    let (openverse, commons) = tokio::join!(
        rechercher_openverse(http, requete),
        rechercher_commons(http, requete)
    );
    let mut candidats: Vec<CandidatImage> = Vec::new();
    for (source, resultat) in [("Openverse", openverse), ("Wikimedia Commons", commons)] {
        match resultat {
            Ok(trouves) => candidats.extend(trouves),
            // Une source en panne ne bloque pas l'autre.
            Err(e) => eprintln!("avertissement : {source} indisponible : {e}"),
        }
    }

    let meilleur = candidats
        .into_iter()
        .filter(exploitable)
        .max_by_key(|c| {
            (
                score_pertinence(requete, style, c),
                c.largeur.unwrap_or(0) * c.hauteur.unwrap_or(0),
            )
        })
        .ok_or_else(|| Error::Tool(format!("aucune image licenciee trouvee pour « {requete} »")))?;

    // Telechargement du fichier retenu.
    let octets = http
        .get(&meilleur.url_fichier)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("telechargement de l'image : {e}")))?;
    let statut = octets.status();
    if !statut.is_success() {
        return Err(Error::Tool(format!(
            "telechargement de l'image : statut {statut}"
        )));
    }
    let octets = octets
        .bytes()
        .await
        .map_err(|e| Error::Tool(format!("lecture de l'image : {e}")))?;
    if octets.len() > TAILLE_MAX_TELECHARGEMENT {
        return Err(Error::Tool(format!(
            "image trop volumineuse ({} Mio, maximum {} Mio)",
            octets.len() / 1024 / 1024,
            TAILLE_MAX_TELECHARGEMENT / 1024 / 1024
        )));
    }

    let extension = extension_image(&meilleur.url_fichier).expect("deja verifiee");
    let nom_fichier = format!("scene-{scene}.{extension}");
    std::fs::create_dir_all(dossier)?;
    std::fs::write(dossier.join(&nom_fichier), &octets)?;

    Ok(Asset {
        scene,
        fichier: nom_fichier,
        source: meilleur.source,
        titre: meilleur.titre,
        auteur: meilleur.auteur,
        url_page: meilleur.url_page,
        url_fichier: meilleur.url_fichier,
        licence: meilleur.licence,
        licence_url: meilleur.licence_url,
        largeur: meilleur.largeur,
        hauteur: meilleur.hauteur,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtre_les_licences() {
        assert!(licence_acceptee("cc0"));
        assert!(licence_acceptee("by"));
        assert!(licence_acceptee("pdm"));
        assert!(licence_acceptee("Public domain"));
        assert!(licence_acceptee("CC BY 2.0"));
        assert!(licence_acceptee("CC BY 4.0"));
        assert!(!licence_acceptee("CC BY-SA 4.0"));
        assert!(!licence_acceptee("by-sa"));
        assert!(!licence_acceptee("CC BY-NC 2.0"));
        assert!(!licence_acceptee("by-nc-nd"));
    }

    #[test]
    fn extrait_les_mots_cles_discriminants() {
        let cles = mots_cles(
            "Pièce sombre aux murs de pierre, éclairée par une lumière douce. \
             Trois illustrations médiévales sont accrochées côte à côte.",
            4,
        );
        // Mots vides et mots courts ignores, longueur decroissante, sans doublon.
        assert_eq!(cles.len(), 4);
        assert_eq!(cles[0], "illustrations");
        assert!(cles.contains(&"médiévales".to_string()));
        assert!(!cles.iter().any(|m| m == "trois" || m == "sont" || m == "aux"));
        let mut sans_doublon = cles.clone();
        sans_doublon.dedup();
        assert_eq!(cles, sans_doublon);

        // Description vide : aucun mot-cle, pas de panique.
        assert!(mots_cles("", 4).is_empty());
    }

    #[test]
    fn parse_la_reponse_openverse() {
        // Forme reelle de la reponse Openverse (tronquee).
        let json = r#"{
            "result_count": 2,
            "results": [
                {
                    "id": "abc",
                    "title": "Feuille au soleil",
                    "url": "https://images.test/feuille.jpg",
                    "foreign_landing_url": "https://photos.test/feuille",
                    "creator": "Jane Doe",
                    "license": "by",
                    "license_version": "2.0",
                    "license_url": "https://creativecommons.org/licenses/by/2.0/",
                    "width": 1024,
                    "height": 768,
                    "tags": [ { "name": "leaf" }, { "name": "sun" } ]
                },
                {
                    "id": "def",
                    "title": "Logo NC",
                    "url": "https://images.test/logo.png",
                    "foreign_landing_url": null,
                    "creator": "John",
                    "license": "by-nc",
                    "license_version": "4.0",
                    "license_url": null,
                    "width": 800,
                    "height": 600,
                    "tags": []
                }
            ]
        }"#;
        let brute: ReponseOpenverse = serde_json::from_str(json).expect("JSON valide");
        let candidats: Vec<_> = brute
            .results
            .into_iter()
            .filter_map(en_candidat_openverse)
            .collect();

        // Le candidat by-nc est ecarte.
        assert_eq!(candidats.len(), 1);
        let c = &candidats[0];
        assert_eq!(c.titre.as_deref(), Some("Feuille au soleil"));
        assert_eq!(c.licence, "CC BY 2.0");
        assert_eq!(c.url_page, "https://photos.test/feuille");
        assert!(c.mots_cles.contains(&"leaf".to_string()));
    }

    #[test]
    fn parse_la_reponse_commons() {
        // Forme reelle de la reponse Commons (tronquee).
        let json = r#"{
            "query": {
                "pages": {
                    "123": {
                        "pageid": 123,
                        "title": "File:Feuille verte.jpg",
                        "imageinfo": [
                            {
                                "url": "https://upload.wikimedia.org/feuille.jpg",
                                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Feuille_verte.jpg",
                                "width": 2000,
                                "height": 1500,
                                "extmetadata": {
                                    "LicenseShortName": { "value": "CC BY-SA 4.0" },
                                    "Artist": { "value": "<a href=\"//x\">Kelvin</a>" }
                                }
                            }
                        ]
                    },
                    "456": {
                        "pageid": 456,
                        "title": "File:Photosynthese.jpg",
                        "imageinfo": [
                            {
                                "url": "https://upload.wikimedia.org/photo.jpg",
                                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Photosynthese.jpg",
                                "width": 1600,
                                "height": 1200,
                                "extmetadata": {
                                    "LicenseShortName": { "value": "CC0" },
                                    "Artist": { "value": "<span>Anne</span>" }
                                }
                            }
                        ]
                    }
                }
            }
        }"#;
        let brute: ReponseCommons = serde_json::from_str(json).expect("JSON valide");
        let pages: Vec<_> = brute.query.expect("query").pages.into_values().collect();
        let candidats: Vec<_> = pages.into_iter().filter_map(en_candidat_commons).collect();

        // La page CC BY-SA est ecartee.
        assert_eq!(candidats.len(), 1);
        let c = &candidats[0];
        assert_eq!(c.titre.as_deref(), Some("Photosynthese.jpg"));
        assert_eq!(c.auteur.as_deref(), Some("Anne"));
        assert_eq!(c.licence, "CC0");
        assert_eq!(c.source, SourceImage::WikimediaCommons);
    }

    #[test]
    fn retire_le_html_des_champs_commons() {
        assert_eq!(sans_html("<a href=\"//x\">Kelvin</a>"), "Kelvin");
        assert_eq!(sans_html("<span class=\"s\">Anne</span>"), "Anne");
        assert_eq!(sans_html("Deja propre"), "Deja propre");
    }

    #[test]
    fn score_selon_recouvrement_de_mots() {
        let candidat = |titre: &str, tags: &[&str]| CandidatImage {
            titre: Some(titre.to_string()),
            auteur: None,
            url_page: String::new(),
            url_fichier: "https://x.test/a.jpg".to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur: None,
            hauteur: None,
            source: SourceImage::Openverse,
            mots_cles: std::iter::once(titre.to_string())
                .chain(tags.iter().map(|t| t.to_string()))
                .collect(),
        };
        let pertinent = candidat("Green leaf in sunlight", &["leaf", "photosynthesis"]);
        let vague = candidat("Abstract texture", &["wallpaper"]);

        assert!(
            score_pertinence("photosynthesis leaf", "green", &pertinent)
                > score_pertinence("photosynthesis leaf", "green", &vague)
        );
        assert_eq!(score_pertinence("photosynthesis leaf", "green", &vague), 0);
    }

    #[test]
    fn filtre_les_candidats_exploitables() {
        let candidat = |url: &str, largeur: Option<u32>| CandidatImage {
            titre: None,
            auteur: None,
            url_page: String::new(),
            url_fichier: url.to_string(),
            licence: "CC0".to_string(),
            licence_url: None,
            largeur,
            hauteur: None,
            source: SourceImage::Openverse,
            mots_cles: vec![],
        };
        assert!(exploitable(&candidat("https://x.test/a.jpg", Some(1024))));
        assert!(exploitable(&candidat("https://x.test/a.png?dl=1", None)));
        // Trop petit ou format non exploitable par ffmpeg.
        assert!(!exploitable(&candidat("https://x.test/a.jpg", Some(200))));
        assert!(!exploitable(&candidat("https://x.test/a.svg", Some(2000))));
        assert!(!exploitable(&candidat("https://x.test/a.gif", None)));
    }

    /// Verification reelle contre les deux sources : ignoree tant que
    /// `VIDEO_TEST_RESEAU` n'est pas definie (donc en CI).
    #[tokio::test]
    async fn choisir_une_image_reelle() {
        if std::env::var("VIDEO_TEST_RESEAU").is_err() {
            eprintln!("VIDEO_TEST_RESEAU absente : choisir_une_image_reelle ignore.");
            return;
        }
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let http = client_http().expect("client HTTP");
        let asset = choisir_image(&http, temp.path(), 0, "photosynthesis leaf", "macro")
            .await
            .expect("une image licenciee doit etre trouvee");

        assert!(temp.path().join(&asset.fichier).exists());
        assert!(licence_acceptee(&asset.licence));
        assert!(!asset.attribution().is_empty());
    }
}
