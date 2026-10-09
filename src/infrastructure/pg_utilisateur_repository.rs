//! Implémentation PostgreSQL de `UtilisateurRepositoryPort` (voir
//! `domain::ports`). Regroupe tout le SQL lié aux utilisateurs : si un
//! jour on change de base de données, seul ce fichier change.
//!
//! Ce service ne crée pas directement la table `utilisateurs` au runtime :
//! les migrations Prisma versionnées dans ce dépôt sont appliquées séparément,
//! notamment par `npx prisma migrate deploy` au démarrage du conteneur. Le
//! schéma attendu par les requêtes ci-dessous est documenté dans le README.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::ports::{RepoError, ResultatChangementRole, UtilisateurRepositoryPort};
use crate::domain::utilisateur::{Role, Utilisateur, UtilisateurLookupPublic};

#[derive(Clone)]
pub struct PgUtilisateurRepository {
    pool: PgPool,
}

impl PgUtilisateurRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Échappe les caractères spéciaux de `LIKE` (`\\`, `%`, `_`) pour qu'ils
/// soient comparés littéralement (clause `ESCAPE '\\'`).
fn echapper_like(texte: &str) -> String {
    let mut resultat = String::with_capacity(texte.len());
    for caractere in texte.chars() {
        if matches!(caractere, '\\' | '%' | '_') {
            resultat.push('\\');
        }
        resultat.push(caractere);
    }
    resultat
}

#[async_trait]
impl UtilisateurRepositoryPort for PgUtilisateurRepository {
    async fn creer_session(
        &self,
        id: Uuid,
        version: i32,
        famille: Uuid,
        hache: &str,
    ) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(mapper_erreur_sql)?;
        let current: Option<i32> =
            sqlx::query_scalar("SELECT version_auth FROM utilisateurs WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
        if current != Some(version) {
            return Err(RepoError::Interne(
                "compte modifié pendant la connexion".into(),
            ));
        }
        sqlx::query("INSERT INTO sessions_refresh (hache, famille, utilisateur_id, version_auth) VALUES ($1,$2,$3,$4)")
            .bind(hache).bind(famille).bind(id).bind(version).execute(&mut *tx).await.map_err(mapper_erreur_sql)?;
        tx.commit().await.map_err(mapper_erreur_sql)
    }

    async fn renouveler_session(
        &self,
        ancien: &str,
        nouveau: &str,
    ) -> Result<Option<(Utilisateur, Uuid)>, RepoError> {
        let mut tx = self.pool.begin().await.map_err(mapper_erreur_sql)?;
        let token: Option<(Uuid, Uuid)> =
            sqlx::query_as("SELECT utilisateur_id, famille FROM sessions_refresh WHERE hache = $1")
                .bind(ancien)
                .fetch_optional(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
        let Some((id, famille)) = token else {
            return Ok(None);
        };
        // Same user row lock for refresh, logout and password/version changes.
        let user =
            sqlx::query_as::<_, Utilisateur>("SELECT * FROM utilisateurs WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
        let Some(user) = user else {
            return Ok(None);
        };
        let expiration: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "UPDATE sessions_refresh SET consomme = true WHERE hache = $1 AND NOT consomme AND NOT revoque AND expire_le > clock_timestamp() AND version_auth = $2 RETURNING expire_le")
            .bind(ancien).bind(user.version_auth).fetch_optional(&mut *tx).await.map_err(mapper_erreur_sql)?;
        let Some(expiration) = expiration else {
            return Ok(None);
        };
        sqlx::query("INSERT INTO sessions_refresh (hache, famille, utilisateur_id, version_auth, expire_le) VALUES ($1,$2,$3,$4,$5)")
            .bind(nouveau).bind(famille).bind(id).bind(user.version_auth).bind(expiration)
            .execute(&mut *tx).await.map_err(mapper_erreur_sql)?;
        tx.commit().await.map_err(mapper_erreur_sql)?;
        Ok(Some((user, famille)))
    }

    async fn revoquer_session(&self, hache: &str) -> Result<(), RepoError> {
        let mut tx = self.pool.begin().await.map_err(mapper_erreur_sql)?;
        let token: Option<(Uuid, Uuid)> =
            sqlx::query_as("SELECT utilisateur_id, famille FROM sessions_refresh WHERE hache = $1")
                .bind(hache)
                .fetch_optional(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
        if let Some((id, famille)) = token {
            sqlx::query("SELECT id FROM utilisateurs WHERE id = $1 FOR UPDATE")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
            sqlx::query("UPDATE sessions_refresh SET revoque = true WHERE famille = $1")
                .bind(famille)
                .execute(&mut *tx)
                .await
                .map_err(mapper_erreur_sql)?;
        }
        tx.commit().await.map_err(mapper_erreur_sql)
    }

    async fn session_active(&self, famille: Uuid, id: Uuid) -> Result<bool, RepoError> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions_refresh WHERE famille = $1 AND utilisateur_id = $2 AND NOT revoque AND NOT consomme AND expire_le > now())")
            .bind(famille).bind(id).fetch_one(&self.pool).await.map_err(mapper_erreur_sql)
    }
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
            RETURNING id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le
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
            "SELECT id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le \
             FROM utilisateurs WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn trouver_par_identifiant(
        &self,
        identifiant: &str,
    ) -> Result<Option<Utilisateur>, RepoError> {
        // `LOWER(courriel) = LOWER($1)` rend la recherche de courriel
        // insensible à la casse tout en permettant à PostgreSQL d'utiliser
        // son index fonctionnel sur `LOWER(courriel)`. `pseudo`, lui, reste
        // sensible à la casse (comparaison exacte), volontairement : rien
        // dans l'énoncé n'exige l'insensibilité à la casse pour un pseudo.
        sqlx::query_as::<_, Utilisateur>(
            "SELECT id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le \
             FROM utilisateurs WHERE LOWER(courriel) = LOWER($1) OR pseudo = $1",
        )
        .bind(identifiant)
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn lister(&self, limite: i64, decalage: i64) -> Result<Vec<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            "SELECT id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le \
             FROM utilisateurs ORDER BY cree_le DESC LIMIT $1 OFFSET $2",
        )
        .bind(limite)
        .bind(decalage)
        .fetch_all(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn rechercher_par_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError> {
        // `= ANY($1)` avec un tableau lié plutôt qu'un `IN (...)` construit
        // dynamiquement : un seul paramètre lié (pas d'injection SQL
        // possible via la taille de la liste) et sqlx sait encoder
        // `&[Uuid]` comme un tableau PostgreSQL natif. Ne sélectionne que
        // `id`/`pseudo` (pas `SELECT *`) : cette route est accessible à
        // tout utilisateur authentifié, `mot_de_passe_hache` ne doit même
        // pas transiter en mémoire ici.
        sqlx::query_as::<_, UtilisateurLookupPublic>(
            "SELECT id, pseudo FROM utilisateurs WHERE id = ANY($1)",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn rechercher_par_pseudo(
        &self,
        prefixe: &str,
        limite: i64,
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError> {
        // `%`, `_` et `\` du texte saisi sont échappés : sans ça, chercher
        // "a_" ou "%" se comporterait comme un joker LIKE. Même restriction
        // aux seules colonnes id/pseudo que `rechercher_par_ids`.
        let motif = format!("{}%", echapper_like(&prefixe.to_lowercase()));
        sqlx::query_as::<_, UtilisateurLookupPublic>(
            "SELECT id, pseudo FROM utilisateurs \
             WHERE LOWER(pseudo) LIKE $1 ESCAPE '\\' \
             ORDER BY LOWER(pseudo), pseudo \
             LIMIT $2",
        )
        .bind(motif)
        .bind(limite)
        .fetch_all(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn modifier(
        &self,
        id: Uuid,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            r#"
            UPDATE utilisateurs
            SET pseudo = COALESCE($2, pseudo),
                courriel = COALESCE($3, courriel),
                mot_de_passe_hache = COALESCE($4, mot_de_passe_hache),
                version_auth = version_auth + CASE WHEN $4 IS NULL THEN 0 ELSE 1 END,
                modifie_le = now()
            WHERE id = $1
            RETURNING id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le
            "#,
        )
        .bind(id)
        .bind(pseudo)
        .bind(courriel)
        .bind(mot_de_passe_hache)
        .fetch_optional(&self.pool)
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

    async fn invalider_sessions(&self, id: Uuid) -> Result<bool, RepoError> {
        let resultat = sqlx::query(
            "UPDATE utilisateurs SET version_auth = version_auth + 1, modifie_le = now() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(mapper_erreur_sql)?;
        Ok(resultat.rows_affected() > 0)
    }

    async fn modifier_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError> {
        sqlx::query_as::<_, Utilisateur>(
            r#"
            UPDATE utilisateurs
            SET pseudo = COALESCE($3, pseudo),
                courriel = COALESCE($4, courriel),
                mot_de_passe_hache = COALESCE($5, mot_de_passe_hache),
                version_auth = version_auth + CASE WHEN $5 IS NULL THEN 0 ELSE 1 END,
                modifie_le = now()
            WHERE id = $1 AND mot_de_passe_hache = $2
            RETURNING id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le
            "#,
        )
        .bind(id)
        .bind(mot_de_passe_hache_actuel)
        .bind(pseudo)
        .bind(courriel)
        .bind(mot_de_passe_hache)
        .fetch_optional(&self.pool)
        .await
        .map_err(mapper_erreur_sql)
    }

    async fn supprimer_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
    ) -> Result<bool, RepoError> {
        let resultat =
            sqlx::query("DELETE FROM utilisateurs WHERE id = $1 AND mot_de_passe_hache = $2")
                .bind(id)
                .bind(mot_de_passe_hache_actuel)
                .execute(&self.pool)
                .await
                .map_err(mapper_erreur_sql)?;
        Ok(resultat.rows_affected() > 0)
    }

    async fn changer_role_si_non_admin(
        &self,
        id: Uuid,
        role: Role,
    ) -> Result<ResultatChangementRole, RepoError> {
        let mut transaction = self.pool.begin().await.map_err(mapper_erreur_sql)?;
        let actuel: Option<Utilisateur> =
            sqlx::query_as("SELECT id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le FROM utilisateurs WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(mapper_erreur_sql)?;

        let Some(actuel) = actuel else {
            return Ok(ResultatChangementRole::Introuvable);
        };
        if actuel.role == Role::Admin.as_str() {
            return Ok(ResultatChangementRole::CibleAdmin);
        }
        if actuel.role == role.as_str() {
            return Ok(ResultatChangementRole::Inchange(actuel));
        }

        let utilisateur = sqlx::query_as::<_, Utilisateur>(
            r#"
            UPDATE utilisateurs
            SET role = $2, version_auth = version_auth + 1, modifie_le = now()
            WHERE id = $1
            RETURNING id, pseudo, courriel, mot_de_passe_hache, version_auth, role, cree_le, modifie_le
            "#,
        )
        .bind(id)
        .bind(role.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(mapper_erreur_sql)?;
        transaction.commit().await.map_err(mapper_erreur_sql)?;
        Ok(ResultatChangementRole::Modifie(utilisateur))
    }

    async fn verifier_sante(&self) -> Result<(), RepoError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(mapper_erreur_sql)?;
        Ok(())
    }
}

/// Traduit une erreur SQL "brute" (ex. contrainte UNIQUE violée) en
/// `RepoError`, le seul type d'erreur que le reste de l'application
/// connaît.
fn mapper_erreur_sql(e: sqlx::Error) -> RepoError {
    // Code "23505" = unique_violation en PostgreSQL.
    let est_conflit_unique =
        matches!(&e, sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505"));
    if est_conflit_unique {
        RepoError::Conflit
    } else {
        RepoError::Interne(e.to_string())
    }
}

/// Tests contre une vraie base PostgreSQL migrée (voir README/AGENTS.md :
/// ce service n'a pas de mode mock — même ses tests suivent cette règle).
/// `DATABASE_URL` doit pointer vers une base migrée (`npx prisma migrate
/// deploy`), ex. via `docker compose up -d postgres` avant `cargo test`
/// — voir README, section "Lancer les tests".
#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db;

    #[tokio::test]
    async fn sessions_rotation_atomique_expiration_revocation_et_version() {
        let pool = pool_de_test().await;
        let repo = PgUtilisateurRepository::new(pool.clone());
        let pseudo = identifiant_unique("session");
        let user = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();
        let famille = Uuid::new_v4();
        let old = Uuid::new_v4().to_string();
        let next = Uuid::new_v4().to_string();
        let other = Uuid::new_v4().to_string();
        repo.creer_session(user.id, user.version_auth, famille, &old)
            .await
            .unwrap();
        assert!(repo.session_active(famille, user.id).await.unwrap());
        let (a, b) = tokio::join!(
            repo.renouveler_session(&old, &next),
            repo.renouveler_session(&old, &other)
        );
        assert_eq!(
            [a.as_ref().unwrap().is_some(), b.as_ref().unwrap().is_some()]
                .iter()
                .filter(|v| **v)
                .count(),
            1
        );
        let current = if a.unwrap().is_some() { &next } else { &other };
        assert!(repo
            .renouveler_session(&old, "replay")
            .await
            .unwrap()
            .is_none());
        repo.revoquer_session(&old).await.unwrap();
        assert!(!repo.session_active(famille, user.id).await.unwrap());
        assert!(repo
            .renouveler_session(current, "revoked")
            .await
            .unwrap()
            .is_none());
        repo.revoquer_session("unknown").await.unwrap();
        assert!(repo
            .renouveler_session("unknown", "none")
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .creer_session(
                user.id,
                user.version_auth + 1,
                Uuid::new_v4(),
                "wrong-version"
            )
            .await
            .is_err());

        let expired = Uuid::new_v4().to_string();
        let expired_family = Uuid::new_v4();
        repo.creer_session(user.id, user.version_auth, expired_family, &expired)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE sessions_refresh SET expire_le = now() - interval '1 second' WHERE hache = $1",
        )
        .bind(&expired)
        .execute(&pool)
        .await
        .unwrap();
        assert!(repo
            .renouveler_session(&expired, "expired-next")
            .await
            .unwrap()
            .is_none());
        assert!(!repo.session_active(expired_family, user.id).await.unwrap());
        let valid = Uuid::new_v4().to_string();
        repo.creer_session(user.id, user.version_auth, Uuid::new_v4(), &valid)
            .await
            .unwrap();
        repo.invalider_sessions(user.id).await.unwrap();
        assert!(repo
            .renouveler_session(&valid, "version-next")
            .await
            .unwrap()
            .is_none());
        repo.supprimer(user.id).await.unwrap();
        assert!(repo
            .renouveler_session(&valid, "deleted-next")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn sessions_erreurs_sql_et_rollback_rotation() {
        let pool = pool_de_test().await;
        let repo = PgUtilisateurRepository::new(pool.clone());
        let pseudo = identifiant_unique("rollback");
        let user = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();
        let old = Uuid::new_v4().to_string();
        repo.creer_session(user.id, user.version_auth, Uuid::new_v4(), &old)
            .await
            .unwrap();
        assert!(repo.renouveler_session(&old, &old).await.is_err());
        assert!(repo
            .renouveler_session(&old, &Uuid::new_v4().to_string())
            .await
            .unwrap()
            .is_some());
        assert!(repo
            .creer_session(user.id, user.version_auth, Uuid::new_v4(), &old)
            .await
            .is_err());
        assert!(repo
            .creer_session(Uuid::new_v4(), 0, Uuid::new_v4(), "absent")
            .await
            .is_err());
        repo.supprimer(user.id).await.unwrap();
        pool.close().await;
        assert!(repo
            .creer_session(user.id, 0, Uuid::new_v4(), "closed")
            .await
            .is_err());
        assert!(repo.renouveler_session("closed", "new").await.is_err());
        assert!(repo.revoquer_session("closed").await.is_err());
        assert!(repo.session_active(Uuid::new_v4(), user.id).await.is_err());
    }

    async fn pool_de_test() -> PgPool {
        let url = {
            let _garde = crate::config::VERROU_ENV_TEST
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::env::var("DATABASE_URL")
        }
        .expect(
            "DATABASE_URL doit être définie pour ces tests (voir README, section \"Lancer les tests\") \
             — ex. `docker compose up -d postgres` puis exporter la même URL que .env.example",
        );
        db::connect(&url).await.expect(
            "connexion à PostgreSQL requise pour ces tests — vérifiez DATABASE_URL et que la migration \
             Prisma a été appliquée",
        )
    }

    /// Pseudo/courriel unique par test (les tests d'un même fichier
    /// s'exécutent en parallèle sur la même table) pour éviter les faux
    /// conflits entre exécutions.
    fn identifiant_unique(prefixe: &str) -> String {
        format!("{prefixe}{}", Uuid::new_v4().simple())
    }

    #[tokio::test]
    async fn creer_puis_retrouver_par_id_pseudo_et_courriel() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("alice");
        let courriel = format!("{pseudo}@example.com");

        let cree = repo.creer(&pseudo, &courriel, "hash").await.unwrap();
        assert_eq!(cree.pseudo, pseudo);
        assert_eq!(cree.role, "utilisateur");

        assert_eq!(
            repo.trouver_par_id(cree.id).await.unwrap().unwrap().id,
            cree.id
        );
        assert_eq!(
            repo.trouver_par_identifiant(&pseudo)
                .await
                .unwrap()
                .unwrap()
                .id,
            cree.id
        );
        assert_eq!(
            repo.trouver_par_identifiant(&courriel)
                .await
                .unwrap()
                .unwrap()
                .id,
            cree.id
        );

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn trouver_par_identifiant_ignore_la_casse_du_courriel_mais_pas_du_pseudo() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("Helga");
        let courriel = format!("{pseudo}@example.com").to_lowercase();
        let cree = repo.creer(&pseudo, &courriel, "hash").await.unwrap();

        let trouve = repo
            .trouver_par_identifiant(&courriel.to_uppercase())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(trouve.id, cree.id);

        // Le pseudo reste sensible à la casse.
        assert!(repo
            .trouver_par_identifiant(&pseudo.to_lowercase())
            .await
            .unwrap()
            .is_none());

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn trouver_par_id_inexistant_retourne_none() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(repo.trouver_par_id(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn trouver_par_identifiant_inexistant_retourne_none() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(repo
            .trouver_par_identifiant("personne-de-ce-nom-nexiste-pas")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn creer_refuse_un_pseudo_deja_pris() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("bob");
        let courriel = format!("{pseudo}@example.com");
        let cree = repo.creer(&pseudo, &courriel, "hash").await.unwrap();

        let resultat = repo
            .creer(&pseudo, &format!("autre-{courriel}"), "hash")
            .await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn creer_refuse_un_courriel_deja_pris_meme_a_la_casse_pres() {
        // Contrairement au test ci-dessus (conflit de pseudo), on appelle
        // ici le dépôt directement avec des casses différentes — sans
        // passer par `AuthService::normaliser_courriel` — pour vérifier
        // que c'est bien l'index PostgreSQL fonctionnel sur
        // `LOWER(courriel)` (voir la migration
        // `index_courriel_insensible_a_la_casse`) qui empêche le doublon,
        // pas seulement la discipline applicative.
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("Carol");
        let courriel = format!("{pseudo}@Example.com");
        let cree = repo.creer(&pseudo, &courriel, "hash").await.unwrap();

        let autre_pseudo = identifiant_unique("carol2");
        let resultat = repo
            .creer(&autre_pseudo, &courriel.to_lowercase(), "hash")
            .await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn lister_contient_les_comptes_crees() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("carla");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();

        let tous = repo.lister(100, 0).await.unwrap();
        assert!(tous.iter().any(|u| u.id == cree.id));

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn rechercher_par_ids_omet_les_ids_inexistants_et_respecte_lordre_de_la_base() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo_a = identifiant_unique("look-a");
        let a = repo
            .creer(&pseudo_a, &format!("{pseudo_a}@example.com"), "hash")
            .await
            .unwrap();
        let pseudo_b = identifiant_unique("look-b");
        let b = repo
            .creer(&pseudo_b, &format!("{pseudo_b}@example.com"), "hash")
            .await
            .unwrap();
        let id_inconnu = Uuid::new_v4();

        let resultat = repo
            .rechercher_par_ids(&[a.id, id_inconnu, b.id])
            .await
            .unwrap();

        assert_eq!(resultat.len(), 2);
        assert!(resultat
            .iter()
            .any(|u| u.id == a.id && u.pseudo == pseudo_a));
        assert!(resultat
            .iter()
            .any(|u| u.id == b.id && u.pseudo == pseudo_b));
        assert!(!resultat.iter().any(|u| u.id == id_inconnu));

        repo.supprimer(a.id).await.unwrap();
        repo.supprimer(b.id).await.unwrap();
    }

    #[tokio::test]
    async fn rechercher_par_ids_liste_vide_retourne_vecteur_vide() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(repo.rechercher_par_ids(&[]).await.unwrap().is_empty());
    }

    #[test]
    fn echapper_like_neutralise_les_jokers() {
        assert_eq!(echapper_like("a_b%c\\d"), "a\\_b\\%c\\\\d");
        assert_eq!(echapper_like("alice"), "alice");
    }

    #[tokio::test]
    async fn rechercher_par_pseudo_prefixe_insensible_a_la_casse_et_litteral() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        // Préfixe unique pour ne pas dépendre des autres comptes de la base.
        let base = identifiant_unique("Srch");
        let pseudo_a = format!("{base}_a");
        let pseudo_b = format!("{base}xb");
        let a = repo
            .creer(&pseudo_a, &format!("{pseudo_a}@example.com"), "hash")
            .await
            .unwrap();
        let b = repo
            .creer(&pseudo_b, &format!("{pseudo_b}@example.com"), "hash")
            .await
            .unwrap();

        let tous = repo
            .rechercher_par_pseudo(&base.to_uppercase(), 50)
            .await
            .unwrap();
        assert_eq!(tous.len(), 2);

        // `_` est littéral : "<base>_" ne doit PAS correspondre à "<base>xb".
        let litteral = repo
            .rechercher_par_pseudo(&format!("{base}_"), 50)
            .await
            .unwrap();
        assert_eq!(litteral.len(), 1);
        assert_eq!(litteral[0].id, a.id);

        let limite = repo.rechercher_par_pseudo(&base, 1).await.unwrap();
        assert_eq!(limite.len(), 1);

        repo.supprimer(a.id).await.unwrap();
        repo.supprimer(b.id).await.unwrap();
    }

    #[tokio::test]
    async fn modifier_met_a_jour_seulement_les_champs_fournis() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("dora");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();

        let nouveau_pseudo = identifiant_unique("dora2");
        let modifie = repo
            .modifier(cree.id, Some(&nouveau_pseudo), None, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(modifie.pseudo, nouveau_pseudo);
        assert_eq!(modifie.courriel, cree.courriel);
        assert!(modifie.modifie_le >= cree.modifie_le);

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn modifier_un_id_inexistant_retourne_none() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let resultat = repo
            .modifier(Uuid::new_v4(), Some("x"), None, None)
            .await
            .unwrap();
        assert!(resultat.is_none());
    }

    #[tokio::test]
    async fn modifier_si_mot_de_passe_inchange_refuse_un_hash_perime() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("guarded-update");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash-actuel")
            .await
            .unwrap();

        assert!(repo
            .modifier_si_mot_de_passe_inchange(
                cree.id,
                "hash-perime",
                Some("pseudo-inchange"),
                None,
                None,
            )
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            repo.trouver_par_id(cree.id).await.unwrap().unwrap().pseudo,
            pseudo
        );

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn supprimer_si_mot_de_passe_inchange_supprime_seulement_avec_le_hash_actuel() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("guarded-delete");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash-actuel")
            .await
            .unwrap();

        assert!(!repo
            .supprimer_si_mot_de_passe_inchange(cree.id, "hash-perime")
            .await
            .unwrap());
        assert!(repo
            .supprimer_si_mot_de_passe_inchange(cree.id, "hash-actuel")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn modifier_refuse_un_pseudo_deja_pris() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo_a = identifiant_unique("eve");
        let a = repo
            .creer(&pseudo_a, &format!("{pseudo_a}@example.com"), "hash")
            .await
            .unwrap();
        let pseudo_b = identifiant_unique("finn");
        let b = repo
            .creer(&pseudo_b, &format!("{pseudo_b}@example.com"), "hash")
            .await
            .unwrap();

        let resultat = repo.modifier(b.id, Some(&pseudo_a), None, None).await;
        assert!(matches!(resultat, Err(RepoError::Conflit)));

        repo.supprimer(a.id).await.unwrap();
        repo.supprimer(b.id).await.unwrap();
    }

    #[tokio::test]
    async fn invalider_sessions_incremente_version_auth() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("henri");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();

        assert!(repo.invalider_sessions(cree.id).await.unwrap());
        let relu = repo.trouver_par_id(cree.id).await.unwrap().unwrap();
        assert_eq!(relu.version_auth, cree.version_auth + 1);

        assert!(!repo.invalider_sessions(Uuid::new_v4()).await.unwrap());
        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn supprimer_un_id_inexistant_retourne_false() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(!repo.supprimer(Uuid::new_v4()).await.unwrap());
    }

    #[tokio::test]
    async fn changer_role_si_non_admin_met_a_jour_le_role() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("gina");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();

        let resultat = repo
            .changer_role_si_non_admin(cree.id, Role::Admin)
            .await
            .unwrap();
        let ResultatChangementRole::Modifie(modifie) = resultat else {
            panic!("le compte non-admin doit pouvoir être promu");
        };
        assert_eq!(modifie.role, "admin");

        assert!(matches!(
            repo.changer_role_si_non_admin(cree.id, Role::Utilisateur)
                .await
                .unwrap(),
            ResultatChangementRole::CibleAdmin
        ));

        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn changer_role_si_non_admin_id_inexistant_retourne_introuvable() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(matches!(
            repo.changer_role_si_non_admin(Uuid::new_v4(), Role::Admin)
                .await
                .unwrap(),
            ResultatChangementRole::Introuvable
        ));
    }

    #[tokio::test]
    async fn role_identique_preserve_la_version_et_la_date() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        let pseudo = identifiant_unique("roleinchange");
        let cree = repo
            .creer(&pseudo, &format!("{pseudo}@example.com"), "hash")
            .await
            .unwrap();
        let resultat = repo
            .changer_role_si_non_admin(cree.id, Role::Utilisateur)
            .await
            .unwrap();
        let ResultatChangementRole::Inchange(inchange) = resultat else {
            panic!("un rôle identique ne doit pas invalider les sessions");
        };
        assert_eq!(inchange.version_auth, cree.version_auth);
        assert_eq!(inchange.modifie_le, cree.modifie_le);
        let relu = repo.trouver_par_id(cree.id).await.unwrap().unwrap();
        assert_eq!(relu.version_auth, cree.version_auth);
        assert_eq!(relu.modifie_le, cree.modifie_le);
        repo.supprimer(cree.id).await.unwrap();
    }

    #[tokio::test]
    async fn erreur_sql_generique_est_mappee_en_interne_pas_en_conflit() {
        let pool = pool_de_test().await;
        let repo = PgUtilisateurRepository::new(pool.clone());
        // Ferme le pool : toute requête suivante échoue immédiatement avec
        // une erreur qui n'est PAS une violation de contrainte unique —
        // vérifie que `mapper_erreur_sql` retombe bien sur
        // `RepoError::Interne` (branche générique), pas `Conflit`.
        pool.close().await;

        let resultat = repo.trouver_par_id(Uuid::new_v4()).await;
        assert!(matches!(resultat, Err(RepoError::Interne(_))));
    }

    #[tokio::test]
    async fn verifier_sante_reussit_avec_une_vraie_connexion() {
        let repo = PgUtilisateurRepository::new(pool_de_test().await);
        assert!(repo.verifier_sante().await.is_ok());
    }

    #[tokio::test]
    async fn verifier_sante_echoue_si_le_pool_est_ferme() {
        let pool = pool_de_test().await;
        let repo = PgUtilisateurRepository::new(pool.clone());
        pool.close().await;

        assert!(matches!(
            repo.verifier_sante().await,
            Err(RepoError::Interne(_))
        ));
    }
}
