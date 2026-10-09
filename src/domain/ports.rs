//! "Port" au sens de l'architecture de l'équipe (voir Architecture.md du
//! frontend : "ports/ — Interfaces nécessaires au domaine/application.
//! Elles décrivent CE DONT le métier a besoin, sans définir COMMENT
//! c'est implémenté. Exemple : MessageRepository, Gateway,
//! EventPublisher.").
//!
//! `UtilisateurRepositoryPort` est ce contrat. `PgUtilisateurRepository`
//! (voir `infrastructure/pg_utilisateur_repository.rs`) en est l'unique
//! implémentation de production, avec PostgreSQL comme source de vérité
//! (aucun mode mock/en mémoire — voir README). Ni `AuthService` ni les
//! contrôleurs HTTP ne dépendent directement de PostgreSQL : ils ne
//! connaissent que ce trait (voir `main.rs`, où `PgUtilisateurRepository`
//! est injecté au démarrage). Les tests d'intégration (`tests/`)
//! fournissent leur propre implémentation en mémoire de ce trait,
//! utilisée uniquement en compilation de test.

use async_trait::async_trait;
use uuid::Uuid;

use super::utilisateur::{Role, Utilisateur, UtilisateurLookupPublic};

#[async_trait]
pub trait UtilisateurRepositoryPort: Send + Sync {
    async fn creer_session(
        &self,
        id: Uuid,
        version: i32,
        famille: Uuid,
        hache: &str,
    ) -> Result<(), RepoError>;
    /// Consomme l'ancien hash une seule fois et insère son successeur dans la même transaction.
    async fn renouveler_session(
        &self,
        ancien: &str,
        nouveau: &str,
    ) -> Result<Option<(Utilisateur, Uuid)>, RepoError>;
    /// Un ancien token consommé permet encore de révoquer sa famille (logout concurrent au refresh).
    async fn revoquer_session(&self, hache: &str) -> Result<(), RepoError>;
    async fn session_active(&self, famille: Uuid, id: Uuid) -> Result<bool, RepoError>;
    async fn creer(
        &self,
        pseudo: &str,
        courriel: &str,
        mot_de_passe_hache: &str,
    ) -> Result<Utilisateur, RepoError>;

    async fn trouver_par_id(&self, id: Uuid) -> Result<Option<Utilisateur>, RepoError>;

    /// Recherche par courriel OU par pseudo (le login accepte les deux).
    async fn trouver_par_identifiant(
        &self,
        identifiant: &str,
    ) -> Result<Option<Utilisateur>, RepoError>;

    async fn lister(&self, limite: i64, decalage: i64) -> Result<Vec<Utilisateur>, RepoError>;

    /// Retourne le sous-ensemble `{id, pseudo}` des comptes qui existent
    /// parmi `ids` (voir `GET /api/users/lookup`, docs/API.md). Un id sans
    /// compte correspondant est silencieusement omis du résultat — ce
    /// n'est ni une erreur ni un `RepoError`, l'appelant HTTP traite une
    /// entrée manquante comme "utilisateur inconnu". `ids` est déjà
    /// validé, non vide et borné en taille par l'appelant (voir
    /// `interface::http::utilisateurs_controller::rechercher`).
    async fn rechercher_par_ids(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError>;

    /// Retourne au plus `limite` comptes `{id, pseudo}` dont le pseudo
    /// commence par `prefixe`, sans tenir compte de la casse, triés par
    /// pseudo (voir `GET /api/users/search`, docs/API.md). `prefixe` est
    /// déjà nettoyé, non vide et borné par l'appelant (voir
    /// `interface::http::utilisateurs_controller::rechercher_par_pseudo`) ;
    /// ses caractères sont pris LITTÉRALEMENT (`%`/`_` ne sont pas des
    /// jokers).
    async fn rechercher_par_pseudo(
        &self,
        prefixe: &str,
        limite: i64,
    ) -> Result<Vec<UtilisateurLookupPublic>, RepoError>;

    /// Modifie un ou plusieurs champs d'un compte existant. Chaque
    /// paramètre `None` laisse la valeur actuelle inchangée (équivalent
    /// d'un `COALESCE` en SQL) — permet de réutiliser cette seule méthode
    /// pour `PATCH /api/auth/me` (pseudo/courriel/mot de passe) et
    /// `PATCH /api/users/{id}` (pseudo/courriel seulement, voir
    /// `AuthService`). Retourne `None` si `id` n'existe pas.
    async fn modifier(
        &self,
        id: Uuid,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError>;

    /// Variante de `modifier` qui n'applique le changement que si le hash
    /// vérifié juste avant est toujours celui stocké. Évite qu'un changement
    /// concurrent de mot de passe contourne la confirmation du mot de passe
    /// actuel entre sa vérification et l'écriture.
    async fn modifier_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
        pseudo: Option<&str>,
        courriel: Option<&str>,
        mot_de_passe_hache: Option<&str>,
    ) -> Result<Option<Utilisateur>, RepoError>;

    async fn supprimer(&self, id: Uuid) -> Result<bool, RepoError>;

    /// Incrémente `version_auth` : tous les jetons déjà émis pour ce compte
    /// deviennent invalides (« se déconnecter partout »). Faux si le compte
    /// n'existe pas.
    async fn invalider_sessions(&self, id: Uuid) -> Result<bool, RepoError>;

    /// Variante de `supprimer` protégée contre la même course que
    /// `modifier_si_mot_de_passe_inchange`.
    async fn supprimer_si_mot_de_passe_inchange(
        &self,
        id: Uuid,
        mot_de_passe_hache_actuel: &str,
    ) -> Result<bool, RepoError>;

    /// Change le rôle uniquement si le compte cible n'est pas déjà admin.
    /// Cette condition doit être atomique pour empêcher une promotion
    /// concurrente de contourner la règle d'administration.
    async fn changer_role_si_non_admin(
        &self,
        id: Uuid,
        role: Role,
    ) -> Result<ResultatChangementRole, RepoError>;

    /// Confirme que le stockage sous-jacent répond réellement (ex. `SELECT
    /// 1` pour `PgUtilisateurRepository`) — utilisé par `GET /health`
    /// (voir `interface::http::routes`), lui-même lu par le `HEALTHCHECK`
    /// du `Dockerfile` et le `depends_on: condition: service_healthy`
    /// d'`infra`. Sans cette vérification, `/health` ne ferait que
    /// confirmer que le processus HTTP tourne, pas que sa dépendance
    /// critique (la base de données) est joignable.
    async fn verifier_sante(&self) -> Result<(), RepoError>;
}

/// Résultat explicite d'une tentative de changement de rôle. Sépare une
/// cible absente d'une cible admin, sans faire deviner la raison au
/// contrôleur à partir d'un `Option`.
/// Prévient les autres microservices qu'un compte a été supprimé, pour
/// qu'ils effacent les données qui le concernent (serveurs possédés,
/// messages, configuration). « Tire et oublie » : la suppression du compte
/// ne dépend jamais de leur disponibilité.
///
/// `sessions_invalidees` : tous les jetons du compte viennent d'être
/// invalidés (mot de passe changé, déconnexion de tous ou des autres
/// appareils). Message-Service ferme aussitôt les sockets temps réel dont le
/// jeton ne vaut plus rien : les autres appareils sont déconnectés sans
/// attendre sa revérification périodique.
pub trait NotificateurSuppressionPort: Send + Sync {
    fn compte_supprime(&self, id: Uuid);
    fn sessions_invalidees(&self, id: Uuid);
}

pub enum ResultatChangementRole {
    Modifie(Utilisateur),
    Inchange(Utilisateur),
    CibleAdmin,
    Introuvable,
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
