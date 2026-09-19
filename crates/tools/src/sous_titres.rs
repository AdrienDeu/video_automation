//! Generation des sous-titres `.srt` synchronises sur les voix off (phase 4,
//! voir `docs/agenda.md`).
//!
//! Un fichier `.srt` est produit par langue : une entree par replique
//! (narration puis dialogues de chaque scene). Les durees reelles des audios
//! generes (une entree audio par scene) bornent chaque scene ; a
//! l'interieur d'une scene, le temps est reparti entre les repliques au
//! prorata de leur longueur en caracteres.
//!
//! Les horodatages suivent la timeline du **rendu final**, qui n'est pas la
//! somme des durees audio : le Monteur enchaine les scenes par un fondu de
//! [`DUREE_FONDU`] secondes, absorbe par les deux scenes qu'il chevauche.

use video_core::scenario::Scenario;

use crate::ffmpeg::DUREE_FONDU;

/// Formate un horodatage `.srt` (`HH:MM:SS,mmm`) a partir d'un temps en
/// secondes.
fn horodatage(secondes: f64) -> String {
    let total_ms = (secondes.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// Repliques d'une scene, dans l'ordre de diffusion : la narration puis les
/// dialogues (prefixes du personnage, ex. `Prof : Comment ... ?`).
fn repliques(scene: &video_core::scenario::Scene) -> Vec<String> {
    let narration = scene.narration.trim();
    let mut textes = Vec::new();
    if !narration.is_empty() {
        textes.push(narration.to_string());
    }
    for dialogue in &scene.dialogues {
        let replique = dialogue.replique.trim();
        if !replique.is_empty() {
            textes.push(format!("{} : {replique}", dialogue.personnage));
        }
    }
    textes
}

/// Genere le contenu d'un fichier `.srt` pour la video entiere.
///
/// `durees` porte la duree reelle de l'audio de chaque scene (meme ordre que
/// `Scenario.scenes`) ; a defaut de mesure, la duree cible de la scene est
/// utilisee. Les scenes sans aucune replique ne produisent pas d'entree.
///
/// Les debuts de scene sont ceux du montage : le fondu enchaine fait demarrer
/// la scene `i` a `somme(durees[0..i]) - DUREE_FONDU * i`. Cumuler les durees
/// brutes ferait deriver les sous-titres de [`DUREE_FONDU`] par scene (3,5 s
/// au bout de 8 scenes).
pub fn generer_srt(scenario: &Scenario, durees: &[f64]) -> String {
    let mut srt = String::new();
    let mut index = 1;
    let mut curseur = 0.0; // debut de la scene courante dans le rendu final

    let derniere = scenario.scenes.len().saturating_sub(1);
    for (i, scene) in scenario.scenes.iter().enumerate() {
        let duree = durees.get(i).copied().unwrap_or(scene.duree_cible);
        // Fenetre visible de la scene : sa duree audio moins le fondu que la
        // scene suivante lui prend (la derniere scene n'en cede aucun). Une
        // duree plus courte que le fondu ne peut pas etre amputee ; le montage
        // refuse de toute facon ce cas (`ffmpeg::construire_args`).
        let fenetre = if i < derniere && duree > DUREE_FONDU {
            duree - DUREE_FONDU
        } else {
            duree
        };
        let textes = repliques(scene);
        // Repartition du temps de la scene au prorata des longueurs de texte.
        let poids: Vec<usize> = textes.iter().map(|t| t.chars().count().max(1)).collect();
        let total: usize = poids.iter().sum();

        let mut debut = curseur;
        let mut poids_cumule = 0;
        for (texte, poids) in textes.iter().zip(&poids) {
            poids_cumule += *poids;
            let fin = curseur + fenetre * (poids_cumule as f64 / total as f64);
            srt.push_str(&format!(
                "{index}\n{} --> {}\n{texte}\n\n",
                horodatage(debut),
                horodatage(fin)
            ));
            index += 1;
            debut = fin;
        }
        curseur += fenetre;
    }
    srt
}

#[cfg(test)]
mod tests {
    use super::*;
    use video_core::scenario::{Dialogue, Scene};

    fn scenario_deux_scenes() -> Scenario {
        Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: vec![
                Scene {
                    narration: "Bonjour le monde.".to_string(),
                    dialogues: vec![Dialogue {
                        personnage: "Prof".to_string(),
                        replique: "Une question ?".to_string(),
                    }],
                    description_visuelle: "Visuel 0".to_string(),
                    duree_cible: 8.0,
                },
                Scene {
                    narration: "Fin de la video.".to_string(),
                    dialogues: vec![],
                    description_visuelle: "Visuel 1".to_string(),
                    duree_cible: 4.0,
                },
            ],
        }
    }

    #[test]
    fn formate_les_horodatages() {
        assert_eq!(horodatage(0.0), "00:00:00,000");
        assert_eq!(horodatage(1.5), "00:00:01,500");
        assert_eq!(horodatage(65.25), "00:01:05,250");
        assert_eq!(horodatage(3600.0), "01:00:00,000");
        assert_eq!(horodatage(3661.007), "01:01:01,007");
    }

    #[test]
    fn genere_un_srt_synchronise_sur_les_durees_reelles() {
        // Durees reelles mesurees : 6 s pour la scene 0, 2 s pour la scene 1.
        let srt = generer_srt(&scenario_deux_scenes(), &[6.0, 2.0]);
        let blocs: Vec<&str> = srt.split("\n\n").filter(|b| !b.is_empty()).collect();

        // 3 entrees : narration + dialogue de la scene 0, narration scene 1.
        assert_eq!(blocs.len(), 3, "{srt}");
        // La scene 0 cede DUREE_FONDU a la scene 1 : sa fenetre est de 5,5 s,
        // repartie au prorata (narration 17 caracteres, dialogue prefixe
        // « Prof : Une question ? » 21 caracteres) → 17/38 puis 21/38.
        assert!(
            blocs[0].starts_with("1\n00:00:00,000 --> 00:00:02,461\n"),
            "{srt}"
        );
        assert!(blocs[0].ends_with("Bonjour le monde."));
        // Le dialogue demarre ou la narration s'arrete, fin de fenetre a 5,5 s.
        assert!(
            blocs[1].starts_with("2\n00:00:02,461 --> 00:00:05,500\n"),
            "{srt}"
        );
        assert!(blocs[1].ends_with("Prof : Une question ?"));
        // La scene 1 demarre la ou le fondu la fait apparaitre (5,5 s, pas
        // 6 s) et, derniere scene, garde sa duree pleine de 2 s.
        assert!(
            blocs[2].starts_with("3\n00:00:05,500 --> 00:00:07,500\n"),
            "{srt}"
        );
    }

    #[test]
    fn retombe_sur_la_duree_cible_sans_mesure() {
        // Pas de durees mesurees : la duree cible de chaque scene fait foi
        // (8 s puis 4 s), diminuee du fondu pour la scene non finale.
        let srt = generer_srt(&scenario_deux_scenes(), &[]);
        assert!(srt.contains("00:00:07,500 --> 00:00:11,500"), "{srt}");
    }

    /// Garde-fou anti-regression : les debuts de scene du `.srt` doivent
    /// coincider avec les `offset` des `xfade` du template de montage. Tant
    /// que les deux modules derivaient chacun leur timeline, les sous-titres
    /// prenaient DUREE_FONDU de retard par scene.
    #[test]
    fn les_debuts_de_scene_collent_aux_fondus_du_montage() {
        let durees = [10.0, 10.0, 10.0, 10.0];
        let scenario = Scenario {
            titre: "Sujet".to_string(),
            public: "tout public".to_string(),
            style_images: "photos".to_string(),
            scenes: durees
                .iter()
                .enumerate()
                .map(|(i, duree)| Scene {
                    narration: format!("Narration de la scene {i}."),
                    dialogues: vec![],
                    description_visuelle: format!("Visuel {i}"),
                    duree_cible: *duree,
                })
                .collect(),
        };
        let srt = generer_srt(&scenario, &durees);

        // Debuts reels dans le rendu : 0 pour la scene 0, puis les offsets des
        // fondus enchaines construits par le Monteur.
        let montage: Vec<crate::ffmpeg::SceneMontage> = durees
            .iter()
            .enumerate()
            .map(|(i, duree)| crate::ffmpeg::SceneMontage {
                image: format!("scene-{i}.jpg"),
                voix: format!("voix-{i}.mp3"),
                duree: *duree,
            })
            .collect();
        let args = crate::ffmpeg::construire_args(
            &montage,
            None,
            &crate::ffmpeg::PROFIL_PREVIEW,
            "video.mp4",
        )
        .expect("arguments de montage valides");
        let position = args
            .iter()
            .position(|a| a == "-filter_complex")
            .expect("option -filter_complex presente");
        let offsets: Vec<f64> = args[position + 1]
            .split(';')
            .filter_map(|chaine| chaine.split("offset=").nth(1))
            .filter_map(|reste| reste.split('[').next())
            .map(|offset| offset.parse().expect("offset numerique"))
            .collect();
        assert_eq!(offsets.len(), durees.len() - 1, "un fondu par transition");

        let debuts: Vec<&str> = srt
            .split("\n\n")
            .filter(|bloc| !bloc.is_empty())
            .map(|bloc| {
                bloc.lines()
                    .nth(1)
                    .expect("ligne d'horodatage")
                    .split(" --> ")
                    .next()
                    .expect("horodatage de debut")
            })
            .collect();
        assert_eq!(debuts.len(), durees.len(), "une entree par scene\n{srt}");

        assert_eq!(debuts[0], horodatage(0.0), "{srt}");
        for (i, offset) in offsets.iter().enumerate() {
            assert_eq!(
                debuts[i + 1],
                horodatage(*offset),
                "scene {} desynchronisee du montage\n{srt}",
                i + 1
            );
        }
    }
}
