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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::domain::ports::{RepoError, ResultatChangementRole, UtilisateurRepositoryPort};
use crate::domain::utilisateur::{Role, Utilisateur, UtilisateurLookupPublic};

type SessionTest = (Uuid, Uuid, i32, bool, bool);

#[derive(Default)]
pub struct FakeUtilisateurRepository {
    sessions: RwLock<HashMap<String, SessionTest>>,
    utilisateurs: RwLock<HashMap<Uuid, Utilisateur>>,
    /// Bascule *tous* les appels suivants sur ce dépôt en
    /// `RepoError::Interne` (voir `provoquer_erreur_interne`) — permet de
    /// tester la branche 500 de chaque contrôleur sans dépendre d'une
    /// vraie panne PostgreSQL.
    provoque_erreur: AtomicBool,
    /// Variante de `provoque_erreur` qui laisse d'abord passer quelques
    /// appels (voir `provoquer_erreur_interne_apres`) : 0 = désactivé,
    /// sinon nombre d'appels encore acceptés + 1.
    appels_avant_erreur: AtomicUsize,
    /// Simule une suppression concurrente entre le chargement de la cible
    /// par l'extracteur et la mutation atomique du rôle.
    supprimer_avant_changement_role: AtomicBool,
    /// Simule une promotion admin concurrente entre le chargement de la
    /// cible par l'extracteur et la mutation atomique du rôle.
    promouvoir_admin_avant_changement_role: AtomicBool,
    /// Simule un changement de mot de passe concurrent entre la
    /// vérification du mot de passe actuel et la suppression du compte.
    changer_mot_de_passe_avant_suppression: AtomicBool,
    /// Simule la même course pendant une modification de profil.
    changer_mot_de_passe_avant_modification: AtomicBool,
}

impl FakeUtilisateurRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provoquer_erreur_interne(&self, valeur: bool) {
        self.provoque_erreur.store(valeur, Ordering::SeqCst);
        if !valeur {
            self.appels_avant_erreur.store(0, Ordering::SeqCst);
        }
    }

    /// Laisse passer `appels` appels, puis bascule tous les suivants en
    /// `RepoError::Interne` — pour atteindre une erreur de stockage située
    /// après l'authentification (qui lit déjà le dépôt).
    pub fn provoquer_erreur_interne_apres(&self, appels: usize) {
        self.appels_avant_erreur.store(appels + 1, Ordering::SeqCst);
    }

    pub fn provoquer_suppression_avant_changement_role(&self) {
        self.supprimer_avant_changement_role
            .store(true, Ordering::SeqCst);
    }

    pub fn provoquer_promotion_admin_avant_changement_role(&self) {
        self.promouvoir_admin_avant_changement_role
            .store(true, Ordering::SeqCst);
    }

    pub fn provoquer_changement_mot_de_passe_avant_suppression(&self) {
        self.changer_mot_de_passe_avant_suppression
            .store(true, Ordering::SeqCst);
    }

    pub fn provoquer_changement_mot_de_passe_avant_modification(&self) {
        self.changer_mot_de_passe_avant_modification
            .store(true, Ordering::SeqCst);
    }

    /// Prépare un scénario de test qui simule une modification directe en
    /// base, hors des routes d'administration normalement protégées.
    pub async fn changer_role_pour_test(&self, id: Uuid, role: Role) {
        let mut utilisateurs = self.utilisateurs.write().await;
        if let Some(utilisateur) = utilisateurs.get_mut(&id) {
            utilisateur.role = role.as_str().to_string();
            utilisateur.version_auth += 1;
            utilisateur.modifie_le = Utc::now();
        }
    }

    fn verifier_erreur_provoquee(&self) -> Result<(), RepoError> {
        let restants = self.appels_avant_erreur.load(Ordering::SeqCst);
        if restants > 1 {
            self.appels_avant_erreur.fetch_sub(1, Ordering::SeqCst);
        }
        if self.provoque_erreur.load(Ordering::SeqCst) || restants == 1 {
            Err(RepoError::Interne("erreur simulée pour les tests".into()))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl UtilisateurRepositoryPort for FakeUtilisateurRepository {
    async fn creer_session(
        &self,
        id: Uuid,
        version: i32,
        famille: Uuid,
        hache: &str,
    ) -> Result<(), RepoError> {
        self.verifier_erreur_provoquee()?;
        self.sessions
            .write()
            .await
            .insert(hache.into(), (id, famille, version, false, false));
        Ok(())
    }
    async fn renouveler_session(
        &self,
        ancien: &str,
        nouveau: &str,
    ) -> Result<Option<(Utilisateur, Uuid)>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let users = self.utilisateurs.read().await;
        let mut sessions = self.sessions.write().await;
        let Some(&(id, famille, version, used, revoked)) = sessions.get(ancien) else {
            return Ok(None);
        };
        let Some(user) = users.get(&id) else {
            return Ok(None);
        };
        if used || revoked || user.version_auth != version {
            return Ok(None);
        }
        sessions.insert(ancien.into(), (id, famille, version, true, revoked));
        sessions.insert(nouveau.into(), (id, famille, version, false, false));
        Ok(Some((user.clone(), famille)))
    }
    async fn revoquer_session(&self, hache: &str) -> Result<(), RepoError> {
        self.verifier_erreur_provoquee()?;
        let mut sessions = self.sessions.write().await;
        if let Some(&(_, famille, _, _, _)) = sessions.get(hache) {
            for value in sessions.values_mut().filter(|s| s.1 == famille) {
                value.4 = true;
            }
        }
        Ok(())
    }
    async fn session_active(&self, famille: Uuid, id: Uuid) -> Result<bool, RepoError> {
        self.verifier_erreur_provoquee()?;
        Ok(self
            .sessions
            .read()
            .await
            .values()
            .any(|s| s.0 == id && s.1 == famille && !s.3 && !s.4))
    }
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

    async fn lister(&self, limite: i64, decalage: i64) -> Result<Vec<Utilisateur>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let mut utilisateurs: Vec<Utilisateur> =
            self.utilisateurs.read().await.values().cloned().collect();
        utilisateurs.sort_by_key(|u| std::cmp::Reverse(u.cree_le));
        Ok(utilisateurs
            .into_iter()
            .skip(decalage as usize)
            .take(limite as usize)
            .collect())
    }

    async fn rechercher_par_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let utilisateurs = self.utilisateurs.read().await;
        Ok(ids
            .iter()
            .filter_map(|id| {
                utilisateurs.get(id).map(|u| UtilisateurLookupPublic {
                    id: u.id,
                    pseudo: u.pseudo.clone(),
                })
            })
            .collect())
    }

    async fn rechercher_par_pseudo(
        &self,
        prefixe: &str,
        limite: i64,
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError> {
        self.verifier_erreur_provoquee()?;
        let prefixe = prefixe.to_lowercase();
        let mut resultat: Vec<UtilisateurLookupPublic> = self
            .utilisateurs
            .read()
            .await
            .values()
            .filter(|u| u.pseudo.to_lowercase().starts_with(&prefixe))
            .map(|u| UtilisateurLookupPublic {
                id: u.id,
                pseudo: u.pseudo.clone(),
            })
            .collect();
        resultat.sort_by(|a, b| {
            a.pseudo
                .to_lowercase()
                .cmp(&b.pseudo.to_lowercase())
                .then_with(|| a.pseudo.cmp(&b.pseudo))
        });
        resultat.truncate(limite.max(0) as usize);
        Ok(resultat)
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
        if !utilisateurs.contains_key(&id) {
            return Ok(None);
        }

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

    async fn invalider_sessions(&self, id: Uuid) -> Result<bool, RepoError> {
        self.verifier_erreur_provoquee()?;
        Ok(match self.utilisateurs.write().await.get_mut(&id) {
            Some(utilisateur) => {
                utilisateur.version_auth += 1;
                true
            }
            None => false,
        })
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
        if self
            .changer_mot_de_passe_avant_modification
            .swap(false, Ordering::SeqCst)
        {
            if let Some(user) = utilisateurs.get_mut(&id) {
                user.mot_de_passe_hache = "hachage-modifie-concurremment".into();
                user.version_auth += 1;
            }
        }
        if !utilisateurs.contains_key(&id) {
            return Ok(None);
        }
        if utilisateurs
            .get(&id)
            .is_some_and(|u| u.mot_de_passe_hache != mot_de_passe_hache_actuel)
        {
            return Ok(None);
        }
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

    async fn supprimer_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
    ) -> Result<bool, RepoError> {
        self.verifier_erreur_provoquee()?;

        let mut utilisateurs = self.utilisateurs.write().await;
        if self
            .changer_mot_de_passe_avant_suppression
            .swap(false, Ordering::SeqCst)
        {
            if let Some(utilisateur) = utilisateurs.get_mut(&id) {
                utilisateur.mot_de_passe_hache = "change-en-parallele".to_string();
            }
        }
        let Some(utilisateur) = utilisateurs.get(&id) else {
            return Ok(false);
        };
        if utilisateur.mot_de_passe_hache != mot_de_passe_hache_actuel {
            return Ok(false);
        }
        utilisateurs.remove(&id);
        Ok(true)
    }

    async fn changer_role_si_non_admin(
        &self,
        id: Uuid,
        role: Role,
    ) -> Result<ResultatChangementRole, RepoError> {
        self.verifier_erreur_provoquee()?;
        let mut utilisateurs = self.utilisateurs.write().await;
        if self
            .supprimer_avant_changement_role
            .swap(false, Ordering::SeqCst)
        {
            utilisateurs.remove(&id);
            return Ok(ResultatChangementRole::Introuvable);
        }
        let Some(utilisateur) = utilisateurs.get_mut(&id) else {
            return Ok(ResultatChangementRole::Introuvable);
        };
        if self
            .promouvoir_admin_avant_changement_role
            .swap(false, Ordering::SeqCst)
        {
            utilisateur.role = Role::Admin.as_str().to_string();
        }
        if utilisateur.role == Role::Admin.as_str() {
            return Ok(ResultatChangementRole::CibleAdmin);
        }
        if utilisateur.role == role.as_str() {
            return Ok(ResultatChangementRole::Inchange(utilisateur.clone()));
        }
        utilisateur.role = role.as_str().to_string();
        utilisateur.version_auth += 1;
        utilisateur.modifie_le = Utc::now();
        Ok(ResultatChangementRole::Modifie(utilisateur.clone()))
    }

    async fn verifier_sante(&self) -> Result<(), RepoError> {
        self.verifier_erreur_provoquee()
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn recherche_departage_la_casse_et_arret_panne_retablit_le_depot() {
        use super::*;
        let repo = FakeUtilisateurRepository::new();
        repo.creer("Alice", "a@example.com", "hash").await.unwrap();
        repo.creer("alice", "b@example.com", "hash").await.unwrap();
        let users = repo.rechercher_par_pseudo("al", 20).await.unwrap();
        assert_eq!(
            users.iter().map(|u| u.pseudo.as_str()).collect::<Vec<_>>(),
            ["Alice", "alice"]
        );
        repo.provoquer_erreur_interne_apres(0);
        assert!(repo.verifier_sante().await.is_err());
        repo.provoquer_erreur_interne(false);
        assert!(repo.verifier_sante().await.is_ok());
        repo.changer_role_pour_test(Uuid::new_v4(), Role::Admin)
            .await;
    }
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
        assert!(matches!(
            repo.lister(100, 0).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.rechercher_par_ids(&[Uuid::new_v4()]).await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.rechercher_par_pseudo("a", 20).await,
            Err(RepoError::Interne(_))
        ));
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
            repo.changer_role_si_non_admin(Uuid::new_v4(), Role::Admin)
                .await,
            Err(RepoError::Interne(_))
        ));
        assert!(matches!(
            repo.verifier_sante().await,
            Err(RepoError::Interne(_))
        ));

        repo.provoquer_erreur_interne(false);
        assert!(repo.lister(100, 0).await.is_ok());
        assert!(repo.verifier_sante().await.is_ok());
    }

    #[tokio::test]
    async fn rechercher_par_ids_omet_les_ids_inexistants() {
        let repo = FakeUtilisateurRepository::new();
        let alice = repo
            .creer("alice", "alice@example.com", "hash")
            .await
            .unwrap();
        let bob = repo.creer("bob", "bob@example.com", "hash").await.unwrap();
        let id_inconnu = Uuid::new_v4();

        let resultat = repo
            .rechercher_par_ids(&[alice.id, id_inconnu, bob.id])
            .await
            .unwrap();

        assert_eq!(resultat.len(), 2);
        assert!(resultat
            .iter()
            .any(|u| u.id == alice.id && u.pseudo == "alice"));
        assert!(resultat.iter().any(|u| u.id == bob.id && u.pseudo == "bob"));
    }

    #[tokio::test]
    async fn rechercher_par_ids_liste_vide_retourne_vecteur_vide() {
        let repo = FakeUtilisateurRepository::new();
        assert!(repo.rechercher_par_ids(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rechercher_par_pseudo_filtre_par_prefixe_trie_et_limite() {
        let repo = FakeUtilisateurRepository::new();
        for pseudo in ["Bob", "alice", "Alicia", "albert", "zoe"] {
            repo.creer(pseudo, &format!("{pseudo}@example.com"), "hash")
                .await
                .unwrap();
        }

        let pseudos = |resultat: Vec<UtilisateurLookupPublic>| {
            resultat.into_iter().map(|u| u.pseudo).collect::<Vec<_>>()
        };
        assert_eq!(
            pseudos(repo.rechercher_par_pseudo("AL", 20).await.unwrap()),
            ["albert", "alice", "Alicia"]
        );
        assert_eq!(
            pseudos(repo.rechercher_par_pseudo("al", 2).await.unwrap()),
            ["albert", "alice"]
        );
        assert!(repo
            .rechercher_par_pseudo("x", 20)
            .await
            .unwrap()
            .is_empty());
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
    async fn modifier_un_mot_de_passe_incremente_la_version_auth() {
        let repo = FakeUtilisateurRepository::new();
        let utilisateur = repo
            .creer("alice", "alice@example.com", "hash-initial")
            .await
            .unwrap();

        let modifie = repo
            .modifier(utilisateur.id, None, None, Some("hash-nouveau"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(modifie.mot_de_passe_hache, "hash-nouveau");
        assert_eq!(modifie.version_auth, utilisateur.version_auth + 1);
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

        let tous = repo.lister(100, 0).await.unwrap();
        assert_eq!(tous.first().unwrap().id, second.id);
        assert_eq!(tous.last().unwrap().id, premier.id);
    }
}
