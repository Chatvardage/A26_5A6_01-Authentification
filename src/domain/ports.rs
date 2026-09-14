//! "Port" au sens de l'architecture de l'équipe (voir Architecture.md du
//! frontend : "ports/ — Interfaces nécessaires au domaine/application.
//! Elles décrivent CE DONT le métier a besoin, sans définir COMMENT
//! c'est implémenté. Exemple : MessageRepository, Gateway,
//! EventPublisher.").
//!
//! `UtilisateurRepositoryPort` est ce contrat. Deux implémentations
//! existent dans `infrastructure/` :
//! - `PgUtilisateurRepository` : la vraie base PostgreSQL.
//! - `MockUtilisateurRepository` : un stockage en mémoire, pour
//!   développer et tester sans dépendre de PostgreSQL ni du service qui
//!   gère les migrations — même idée que `MockGateway` côté frontend.
//!
//! Ni `AuthService` ni les contrôleurs HTTP ne savent laquelle des deux
//! est utilisée : ils ne connaissent que ce trait (voir `main.rs`, où le
//! bon choix est fait une seule fois, au démarrage, selon `USE_MOCK_DB`).

use async_trait::async_trait;
use uuid::Uuid;

use super::utilisateur::{Role, Utilisateur};

#[async_trait]
pub trait UtilisateurRepositoryPort: Send + Sync {
    async fn creer(
        &self,
        pseudo: &str,
        courriel: &str,
        mot_de_passe_hache: &str,
    ) -> Result<Utilisateur, RepoError>;

    async fn trouver_par_id(&self, id: Uuid) -> Result<Option<Utilisateur>, RepoError>;

    /// Recherche par courriel OU par pseudo (le login accepte les deux).
    async fn trouver_par_identifiant(&self, identifiant: &str) -> Result<Option<Utilisateur>, RepoError>;

    async fn lister(&self) -> Result<Vec<Utilisateur>, RepoError>;

    async fn supprimer(&self, id: Uuid) -> Result<bool, RepoError>;

    async fn changer_role(&self, id: Uuid, role: Role) -> Result<Option<Utilisateur>, RepoError>;
}

/// Erreur générique de stockage, indépendante de PostgreSQL : ni
/// `AuthService`, ni les contrôleurs HTTP ne devraient savoir ce qu'est
/// une erreur `sqlx` — seules les implémentations dans `infrastructure/`
/// la connaissent, et la traduisent vers ce type.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// Le pseudo ou le courriel est déjà utilisé par un autre compte.
    #[error("valeur déjà utilisée (pseudo ou courriel)")]
    Conflit,
    #[error("erreur de stockage: {0}")]
    Interne(String),
}
