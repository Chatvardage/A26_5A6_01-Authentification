//! Cas d'utilisation ("use cases") liés à l'authentification :
//! inscription et connexion. C'est ici que la logique applicative vit —
//! les contrôleurs HTTP (couche `interface`) restent volontairement
//! "minces" : ils décodent le JSON, appellent ce service, et formatent
//! la réponse.
//!
//! `AuthService` ne dépend que du port `UtilisateurRepositoryPort` (voir
//! `domain::ports`), jamais de PostgreSQL directement : selon
//! `USE_MOCK_DB`, `main.rs` lui injecte soit `PgUtilisateurRepository`,
//! soit `MockUtilisateurRepository`, sans que ce fichier n'ait besoin de
//! le savoir.

use std::sync::Arc;

use uuid::Uuid;
use validator::Validate;

use crate::domain::ports::{RepoError, UtilisateurRepositoryPort};
use crate::domain::utilisateur::UtilisateurPublic;
use crate::error::AppError;
use crate::infrastructure::jwt::JwtService;
use crate::infrastructure::mot_de_passe::{hacher_mot_de_passe, verifier_mot_de_passe};

use super::dto::{
    AuthResponse, ConnexionRequest, InscriptionRequest, ModifierProfilRequest,
    ModifierUtilisateurRequest,
};

#[derive(Clone)]
pub struct AuthService {
    depot: Arc<dyn UtilisateurRepositoryPort>,
    jwt: Arc<JwtService>,
}

impl AuthService {
    pub fn new(depot: Arc<dyn UtilisateurRepositoryPort>, jwt: Arc<JwtService>) -> Self {
        Self { depot, jwt }
    }

    pub async fn inscrire(&self, requete: InscriptionRequest) -> Result<AuthResponse, AppError> {
        // On ne fait confiance à AUCUNE donnée venant du client sans la
        // valider d'abord (voir `validator` sur les DTO).
        requete.validate().map_err(AppError::Validation)?;

        let hache = hacher_mot_de_passe(&requete.mot_de_passe)?;

        let utilisateur = self
            .depot
            .creer(&requete.pseudo, &requete.courriel, &hache)
            .await
            .map_err(mapper_erreur_depot)?;

        let jeton =
            self.jwt
                .generer_token(utilisateur.id, &utilisateur.pseudo, utilisateur.role())?;
        Ok(AuthResponse {
            jeton,
            utilisateur: UtilisateurPublic::from(utilisateur),
        })
    }

    pub async fn connecter(&self, requete: ConnexionRequest) -> Result<AuthResponse, AppError> {
        requete.validate().map_err(AppError::Validation)?;

        let utilisateur = self
            .depot
            .trouver_par_identifiant(&requete.identifiant)
            .await
            .map_err(|e| AppError::Interne(e.to_string()))?
            // Message volontairement générique : on ne révèle jamais si
            // c'est l'identifiant ou le mot de passe qui est incorrect
            // (bonne pratique de sécurité, évite l'énumération de comptes).
            .ok_or_else(|| AppError::NonAutorise("identifiant ou mot de passe invalide".into()))?;

        if !verifier_mot_de_passe(&requete.mot_de_passe, &utilisateur.mot_de_passe_hache) {
            return Err(AppError::NonAutorise(
                "identifiant ou mot de passe invalide".into(),
            ));
        }

        let jeton =
            self.jwt
                .generer_token(utilisateur.id, &utilisateur.pseudo, utilisateur.role())?;
        Ok(AuthResponse {
            jeton,
            utilisateur: UtilisateurPublic::from(utilisateur),
        })
    }

    /// `PATCH /api/auth/me` : modifie son propre profil (pseudo, courriel
    /// et/ou mot de passe — tous optionnels, au moins un requis).
    pub async fn modifier_profil(
        &self,
        id: Uuid,
        requete: ModifierProfilRequest,
    ) -> Result<UtilisateurPublic, AppError> {
        requete.validate().map_err(AppError::Validation)?;
        if requete.pseudo.is_none() && requete.courriel.is_none() && requete.mot_de_passe.is_none()
        {
            return Err(AppError::Requete(
                "au moins un champ doit être fourni".into(),
            ));
        }

        let hache_mot_de_passe = requete
            .mot_de_passe
            .as_deref()
            .map(hacher_mot_de_passe)
            .transpose()?;

        let utilisateur = self
            .depot
            .modifier(
                id,
                requete.pseudo.as_deref(),
                requete.courriel.as_deref(),
                hache_mot_de_passe.as_deref(),
            )
            .await
            .map_err(mapper_erreur_depot)?
            .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

        Ok(UtilisateurPublic::from(utilisateur))
    }

    /// `DELETE /api/auth/me` : supprime son propre compte.
    pub async fn supprimer_compte(&self, id: Uuid) -> Result<(), AppError> {
        let supprime = self
            .depot
            .supprimer(id)
            .await
            .map_err(|e| AppError::Interne(e.to_string()))?;

        if supprime {
            Ok(())
        } else {
            Err(AppError::Introuvable("utilisateur introuvable".into()))
        }
    }

    /// `PATCH /api/users/{id}` (admin) : modifie le pseudo et/ou le
    /// courriel d'un autre compte — jamais le mot de passe ni le rôle
    /// (voir `changer_role` dans `utilisateurs_controller.rs` pour le
    /// rôle).
    pub async fn modifier_utilisateur(
        &self,
        id: Uuid,
        requete: ModifierUtilisateurRequest,
    ) -> Result<UtilisateurPublic, AppError> {
        requete.validate().map_err(AppError::Validation)?;
        if requete.pseudo.is_none() && requete.courriel.is_none() {
            return Err(AppError::Requete(
                "au moins un champ doit être fourni".into(),
            ));
        }

        let utilisateur = self
            .depot
            .modifier(
                id,
                requete.pseudo.as_deref(),
                requete.courriel.as_deref(),
                None,
            )
            .await
            .map_err(mapper_erreur_depot)?
            .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

        Ok(UtilisateurPublic::from(utilisateur))
    }
}

/// Traduit les erreurs de stockage (ex. pseudo/courriel déjà pris) en
/// erreur applicative compréhensible pour le client HTTP.
fn mapper_erreur_depot(e: RepoError) -> AppError {
    match e {
        RepoError::Conflit => AppError::Conflit("ce pseudo ou ce courriel est déjà utilisé".into()),
        RepoError::Interne(msg) => AppError::Interne(msg),
    }
}
