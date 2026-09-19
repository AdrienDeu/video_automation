//! Verifie que l'interface a ete construite avant de compiler le serveur.
//!
//! `src/ui.rs` embarque `ui/dist/` via `include_str!`. Ce dossier est genere
//! par Vite et n'est pas versionne : sans lui, la compilation echouerait sur
//! une erreur de macro peu parlante. On la remplace par une consigne claire.

use std::path::Path;

const FICHIERS: &[&str] = &["index.html", "app.js", "style.css"];

fn main() {
    let dist = Path::new("ui/dist");
    println!("cargo:rerun-if-changed=ui/dist");

    let manquants: Vec<&str> = FICHIERS
        .iter()
        .copied()
        .filter(|nom| !dist.join(nom).exists())
        .collect();

    if !manquants.is_empty() {
        println!(
            "cargo:warning=interface non construite : {}",
            manquants.join(", ")
        );
        panic!(
            "l'interface web n'a pas ete construite ({} absent(s) de apps/server/ui/dist).\n\
             Lancez : (cd apps/server/ui && npm ci && npm run build)",
            manquants.join(", ")
        );
    }

    for nom in FICHIERS {
        println!("cargo:rerun-if-changed=ui/dist/{nom}");
    }
}
