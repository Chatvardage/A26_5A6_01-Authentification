//! Point d'entrée du service d'authentification ChatVardage.
//!
//! Ce que fait ce fichier, dans l'ordre : (1) charge la configuration,
//! (2) ouvre le pool de connexions PostgreSQL, (3) assemble les services
//! (infrastructure -> application), (4) démarre le serveur HTTP. C'est le
//! SEUL fichier où toutes les couches se rencontrent — le reste du code
//! ne connaît que ses propres voisins.

mod application;
mod config;
mod domain;
mod error;
mod infrastructure;
mod interface;
mod state;

use std::sync::Arc;

use config::Config;
use domain::ports::UtilisateurRepositoryPort;
use infrastructure::{db, jwt::JwtService, pg_utilisateur_repository::PgUtilisateurRepository};
use state::AppState;

#[tokio::main]
async fn main() {
    config::charger_dotenv();

    // Journalisation : affiche les logs `info` par défaut, ou selon la
    // variable d'env RUST_LOG (ex. `RUST_LOG=debug`), chargée depuis
    // `.env` par `config::charger_dotenv()` ci-dessus si nécessaire.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::depuis_env();

    tracing::info!("connexion à la base de données...");
    let pool = db::connect(&config.database_url)
        .await
        .expect("impossible de se connecter à PostgreSQL — vérifiez DATABASE_URL");
    let depot_utilisateurs: Arc<dyn UtilisateurRepositoryPort> =
        Arc::new(PgUtilisateurRepository::new(pool));

    let jwt_service = Arc::new(JwtService::new(
        config.jwt_secret.clone(),
        config.jwt_duree_de_vie_heures,
    ));
    let auth_service = application::auth_service::AuthService::new(
        depot_utilisateurs.clone(),
        jwt_service.clone(),
    );

    let state = AppState {
        auth_service,
        depot_utilisateurs,
        jwt_service,
    };

    let app = interface::http::routes::construire(state, config.cors_allowed_origins.as_deref());

    let adresse = format!("0.0.0.0:{}", config.port);
    tracing::info!("service d'authentification à l'écoute sur {adresse}");

    let listener = tokio::net::TcpListener::bind(&adresse)
        .await
        .expect("impossible d'ouvrir le port d'écoute");

    axum::serve(listener, app)
        .with_graceful_shutdown(attendre_signal_arret())
        .await
        .expect("le serveur HTTP s'est arrêté de façon inattendue");
}

#[cfg(unix)]
async fn attendre_signal_arret() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm =
        signal(SignalKind::terminate()).expect("impossible d'écouter le signal SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
    tracing::info!("signal d'arrêt reçu, fin des requêtes en cours");
}

#[cfg(not(unix))]
async fn attendre_signal_arret() {
    tokio::signal::ctrl_c()
        .await
        .expect("impossible d'écouter le signal d'arrêt");
    tracing::info!("signal d'arrêt reçu, fin des requêtes en cours");
}
