//! Connexion à PostgreSQL.
//!
//! Ce service ne crée plus la table `utilisateurs` lui-même : un autre service
//! de l'équipe gère les migrations (voir README, section "Schéma de
//! données attendu"). Ce fichier se contente donc d'ouvrir un pool de
//! connexions — utilisé uniquement quand `USE_MOCK_DB` n'est pas activé
//! (voir `main.rs`).

use sqlx::postgres::{PgPool, PgPoolOptions};

/// Crée un pool de connexions PostgreSQL à partir de `DATABASE_URL`.
/// Un "pool" garde plusieurs connexions ouvertes et les réutilise entre
/// les requêtes HTTP, plutôt que d'ouvrir/fermer une connexion à chaque
/// fois (beaucoup plus rapide).
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}
