//! CLI de pilotage du projet (debug, demonstration).
//!
//! Phase 0 : une seule commande, `demo-llm`, qui execute le hello-world de
//! tool calling contre la vraie API Mistral. Parsing des arguments a la main
//! (pas de clap en phase 0).
//! Phase 6 : commande `youtube-auth`, bootstrap OAuth YouTube (flux
//! installed app avec redirection loopback) : le consentement se fait une
//! fois ici, hors serveur, et le refresh token est stocke dans
//! `data/youtube_token.json` (jamais commite).

use anyhow::{Context, Result};
use llm::client;
use llm::hello::{executer_hello_world, DireBonjour};
use video_core::config::Config;

const USAGE: &str = "Usage : cli <commande>

Commandes :
  demo-llm      Execute le hello-world de tool calling via l'API Mistral
                (requiert MISTRAL_API_KEY dans l'environnement ou .env)
  youtube-auth  Autorise l'application a publier sur YouTube et stocke le
                refresh token dans data/youtube_token.json
                (requiert YOUTUBE_CLIENT_ID et YOUTUBE_CLIENT_SECRET)";

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let mut args = std::env::args();
    let _binaire = args.next();
    let Some(commande) = args.next() else {
        eprintln!("{USAGE}");
        return Ok(());
    };

    match commande.as_str() {
        "demo-llm" => demo_llm().await,
        "youtube-auth" => youtube_auth().await,
        autre => {
            eprintln!("commande inconnue : {autre}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Construit l'agent Mistral via la facade `llm` et execute le hello-world.
async fn demo_llm() -> Result<()> {
    let config = Config::load().context("chargement de la configuration")?;

    let agent = client::construire_agent_depuis_config(&config.llm)
        .context("construction de l'agent (MISTRAL_API_KEY est-elle definie ?)")?
        .preamble("Tu reponds toujours en francais.")
        .tool(DireBonjour)
        .build();

    let reponse = executer_hello_world(&agent)
        .await
        .context("execution du hello-world de tool calling")?;
    println!("{reponse}");
    Ok(())
}

/// Bootstrap OAuth YouTube (flux installed app, une seule fois) :
///
/// 1. ouvre un listener loopback (`http://127.0.0.1:<port>`) et affiche
///    l'URL de consentement Google (portee `youtube.upload` seule) ;
/// 2. attend la redirection du navigateur et en extrait le code ;
/// 3. echange le code contre un refresh token ;
/// 4. stocke le refresh token dans `data/youtube_token.json` (permissions
///    0600, dossier gitignore).
async fn youtube_auth() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let config = Config::load().context("chargement de la configuration")?;
    let variable = |nom: &str| std::env::var(nom).ok().filter(|v| !v.is_empty());
    let client_id = variable("YOUTUBE_CLIENT_ID")
        .context("YOUTUBE_CLIENT_ID absente de l'environnement (ou de .env)")?;
    let client_secret = variable("YOUTUBE_CLIENT_SECRET")
        .context("YOUTUBE_CLIENT_SECRET absente de l'environnement (ou de .env)")?;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("ouverture du listener loopback")?;
    let port = listener
        .local_addr()
        .context("adresse du listener loopback")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    println!("Ouvrez cette URL dans votre navigateur, connectez-vous avec le compte de la chaine de test et acceptez :\n");
    println!(
        "{}\n",
        tools::youtube::url_consentement(&client_id, &redirect_uri)
    );
    println!("En attente du consentement (Ctrl+C pour annuler)...");

    let (mut socket, _) = listener
        .accept()
        .await
        .context("attente de la redirection OAuth")?;
    // La premiere ligne de la requete (`GET /?code=... HTTP/1.1`) suffit ;
    // on lit jusqu'au premier saut de ligne.
    let mut recu = Vec::new();
    let mut bloc = [0u8; 4096];
    while !recu.windows(2).any(|f| f == b"\r\n") {
        let n = socket
            .read(&mut bloc)
            .await
            .context("lecture de la redirection OAuth")?;
        if n == 0 || recu.len() > 64 * 1024 {
            break;
        }
        recu.extend_from_slice(&bloc[..n]);
    }
    let requete = String::from_utf8_lossy(&recu);
    let code = tools::youtube::extraire_code(&requete)
        .context("la redirection ne contient pas de code d'autorisation (consentement refuse ?)")?;
    let page = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
                <h1>Autorisation recue</h1><p>Vous pouvez fermer cette page et retourner au terminal.</p>";
    socket.write_all(page.as_bytes()).await.ok();

    let http = tools::youtube::client_http().context("construction du client HTTP")?;
    let refresh_token = tools::youtube::echanger_code(
        &http,
        &tools::youtube::EndpointsYoutube::default(),
        &client_id,
        &client_secret,
        &code,
        &redirect_uri,
    )
    .await
    .context("echange du code contre un refresh token")?;

    std::fs::create_dir_all(&config.data_dir).context("creation du dossier de donnees")?;
    let chemin = config
        .data_dir
        .join(video_core::config::FICHIER_JETON_YOUTUBE);
    let contenu = serde_json::json!({ "refresh_token": refresh_token });
    std::fs::write(&chemin, serde_json::to_string_pretty(&contenu)?)
        .context("ecriture du fichier de jeton")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&chemin, std::fs::Permissions::from_mode(0o600))
            .context("permissions 0600 sur le fichier de jeton")?;
    }

    println!(
        "\nRefresh token stocke dans {} (jamais commite).",
        chemin.display()
    );
    println!(
        "La publication est active : les videos seront publiees en '{}' (section [youtube] de config.toml).",
        config.youtube.visibilite
    );
    Ok(())
}
