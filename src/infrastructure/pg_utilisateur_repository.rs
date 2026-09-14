//! Implémentation PostgreSQL de `UtilisateurRepositoryPort` (voir
//! `domain::ports`). Regroupe tout le SQL lié aux utilisateurs : si un
//! jour on change de base de données, seul ce fichier change.
//!
//! Ce service ne crée plus lui-même la table `utilisateurs` : un autre
//! service de l'équipe gère les migrations. Le schéma attendu par les
//! requêtes ci-dessous (noms de table et de colonnes en français) est
//! documenté dans le README, section "Schéma de données attendu" — c'est
//! la référence à partager avec la personne qui gère les migrations.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::ports::{RepoError, UtilisateurRepositoryPort};
use crate::domain::utilisateur::{Role, Utilisateur};

#[derive(Clone)]
pub struct PgUtilisateurRepository {
    pool: PgPool,
}

impl PgUtilisateurRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UtilisateurRepositoryPort for PgUtilisateurRepository {
    async fn creer(
        &self,
        pseudo: &str,
        courriel: &str,
        mot_de_passe_hache: &str,
    ) -> Result<Utilisateur, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            r#"
            INSERT INTO utilisateurs (pseudo, courriel, mot_de_passe_hache)
            VALUES ($1, $2, $3)
            RETURNING id, pseudo, courriel, mot_de_passe_hache, role, cree_le, modifie_le
            "#,
        )
        .bind(pseudo)
        .bind(courriel)
        .bind(mot_de_passe_hache)
        .fetch_one(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn trouver_par_id(&self, id: Uuid) -> Result<Option<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            "SELECT id, pseudo, courriel, mot_de_passe_hache, role, cree_le, modifie_le \
             FROM utilisateurs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn trouver_par_identifiant(&self, identifiant: &str) -> Result<Option<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            "SELECT id, pseudo, courriel, mot_de_passe_hache, role, cree_le, modifie_le \
             FROM utilisateurs WHERE courriel = $1 OR pseudo = $1",
        )
        .bind(identifiant)
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn lister(&self) -> Result<Vec<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            "SELECT id, pseudo, courriel, mot_de_passe_hache, role, cree_le, modifie_le \
             FROM utilisateurs ORDER BY cree_le DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn supprimer(&self, id: Uuid) -> Result<bool, RepoError> {
        let resultat = sqlx::query("DELETE FROM utilisateurs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(mapper_erreur_sql)?;
        Ok(resultat.rows_affected() > 0)
    }

    async fn changer_role(&self, id: Uuid, role: Role) -> Result<Option<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            r#"
            UPDATE utilisateurs SET role = $2, modifie_le = now()
            WHERE id = $1
            RETURNING id, pseudo, courriel, mot_de_passe_hache, role, cree_le, modifie_le
            "#,
        )
        .bind(id)
        .bind(role.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }
}

/// Traduit une erreur SQL "brute" (ex. contrainte UNIQUE violée) en
/// `RepoError`, le seul type d'erreur que le reste de l'application
/// connaît.
fn mapper_erreur_sql(e: sqlx::Error) -> RepoError {
    if let sqlx::Error::Database(db_err) = &e {
        // Code "23505" = unique_violation en PostgreSQL.
        if db_err.code().as_deref() == Some("23505") {
            return RepoError::Conflit;
        }
    }
    RepoError::Interne(e.to_string())
}
