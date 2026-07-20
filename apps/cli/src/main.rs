//! CLI de pilotage du projet (debug, demonstration).
//!
//! Phase 0 : une seule commande, `demo-llm`, qui execute le hello-world de
//! tool calling contre la vraie API Mistral. Parsing des arguments a la main
//! (pas de clap en phase 0).

use anyhow::{Context, Result};
use llm::client;
use llm::hello::{executer_hello_world, DireBonjour};
use video_core::config::Config;

const USAGE: &str = "Usage : cli <commande>

Commandes :
  demo-llm    Execute le hello-world de tool calling via l'API Mistral
              (requiert MISTRAL_API_KEY dans l'environnement ou .env)";

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
