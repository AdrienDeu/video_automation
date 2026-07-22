//! Outil `ffmpeg` : rendu de la video finale et de la preview de validation
//! (phase 5, voir `docs/architecture.md` §7 et `docs/agenda.md` phase 5).
//!
//! **Whitelist de templates uniquement** : les commandes sont construites par
//! [`construire_args`], parametree uniquement par des noms de fichiers, des
//! durees et un profil de rendu constant — jamais de ligne de commande libre.
//! Les arguments sont passes en argv (pas de shell) et ffmpeg est execute avec
//! le dossier du projet comme repertoire courant : tous les chemins sont de
//! simples noms de fichiers verifies par [`nom_fichier_valide`], donc confines
//! a `data/<id>/`.
//!
//! Le template unique produit, en une seule passe : un ken-burns par scene
//! (`zoompan` apres mise a l'echelle x2 pour la fluidite), un fondu enchaine
//! entre scenes (`xfade`, duree [`DUREE_FONDU`]) avec son pendant audio
//! (`acrossfade`), la normalisation EBU R128 (`loudnorm`) et l'incrustation
//! des sous-titres (`subtitles`, libass) a partir du `.srt` du Conteur.

use std::path::Path;

use video_core::error::Error;

/// Duree du fondu enchaine entre deux scenes, en secondes (video `xfade` et
/// audio `acrossfade`).
pub const DUREE_FONDU: f64 = 0.5;

/// Cadence de sortie des templates (images par seconde).
const FPS: u32 = 25;

/// Une scene a monter : image fixe et voix off dont la duree fait foi.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneMontage {
    /// Nom du fichier image dans le dossier du projet (ex. `scene-0.jpg`).
    pub image: String,
    /// Nom du fichier audio de la voix off (ex. `voix-a1b2....mp3`).
    pub voix: String,
    /// Duree de la scene en secondes (duree reelle de la voix off).
    pub duree: f64,
}

/// Profil d'encodage d'un rendu : resolution, preset x264 et CRF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfilRendu {
    /// Largeur de la video en pixels.
    pub largeur: u32,
    /// Hauteur de la video en pixels.
    pub hauteur: u32,
    /// Preset x264 (compromis vitesse/compression).
    pub preset: &'static str,
    /// CRF x264 (qualite cible, plus bas = meilleure qualite).
    pub crf: u8,
}

/// Video finale : 1080p, qualite soignee.
pub const PROFIL_FINAL: ProfilRendu = ProfilRendu {
    largeur: 1920,
    hauteur: 1080,
    preset: "medium",
    crf: 20,
};

/// Preview de validation humaine : 480p, encodage rapide.
pub const PROFIL_PREVIEW: ProfilRendu = ProfilRendu {
    largeur: 854,
    hauteur: 480,
    preset: "veryfast",
    crf: 30,
};

/// Un nom de fichier est-il un simple nom, sans traversée ni caractere
/// special des filtres ffmpeg (`:`, `;`, `'`, crochets, virgule) ?
///
/// Garde-fou de la whitelist : les templates n'acceptent que des noms plats,
/// ce qui confine tous les acces au dossier du projet (repertoire courant de
/// la commande) et interdit toute injection dans le graphe de filtres.
fn nom_fichier_valide(nom: &str) -> bool {
    !nom.is_empty()
        && !nom.contains("..")
        && !nom.contains(['/', '\\', ':', ';', '\'', '[', ']', ','])
}

/// Verifie que `ffmpeg` est present et executable (`ffmpeg -version`).
pub async fn ffmpeg_disponible() -> bool {
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .map(|sortie| sortie.status.success())
        .unwrap_or(false)
}

/// Construit les arguments argv de la commande ffmpeg du template de montage
/// (fonction pure, testable sans ffmpeg).
///
/// Sortie attendue (forme) :
/// `ffmpeg -y -loop 1 -t <d> -i <img> ... -i <voix> ... -filter_complex
/// "<ken-burns/xfade/acrossfade/loudnorm/subtitles>" -map [vout] -map [aout]
/// -c:v libx264 ... <sortie>`, a executer avec le dossier du projet comme
/// repertoire courant.
///
/// # Erreurs
/// `Error::Tool` si la liste de scenes est vide, si une duree est invalide
/// (<= 0, ou <= [`DUREE_FONDU`] des qu'un fondu enchaine est necessaire), ou
/// si un nom de fichier est invalide.
pub fn construire_args(
    scenes: &[SceneMontage],
    sous_titres: Option<&str>,
    profil: &ProfilRendu,
    sortie: &str,
) -> Result<Vec<String>, Error> {
    if scenes.is_empty() {
        return Err(Error::Tool("montage sans aucune scene".to_string()));
    }
    for scene in scenes {
        if !nom_fichier_valide(&scene.image) || !nom_fichier_valide(&scene.voix) {
            return Err(Error::Tool(format!(
                "nom de fichier invalide pour le montage : `{}` / `{}`",
                scene.image, scene.voix
            )));
        }
        if scene.duree <= 0.0 {
            return Err(Error::Tool(format!(
                "duree de scene invalide ({:.3} s)",
                scene.duree
            )));
        }
        // Un fondu enchaine doit tenir dans chaque scene qu'il chevauche.
        if scenes.len() > 1 && scene.duree <= DUREE_FONDU {
            return Err(Error::Tool(format!(
                "scene trop courte pour le fondu enchaine ({:.3} s, minimum {DUREE_FONDU} s)",
                scene.duree
            )));
        }
    }
    if let Some(srt) = sous_titres {
        if !nom_fichier_valide(srt) {
            return Err(Error::Tool(format!(
                "nom de fichier de sous-titres invalide : `{srt}`"
            )));
        }
    }
    if !nom_fichier_valide(sortie) {
        return Err(Error::Tool(format!(
            "nom de fichier de sortie invalide : `{sortie}`"
        )));
    }

    let mut args: Vec<String> = vec!["-y".into(), "-loglevel".into(), "error".into()];
    // Entrees 0..n-1 : les images, bouclees sur la duree de leur scene.
    for scene in scenes {
        args.extend([
            "-loop".into(),
            "1".into(),
            "-t".into(),
            format!("{:.3}", scene.duree),
            "-i".into(),
            scene.image.clone(),
        ]);
    }
    // Entrees n..2n-1 : les voix off.
    for scene in scenes {
        args.extend(["-i".into(), scene.voix.clone()]);
    }
    args.extend([
        "-filter_complex".into(),
        graphe_filtres(scenes, sous_titres, profil),
        "-map".into(),
        "[vout]".into(),
        "-map".into(),
        "[aout]".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        profil.preset.into(),
        "-crf".into(),
        profil.crf.to_string(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-movflags".into(),
        "+faststart".into(),
        sortie.into(),
    ]);
    Ok(args)
}

/// Construit le graphe `-filter_complex` du template : ken-burns par scene,
/// fondus enchaines video et audio, normalisation `loudnorm`, sous-titres.
fn graphe_filtres(
    scenes: &[SceneMontage],
    sous_titres: Option<&str>,
    profil: &ProfilRendu,
) -> String {
    let nombre = scenes.len();
    let (w, h) = (profil.largeur, profil.hauteur);
    let mut chaines: Vec<String> = Vec::new();

    // Ken-burns : l'image est d'abord agrandie x2 (zoom fluide, sans
    // pixelisation), puis zoomee lentement image par image via zoompan.
    for index in 0..nombre {
        chaines.push(format!(
            "[{index}:v]scale={w2}:{h2}:force_original_aspect_ratio=increase,\
             crop={w2}:{h2},setsar=1,\
             zoompan=z='zoom+0.0006':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s={w}x{h}:fps={FPS},\
             format=yuv420p[v{index}]",
            w2 = w * 2,
            h2 = h * 2,
        ));
    }

    // Fondus enchaines video : l'offset du i-ieme xfade est la duree cumulee
    // des scenes precedentes moins les fondus deja absorbes.
    let mut precedent = "v0".to_string();
    let mut cumul = scenes[0].duree;
    for (index, scene) in scenes.iter().enumerate().skip(1) {
        chaines.push(format!(
            "[{precedent}][v{index}]xfade=transition=fade:duration={DUREE_FONDU}:offset={:.3}[x{index}]",
            cumul - DUREE_FONDU
        ));
        precedent = format!("x{index}");
        cumul += scene.duree - DUREE_FONDU;
    }

    // Incrustation des sous-titres (libass lit le .srt) ; `null` sinon, pour
    // conserver un label de sortie unique `[vout]`.
    match sous_titres {
        Some(srt) => chaines.push(format!("[{precedent}]subtitles={srt}[vout]")),
        None => chaines.push(format!("[{precedent}]null[vout]")),
    }

    // Voix off : fondu enchaine audio calque sur le video, puis normalisation
    // EBU R128 (cible -16 LUFS, voix off).
    let mut precedent_audio = format!("{nombre}:a");
    for index in 1..nombre {
        chaines.push(format!(
            "[{precedent_audio}][{}:a]acrossfade=d={DUREE_FONDU}[c{index}]",
            nombre + index
        ));
        precedent_audio = format!("c{index}");
    }
    chaines.push(format!(
        "[{precedent_audio}]loudnorm=I=-16:TP=-1.5:LRA=11[aout]"
    ));

    chaines.join(";")
}

/// Execute le template de montage dans le dossier du projet : construit les
/// arguments puis lance ffmpeg (argv, sans shell, repertoire courant = dossier
/// du projet).
///
/// # Erreurs
/// `Error::Tool` si les arguments sont invalides, si ffmpeg ne se lance pas,
/// ou si le rendu echoue (stderr conserve dans le message).
pub async fn monter(
    dossier: &Path,
    scenes: &[SceneMontage],
    sous_titres: Option<&str>,
    profil: &ProfilRendu,
    sortie: &str,
) -> Result<(), Error> {
    let args = construire_args(scenes, sous_titres, profil, sortie)?;
    let resultat = tokio::process::Command::new("ffmpeg")
        .args(&args)
        .current_dir(dossier)
        // Tue le process si la future est abandonnee (annulation du montage
        // via `tokio::select!` cote Monteur).
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("lancement de ffmpeg : {e}")))?;
    if !resultat.status.success() {
        let stderr = String::from_utf8_lossy(&resultat.stderr);
        let detail: String = stderr.chars().take(500).collect();
        return Err(Error::Tool(format!("ffmpeg a echoue : {detail}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(image: &str, voix: &str, duree: f64) -> SceneMontage {
        SceneMontage {
            image: image.to_string(),
            voix: voix.to_string(),
            duree,
        }
    }

    fn graphe(args: &[String]) -> &str {
        let position = args
            .iter()
            .position(|a| a == "-filter_complex")
            .expect("option -filter_complex presente");
        args[position + 1].as_str()
    }

    #[test]
    fn les_noms_de_fichiers_sont_filtres() {
        assert!(nom_fichier_valide("scene-0.jpg"));
        assert!(nom_fichier_valide("sous-titres-fr.srt"));
        assert!(!nom_fichier_valide(""));
        assert!(!nom_fichier_valide("../secret.png"));
        assert!(!nom_fichier_valide("sous/dossier.png"));
        assert!(!nom_fichier_valide("C:\\image.png"));
        // Caracteres speciaux du graphe de filtres.
        assert!(!nom_fichier_valide("sub:inject.srt"));
        assert!(!nom_fichier_valide("s'rt.srt"));
        assert!(!nom_fichier_valide("a;b.png"));
    }

    #[test]
    fn args_une_seule_scene_sans_fondu() {
        let scenes = vec![scene("scene-0.jpg", "voix-0.mp3", 8.0)];
        let args = construire_args(
            &scenes,
            Some("sous-titres-fr.srt"),
            &PROFIL_FINAL,
            "video.mp4",
        )
        .expect("arguments valides");

        assert_eq!(args[0], "-y");
        // Une image bouclee sur 8 s, puis une entree audio.
        let attendu: Vec<&str> = vec!["-loop", "1", "-t", "8.000", "-i", "scene-0.jpg"];
        assert!(args.windows(attendu.len()).any(|f| f == attendu.as_slice()));
        let script = graphe(&args);
        assert!(script.contains("zoompan"), "{script}");
        assert!(script.contains("s=1920x1080:fps=25"), "{script}");
        assert!(!script.contains("xfade"), "{script}");
        assert!(!script.contains("acrossfade"), "{script}");
        // La voix est l'entree 1 (l'image est l'entree 0).
        assert!(
            script.contains("[1:a]loudnorm=I=-16:TP=-1.5:LRA=11[aout]"),
            "{script}"
        );
        assert!(
            script.contains("subtitles=sous-titres-fr.srt[vout]"),
            "{script}"
        );
        // Sortie 1080p soignee.
        assert!(args.windows(2).any(|f| f == ["-crf", "20"]));
        assert!(args.windows(2).any(|f| f == ["-preset", "medium"]));
        assert_eq!(args.last().expect("sortie"), "video.mp4");
    }

    #[test]
    fn args_trois_scenes_enchainent_les_fondus() {
        let scenes = vec![
            scene("scene-0.jpg", "voix-0.mp3", 8.0),
            scene("scene-1.png", "voix-1.mp3", 4.0),
            scene("scene-2.jpg", "voix-2.mp3", 6.0),
        ];
        let args = construire_args(&scenes, None, &PROFIL_PREVIEW, "preview.mp4")
            .expect("arguments valides");
        let script = graphe(&args);

        // Deux xfades : offsets 8 - 0.5 = 7.5 puis 8 + 4 - 1 = 11.
        assert!(
            script.contains("xfade=transition=fade:duration=0.5:offset=7.500[x1]"),
            "{script}"
        );
        assert!(
            script.contains("xfade=transition=fade:duration=0.5:offset=11.000[x2]"),
            "{script}"
        );
        // Deux acrossfades : entrees audio 3, 4 et 5 (0-2 = images).
        assert!(
            script.contains("[3:a][4:a]acrossfade=d=0.5[c1]"),
            "{script}"
        );
        assert!(script.contains("[c1][5:a]acrossfade=d=0.5[c2]"), "{script}");
        assert!(script.contains("[c2]loudnorm"), "{script}");
        // Sans .srt : passe-plat `null` vers [vout].
        assert!(script.contains("[x2]null[vout]"), "{script}");
        // Preview 480p rapide.
        assert!(script.contains("s=854x480"), "{script}");
        assert!(args.windows(2).any(|f| f == ["-crf", "30"]));
        assert!(args.windows(2).any(|f| f == ["-preset", "veryfast"]));
        assert_eq!(args.last().expect("sortie"), "preview.mp4");
    }

    #[test]
    fn refuse_des_entrees_invalides() {
        let scenes = vec![scene("scene-0.jpg", "voix-0.mp3", 8.0)];

        // Liste vide.
        assert!(construire_args(&[], None, &PROFIL_FINAL, "video.mp4").is_err());
        // Duree nulle.
        assert!(construire_args(
            &[scene("scene-0.jpg", "voix-0.mp3", 0.0)],
            None,
            &PROFIL_FINAL,
            "video.mp4"
        )
        .is_err());
        // Scene trop courte pour le fondu enchaine.
        assert!(construire_args(
            &[
                scene("scene-0.jpg", "voix-0.mp3", 8.0),
                scene("scene-1.jpg", "voix-1.mp3", 0.3),
            ],
            None,
            &PROFIL_FINAL,
            "video.mp4"
        )
        .is_err());
        // Traversee de chemin dans l'image, la voix, le srt ou la sortie.
        assert!(construire_args(
            &[scene("../secret.png", "voix-0.mp3", 8.0)],
            None,
            &PROFIL_FINAL,
            "video.mp4"
        )
        .is_err());
        assert!(construire_args(&scenes, Some("../srt.srt"), &PROFIL_FINAL, "video.mp4").is_err());
        assert!(construire_args(&scenes, None, &PROFIL_FINAL, "/tmp/video.mp4").is_err());
    }

    /// PNG 1x1 valide (rouge), fixture pour le test d'integration reel.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc,
        0xcf, 0xc0, 0x50, 0x0f, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Genere un WAV valide (tonalite 440 Hz, PCM 16 bits mono, 8 kHz).
    ///
    /// Une tonalite et non un silence numerique : `loudnorm` produit des
    /// valeurs NaN/Inf sur un silence parfait et fait echouer l'encodage aac.
    fn wav_tonalite(duree_ms: u32) -> Vec<u8> {
        let taille_donnees = duree_ms * 16;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + taille_donnees).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&taille_donnees.to_le_bytes());
        for i in 0..(duree_ms * 8) {
            let echantillon = (f64::sin(2.0 * std::f64::consts::PI * 440.0 * f64::from(i) / 8000.0)
                * 8000.0) as i16;
            wav.extend_from_slice(&echantillon.to_le_bytes());
        }
        wav
    }

    /// Rendu reel d'une mini-video (2 scenes PNG + tonalites WAV) : saute si
    /// ffmpeg est absent de l'environnement (pattern `VIDEO_TEST_RESEAU`).
    #[tokio::test]
    async fn rend_une_mini_video_reelle() {
        if !ffmpeg_disponible().await {
            eprintln!("ffmpeg absent : rend_une_mini_video_reelle ignore.");
            return;
        }
        let temp = tempfile::tempdir().expect("dossier temporaire");
        let dossier = temp.path();
        std::fs::write(dossier.join("scene-0.png"), PNG_1X1).expect("image 0");
        std::fs::write(dossier.join("scene-1.png"), PNG_1X1).expect("image 1");
        std::fs::write(dossier.join("voix-0.wav"), wav_tonalite(1500)).expect("voix 0");
        std::fs::write(dossier.join("voix-1.wav"), wav_tonalite(1500)).expect("voix 1");
        std::fs::write(
            dossier.join("sous-titres-fr.srt"),
            "1\n00:00:00,000 --> 00:00:01,500\nBonjour.\n\n2\n00:00:01,500 --> 00:00:03,000\nFin.\n",
        )
        .expect("srt");

        let scenes = vec![
            scene("scene-0.png", "voix-0.wav", 1.5),
            scene("scene-1.png", "voix-1.wav", 1.5),
        ];
        monter(
            dossier,
            &scenes,
            Some("sous-titres-fr.srt"),
            &PROFIL_PREVIEW,
            "preview.mp4",
        )
        .await
        .expect("le rendu preview doit aboutir");

        let preview = dossier.join("preview.mp4");
        assert!(preview.exists());
        assert!(std::fs::metadata(&preview).expect("metadonnees").len() > 0);
        // Duree attendue : 1.5 + 1.5 - fondu 0.5 = 2.5 s (tolerance x264).
        let duree = crate::voix::duree_audio_secondes(&preview)
            .await
            .expect("duree mesurable via ffprobe");
        assert!((duree - 2.5).abs() < 0.3, "duree inattendue : {duree} s");
    }
}
