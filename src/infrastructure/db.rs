//! Connexion à PostgreSQL.
//!
//! Ce service ne crée pas directement la table `utilisateurs` au runtime :
//! les migrations Prisma versionnées dans ce dépôt sont appliquées séparément,
//! notamment par `npx prisma migrate deploy` au démarrage du conteneur.
//! Ce fichier se contente d'ouvrir le pool PostgreSQL utilisé en production.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echoue_si_aucun_serveur_necoute_sur_lurl() {
        // Port 1 : réservé, rien n'y écoute — connexion refusée
        // immédiatement, pas besoin d'une vraie base pour ce test.
        let resultat = connect("postgres://user:pass@127.0.0.1:1/inexistant").await;
        assert!(resultat.is_err());
    }
}
