//! Service de l'interface web embarquee (phase 5).
//!
//! L'interface est une application React (`apps/server/ui/`, sources TypeScript
//! sous `src/`) construite par Vite en trois fichiers a noms fixes dans
//! `ui/dist/`, compiles dans le binaire via `include_str!` : aucune dependance
//! ni repertoire de fichiers statiques a deployer.
//!
//! `ui/dist/` n'est pas versionne : `build.rs` verifie sa presence et rappelle
//! la commande de construction (`npm ci && npm run build`) a defaut.

use axum::http::header;
use axum::response::Html;

const INDEX: &str = include_str!("../ui/dist/index.html");
const APP_JS: &str = include_str!("../ui/dist/app.js");
const STYLE_CSS: &str = include_str!("../ui/dist/style.css");

/// `GET /` : page unique de l'interface.
pub async fn get_index() -> Html<&'static str> {
    Html(INDEX)
}

/// `GET /app.js` : logique cliente de l'interface.
pub async fn get_app_js() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

/// `GET /style.css` : feuille de style de l'interface.
pub async fn get_style_css() -> ([(header::HeaderName, &'static str); 1], &'static str) {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}
