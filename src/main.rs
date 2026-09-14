//! Point d'entrée du service d'authentification ChatVardage.
//!
//! Ce que fait ce fichier, dans l'ordre : (1) charge la configuration,
//! (2) choisit et prépare le dépôt utilisateurs — PostgreSQL, ou un mock
//! en mémoire si `USE_MOCK_DB=true` (voir `config.rs` et `domain::ports`
//! pour le pourquoi), (3) assemble les services (infrastructure ->
//! application), (4) démarre le serveur HTTP. C'est le SEUL fichier où
//! toutes les couches se rencontrent — le reste du code ne connaît que
//! ses propres voisins.

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
use infrastructure::{
    db, jwt::JwtService, mock_utilisateur_repository::MockUtilisateurRepository,
    pg_utilisateur_repository::PgUtilisateurRepository,
};
use state::AppState;

#[tokio::main]
async fn main() {
    // Journalisation : affiche les logs `info` par défaut, ou selon la
    // variable d'env RUST_LOG (ex. `RUST_LOG=debug`).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::depuis_env();

    let depot_utilisateurs: Arc<dyn UtilisateurRepositoryPort> = if config.use_mock_db {
        tracing::warn!(
            "USE_MOCK_DB=true : dépôt en mémoire utilisé — aucune donnée persistée, \
             aucune connexion à PostgreSQL. Ne pas utiliser ce mode pour la démonstration \
             finale (voir README)."
        );
        Arc::new(MockUtilisateurRepository::new())
    } else {
        tracing::info!("connexion à la base de données...");
        let pool = db::connect(
            config
                .database_url
                .as_deref()
                .expect("validé au chargement de la configuration"),
        )
        .await
        .expect("impossible de se connecter à PostgreSQL — vérifiez DATABASE_URL");
        Arc::new(PgUtilisateurRepository::new(pool))
    };

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

    let app = interface::http::routes::construire(state);

    let adresse = format!("0.0.0.0:{}", config.port);
    tracing::info!("service d'authentification à l'écoute sur {adresse}");

    let listener = tokio::net::TcpListener::bind(&adresse)
        .await
        .expect("impossible d'ouvrir le port d'écoute");

    axum::serve(listener, app)
        .await
        .expect("le serveur HTTP s'est arrêté de façon inattendue");
}
