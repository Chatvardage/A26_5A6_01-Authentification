//! Extracteur axum qui lit le header `Authorization: Bearer <jeton>`,
//! valide le JWT, et rend l'utilisateur courant disponible directement
//! comme paramètre d'un handler (`utilisateur: UtilisateurAuthentifie`).
//!
//! C'est l'équivalent Rust/axum d'un middleware "requireAuth" express,
//! mais typé : un handler qui déclare ce paramètre ne PEUT PAS être
//! appelé sans authentification valide — le compilateur l'impose.

use async_trait::async_trait;
use axum::{extract::FromRequestParts, http::request::Parts};
use axum_extra::headers::{authorization::Bearer, Authorization};
use axum_extra::TypedHeader;

use crate::domain::utilisateur::{Role, UtilisateurPublic};
use crate::error::AppError;
use crate::state::AppState;

use super::extracteurs::Path;

pub struct UtilisateurAuthentifie {
    pub id_utilisateur: uuid::Uuid,
    pub pseudo: String,
    pub role: Role,
    pub utilisateur: UtilisateurPublic,
}

/// Extracteur réservé aux routes d'administration. La vérification du rôle
/// intervient pendant l'extraction des parties de requête, avant `Path` et
/// surtout avant tout extracteur du corps comme `Json`.
pub struct AdministrateurAuthentifie(pub UtilisateurAuthentifie);

/// Extracteur des routes d'administration qui ciblent un autre compte.
/// Il refuse un auto-ciblage avant l'extraction d'un corps JSON.
pub struct AdministrateurEtAutreUtilisateur {
    pub session: UtilisateurAuthentifie,
    pub id_utilisateur_cible: uuid::Uuid,
}

// Dans axum 0.7, le trait `FromRequestParts` est lui-même défini avec la
// macro `#[async_trait]` (pas encore l'"async fn in trait" natif de Rust,
// qui n'arrivera dans axum qu'à partir de la version 0.8). Le compilateur
// exige que l'implémentation utilise EXACTEMENT la même technique — sans
// ce `#[async_trait]` ici, on obtient une erreur de compilation du genre
// "lifetime parameters or bounds ... do not match the trait declaration",
// même si le code a l'air identique. On réutilise la crate `async-trait`
// déjà présente dans Cargo.toml (elle sert aussi pour `UtilisateurRepositoryPort`).
#[async_trait]
impl FromRequestParts<AppState> for UtilisateurAuthentifie {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(porteur)) =
            TypedHeader::<Authorization<Bearer>>::from_request_parts(parts, state)
                .await
                .map_err(|_| {
                    AppError::NonAutorise("en-tête Authorization manquant ou invalide".into())
                })?;

        let claims = state.jwt_service.valider_token(porteur.token())?;

        let id_utilisateur = uuid::Uuid::parse_str(&claims.sub)
            .map_err(|_| AppError::NonAutorise("jeton invalide".into()))?;

        let utilisateur = state
            .depot_utilisateurs
            .trouver_par_id(id_utilisateur)
            .await
            .map_err(|e| AppError::Interne(e.to_string()))?
            .ok_or_else(|| {
                AppError::NonAutorise("jeton révoqué ou utilisateur introuvable".into())
            })?;
        if claims.version_auth != utilisateur.version_auth {
            return Err(AppError::NonAutorise("jeton révoqué".into()));
        }
        let role = utilisateur.role();
        let utilisateur = UtilisateurPublic::from(utilisateur);

        Ok(UtilisateurAuthentifie {
            id_utilisateur,
            pseudo: utilisateur.pseudo.clone(),
            role,
            utilisateur,
        })
    }
}

#[async_trait]
impl FromRequestParts<AppState> for AdministrateurAuthentifie {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let utilisateur = UtilisateurAuthentifie::from_request_parts(parts, state).await?;
        exiger_admin(utilisateur.role)?;
        Ok(Self(utilisateur))
    }
}

#[async_trait]
impl FromRequestParts<AppState> for AdministrateurEtAutreUtilisateur {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AdministrateurAuthentifie(session) =
            AdministrateurAuthentifie::from_request_parts(parts, state).await?;
        let Path(id_utilisateur_cible) =
            Path::<uuid::Uuid>::from_request_parts(parts, state).await?;

        if session.id_utilisateur == id_utilisateur_cible {
            return Err(AppError::Interdit(
                "un administrateur ne peut pas modifier, changer le rôle ou supprimer son propre compte via /api/users".into(),
            ));
        }

        Ok(Self {
            session,
            id_utilisateur_cible,
        })
    }
}

/// À utiliser en plus de `UtilisateurAuthentifie` dans les routes admin
/// (voir `utilisateurs_controller.rs`) : rejette la requête si le rôle n'est
/// pas `admin`, avant même d'exécuter la logique du handler.
pub fn exiger_admin(role: Role) -> Result<(), AppError> {
    if role == Role::Admin {
        Ok(())
    } else {
        Err(AppError::Interdit(
            "cette action est réservée aux administrateurs".into(),
        ))
    }
}
