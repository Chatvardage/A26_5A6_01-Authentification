//! Implémentation en mémoire de `UtilisateurRepositoryPort` — même idée
//! que `MockGateway` côté frontend (voir MOCKGATEWAY.md dans le dépôt
//! desktop) : permet de développer et de démontrer ce service SANS
//! PostgreSQL et SANS attendre que le service qui gère les migrations
//! ait créé la table `utilisateurs`.
//!
//! Activée avec `USE_MOCK_DB=true` (voir `config.rs` et `main.rs`).
//!
//! ⚠️ Les données vivent uniquement en RAM : tout est perdu au
//! redémarrage du service, et rien n'est partagé entre deux instances.
//! Ne JAMAIS utiliser ce mode pour la démonstration finale du contrôle
//! (l'énoncé exige que PostgreSQL soit la source de vérité) — uniquement
//! pour développer en attendant que la vraie base soit prête.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::ports::{RepoError, UtilisateurRepositoryPort};
use crate::domain::utilisateur::{Role, Utilisateur};

/// `RwLock` (et non un simple `Mutex`) : plusieurs requêtes peuvent lire
/// la liste des utilisateurs en même temps, une seule peut écrire à la
/// fois. `tokio::sync::RwLock` (pas `std::sync::RwLock`) car on le
/// verrouille à travers des `.await` : la version de `std` n'est pas
/// conçue pour ça.
#[derive(Default)]
pub struct MockUtilisateurRepository {
    utilisateurs: RwLock<HashMap<Uuid, Utilisateur>>,
}

impl MockUtilisateurRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl UtilisateurRepositoryPort for MockUtilisateurRepository {
    async fn creer(
        &self,
        pseudo: &str,
        courriel: &str,
        mot_de_passe_hache: &str,
    ) -> Result<Utilisateur, RepoError> {
        let mut utilisateurs = self.utilisateurs.write().await;

        // Reproduit la contrainte UNIQUE de la vraie table Postgres,
        // pour que le comportement de l'API soit identique dans les deux
        // modes (même code d'erreur 409 côté client).
        let deja_pris = utilisateurs
            .values()
            .any(|u| u.pseudo == pseudo || u.courriel == courriel);
        if deja_pris {
            return Err(RepoError::Conflit);
        }

        let maintenant = Utc::now();
        let utilisateur = Utilisateur {
            id: Uuid::new_v4(),
            pseudo: pseudo.to_string(),
            courriel: courriel.to_string(),
            mot_de_passe_hache: mot_de_passe_hache.to_string(),
            role: Role::Utilisateur.as_str().to_string(),
            cree_le: maintenant,
            modifie_le: maintenant,
        };
        utilisateurs.insert(utilisateur.id, utilisateur.clone());
        Ok(utilisateur)
    }

    async fn trouver_par_id(&self, id: Uuid) -> Result<Option<Utilisateur>, RepoError> {
        Ok(self.utilisateurs.read().await.get(&id).cloned())
    }

    async fn trouver_par_identifiant(&self, identifiant: &str) -> Result<Option<Utilisateur>, RepoError> {
        Ok(self
            .utilisateurs
            .read()
            .await
            .values()
            .find(|u| u.pseudo == identifiant || u.courriel == identifiant)
            .cloned())
    }

    async fn lister(&self) -> Result<Vec<Utilisateur>, RepoError> {
        let mut utilisateurs: Vec<Utilisateur> =
            self.utilisateurs.read().await.values().cloned().collect();
        utilisateurs.sort_by(|a, b| b.cree_le.cmp(&a.cree_le));
        Ok(utilisateurs)
    }

    async fn supprimer(&self, id: Uuid) -> Result<bool, RepoError> {
        Ok(self.utilisateurs.write().await.remove(&id).is_some())
    }

    async fn changer_role(&self, id: Uuid, role: Role) -> Result<Option<Utilisateur>, RepoError> {
        let mut utilisateurs = self.utilisateurs.write().await;
        Ok(utilisateurs.get_mut(&id).map(|u| {
            u.role = role.as_str().to_string();
            u.modifie_le = Utc::now();
            u.clone()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creer_puis_retrouver_par_pseudo_ou_courriel() {
        let repo = MockUtilisateurRepository::new();
        let utilisateur = repo.creer("alice", "alice@example.com", "hash").await.unwrap();

        assert_eq!(
            repo.trouver_par_identifiant("alice").await.unwrap().unwrap().id,
            utilisateur.id
        );
        assert_eq!(
            repo.trouver_par_identifiant("alice@example.com").await.unwrap().unwrap().id,
            utilisateur.id
        );
    }

    #[tokio::test]
    async fn refuse_un_pseudo_deja_pris() {
        let repo = MockUtilisateurRepository::new();
        repo.creer("alice", "alice@example.com", "hash").await.unwrap();

        let resultat = repo.creer("alice", "autre@example.com", "hash").await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));
    }

    #[tokio::test]
    async fn supprimer_un_utilisateur_inexistant_retourne_false() {
        let repo = MockUtilisateurRepository::new();
        assert!(!repo.supprimer(Uuid::new_v4()).await.unwrap());
    }
}
