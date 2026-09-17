//! Implémentation en mémoire de `UtilisateurRepositoryPort` réservée aux
//! tests (`#[cfg(test)]` — jamais compilée dans le binaire de production,
//! contrairement à l'ancien `MockUtilisateurRepository` qu'elle
//! remplace). Utilisée par les tests HTTP de bout en bout
//! (`interface/http/routes.rs`) pour exercer les contrôleurs/middleware
//! sans dépendre d'une vraie connexion PostgreSQL.
//!
//! `PgUtilisateurRepository` reste la seule implémentation utilisée en
//! production (voir `main.rs`) et a ses propres tests, contre une vraie
//! base, dans `pg_utilisateur_repository.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::ports::{RepoError, UtilisateurRepositoryPort};
use crate::domain::utilisateur::{Role, Utilisateur};

#[derive(Default)]
pub struct FakeUtilisateurRepository {
    utilisateurs: RwLock<HashMap<Uuid, Utilisateur>>,
    /// Bascule *tous* les appels suivants sur ce dépôt en
    /// `RepoError::Interne` (voir `provoquer_erreur_interne`) — permet de
    /// tester la branche 500 de chaque contrôleur sans dépendre d'une
    /// vraie panne PostgreSQL.
    provoque_erreur: AtomicBool,
}

impl FakeUtilisateurRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provoquer_erreur_interne(&self, valeur: bool) {
        self.provoque_erreur.store(valeur, Ordering::SeqCst);
    }

    fn verifier_erreur_provoquee(&self) -> Result<(), RepoError> {
        if self.provoque_erreur.load(Ordering::SeqCst) {
            Err(RepoError::Interne("erreur simulée pour les tests".into()))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl UtilisateurRepositoryPort for FakeUtilisateurRepository {
    async fn creer(
        &self,
        pseudo: &str,
        courriel: &str,
        mot_de_passe_hache: &str,
    ) -> Result<Utilisateur, RepoError> {
        self.verifier_erreur_provoquee()?;

        let mut utilisateurs = self.utilisateurs.write().await;

        let deja_pris = utilisateurs
            .values()
            .any(|u| u.pseudo == pseudo || u.courriel.eq_ignore_ascii_case(courriel));
        if deja_pris {
            return Err(RepoError::Conflit);
        }

        let maintenant = Utc::now();
        let utilisateur = Utilisateur {
            id: Uuid::new_v4(),
            pseudo: pseudo.to_string(),
            courriel: courriel.to_string(),
            mot_de_passe_hache: mot_de_passe_hache.to_string(),
            version_auth: 0,
            role: Role::Utilisateur.as_str().to_string(),
            cree_le: maintenant,
            modifie_le: maintenant,
        };
        utilisateurs.insert(utilisateur.id, utilisateur.clone());
        Ok(utilisateur)
    }

    async fn trouver_par_id(&self, id: Uuid) -> Result<Option<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;
        Ok(self.utilisateurs.read().await.get(&id).cloned())
    }

    async fn trouver_par_identifiant(
        &self,
        identifiant: &str,
    ) -> Result<Option<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;
        Ok(self
            .utilisateurs
            .read()
            .await
            .values()
            // Même règle que PgUtilisateurRepository::trouver_par_identifiant :
            // `courriel` est comparé insensible à la casse (la colonne est
            // toujours stockée en minuscules par AuthService), `pseudo`
            // reste sensible à la casse.
            .find(|u| u.pseudo == identifiant || u.courriel.eq_ignore_ascii_case(identifiant))
            .cloned())
    }

    async fn lister(&self) -> Result<Vec<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let mut utilisateurs: Vec<Utilisateur> =
            self.utilisateurs.read().await.values().cloned().collect();
        utilisateurs.sort_by_key(|u| std::cmp::Reverse(u.cree_le));
        Ok(utilisateurs)
    }

    async fn modifier(
        &self,
        id: Uuid,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;

        let mut utilisateurs = self.utilisateurs.write().await;

        let deja_pris = utilisateurs.values().any(|u| {
            u.id != id
                && ((pseudo.is_some() && Some(u.pseudo.as_str()) == pseudo)
                    || courriel.is_some_and(|courriel| u.courriel.eq_ignore_ascii_case(courriel)))
        });
        if deja_pris {
            return Err(RepoError::Conflit);
        }

        Ok(utilisateurs.get_mut(&id).map(|u| {
            if let Some(pseudo) = pseudo {
                u.pseudo = pseudo.to_string();
            }
            if let Some(courriel) = courriel {
                u.courriel = courriel.to_string();
            }
            if let Some(hache) = mot_de_passe_hache {
                u.mot_de_passe_hache = hache.to_string();
                u.version_auth += 1;
            }
            u.modifie_le = Utc::now();
            u.clone()
        }))
    }

    async fn supprimer(&self, id: Uuid) -> Result<bool, RepoError> {
        self.verifier_erreur_provoquee()?;
        Ok(self.utilisateurs.write().await.remove(&id).is_some())
    }

    async fn modifier_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;

        let mut utilisateurs = self.utilisateurs.write().await;
        let deja_pris = utilisateurs.values().any(|u| {
            u.id != id
                && ((pseudo.is_some() && Some(u.pseudo.as_str()) == pseudo)
                    || courriel.is_some_and(|courriel| u.courriel.eq_ignore_ascii_case(courriel)))
        });
        if deja_pris {
            return Err(RepoError::Conflit);
        }

        Ok(utilisateurs.get_mut(&id).and_then(|u| {
            (u.mot_de_passe_hache == mot_de_passe_hache_actuel).then(|| {
                if let Some(pseudo) = pseudo {
                    u.pseudo = pseudo.to_string();
                }
                if let Some(courriel) = courriel {
                    u.courriel = courriel.to_string();
                }
                if let Some(hache) = mot_de_passe_hache {
                    u.mot_de_passe_hache = hache.to_string();
                    u.version_auth += 1;
                }
                u.modifie_le = Utc::now();
                u.clone()
            })
        }))
    }

    async fn supprimer_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
    ) -> Result<bool, RepoError> {
        self.verifier_erreur_provoquee()?;

        let mut utilisateurs = self.utilisateurs.write().await;
        let Some(utilisateur) = utilisateurs.get(&id) else {
            return Ok(false);
        };
        if utilisateur.mot_de_passe_hache != mot_de_passe_hache_actuel {
            return Ok(false);
        }
        utilisateurs.remove(&id);
        Ok(true)
    }

    async fn changer_role(&self, id: Uuid, role: Role) -> Result<Option<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let mut utilisateurs = self.utilisateurs.write().await;
        Ok(utilisateurs.get_mut(&id).map(|u| {
            u.role = role.as_str().to_string();
            u.modifie_le = Utc::now();
            u.clone()
        }))
    }

    async fn verifier_sante(&self) -> Result<(), RepoError> {
        self.verifier_erreur_provoquee()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creer_puis_retrouver_par_pseudo_ou_courriel() {
        let repo = FakeUtilisateurRepository::new();
        let utilisateur = repo
            .creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();

        assert_eq!(
            repo.trouver_par_identifiant("alice")
                .await
                .unwrap()
                .unwrap()
                .id,
            utilisateur.id
        );
        assert_eq!(
            repo.trouver_par_identifiant("alice@example.com")
                .await
                .unwrap()
                .unwrap()
                .id,
            utilisateur.id
        );
    }

    #[tokio::test]
    async fn trouver_par_identifiant_ignore_la_casse_du_courriel_mais_pas_du_pseudo() {
        let repo = FakeUtilisateurRepository::new();
        let utilisateur = repo
            .creer("Alice", "alice@example.com", "hash")
            .await
            .unwrap();

        assert_eq!(
            repo.trouver_par_identifiant("ALICE@EXAMPLE.COM")
                .await
                .unwrap()
                .unwrap()
                .id,
            utilisateur.id
        );
        // Le pseudo, lui, reste sensible à la casse.
        assert!(repo
            .trouver_par_identifiant("alice")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn refuse_un_pseudo_deja_pris() {
        let repo = FakeUtilisateurRepository::new();
        repo.creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();

        let resultat = repo.creer("alice", "autre@example.com", "hash").await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));
    }

    #[tokio::test]
    async fn creer_refuse_un_courriel_deja_pris_meme_a_la_casse_pres() {
        let repo = FakeUtilisateurRepository::new();
        repo.creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();

        let resultat = repo.creer("bob", "ALICE@example.com", "hash").await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));
    }

    #[tokio::test]
    async fn erreur_provoquee_touche_toutes_les_methodes() {
        let repo = FakeUtilisateurRepository::new();
        repo.provoquer_erreur_interne(true);

        assert!(matches!(
            repo.creer("x", "x@example.com", "hash").await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.trouver_par_id(Uuid::new_v4()).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.trouver_par_identifiant("x").await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(repo.lister().await, Err(RepoError::Interne(_))));
        assert!(matches!(
            repo.modifier(Uuid::new_v4(), Some("x"), None, None).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.modifier_si_mot_de_passe_inchange(Uuid::new_v4(), "hash", Some("x"), None, None)
                .await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.supprimer(Uuid::new_v4()).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.supprimer_si_mot_de_passe_inchange(Uuid::new_v4(), "hash")
                .await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.changer_role(Uuid::new_v4(), Role::Admin).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.verifier_sante().await,
            Err(RepoError::Interne(_))
        ));

        repo.provoquer_erreur_interne(false);
        assert!(repo.lister().await.is_ok());
        assert!(repo.verifier_sante().await.is_ok());
    }

    #[tokio::test]
    async fn supprimer_un_utilisateur_inexistant_retourne_false() {
        let repo = FakeUtilisateurRepository::new();
        assert!(!repo.supprimer(Uuid::new_v4()).await.unwrap());
    }

    #[tokio::test]
    async fn modifier_refuse_un_pseudo_deja_pris_par_un_autre_compte() {
        let repo = FakeUtilisateurRepository::new();
        repo.creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();
        let bob = repo.creer("bob", "bob@example.com", "hash").await.unwrap();

        let resultat = repo.modifier(bob.id, Some("alice"), None, None).await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));
    }

    #[tokio::test]
    async fn modifier_refuse_un_courriel_deja_pris_meme_a_la_casse_pres() {
        let repo = FakeUtilisateurRepository::new();
        repo.creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();
        let bob = repo.creer("bob", "bob@example.com", "hash").await.unwrap();

        let resultat = repo
            .modifier(bob.id, None, Some("ALICE@example.com"), None)
            .await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));
    }

    #[tokio::test]
    async fn modifier_un_utilisateur_inexistant_retourne_none() {
        let repo = FakeUtilisateurRepository::new();
        let resultat = repo
            .modifier(Uuid::new_v4(), Some("x"), None, None)
            .await
            .unwrap();
        assert!(resultat.is_none());
    }

    #[tokio::test]
    async fn operations_conditionnelles_refusent_un_hash_perime() {
        let repo = FakeUtilisateurRepository::new();
        let utilisateur = repo
            .creer("alice", "alice@example.com", "hash-actuel")
            .await
            .unwrap();
        repo.creer("bob", "bob@example.com", "hash-bob")
            .await
            .unwrap();

        assert!(repo
            .modifier_si_mot_de_passe_inchange(
                utilisateur.id,
                "hash-perime",
                Some("alice2"),
                None,
                None,
            )
            .await
            .unwrap()
            .is_none());
        assert!(matches!(
            repo.modifier_si_mot_de_passe_inchange(
                utilisateur.id,
                "hash-actuel",
                Some("bob"),
                None,
                None,
            )
            .await,
            Err(RepoError::Conflit)
        ));
        assert_eq!(
            repo.modifier_si_mot_de_passe_inchange(
                utilisateur.id,
                "hash-actuel",
                Some("alice2"),
                Some("alice2@example.com"),
                Some("hash-nouveau"),
            )
            .await
            .unwrap()
            .unwrap()
            .pseudo,
            "alice2"
        );
        assert!(!repo
            .supprimer_si_mot_de_passe_inchange(utilisateur.id, "hash-actuel")
            .await
            .unwrap());
        assert!(repo
            .supprimer_si_mot_de_passe_inchange(utilisateur.id, "hash-nouveau")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn lister_trie_du_plus_recent_au_plus_ancien() {
        let repo = FakeUtilisateurRepository::new();
        let premier = repo
            .creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();
        // Écart garanti entre les deux horodatages : évite un ordre de tri
        // non déterministe si `Utc::now()` retournait la même valeur pour
        // les deux créations (résolution d'horloge trop grossière).
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = repo.creer("bob", "bob@example.com", "hash").await.unwrap();

        let tous = repo.lister().await.unwrap();
        assert_eq!(tous.first().unwrap().id, second.id);
        assert_eq!(tous.last().unwrap().id, premier.id);
    }
}
