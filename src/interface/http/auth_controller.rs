//! Contrôleurs HTTP pour `/api/auth/*`. Chaque handler reste court :
//! il extrait le JSON, délègue à `AuthService`, et laisse `AppError`
//! (voir error.rs) gérer la mise en forme des erreurs.

use axum::{extract::State, http::StatusCode, Json as ReponseJson};

use crate::application::dto::{
    AuthResponse, ConnexionRequest, InscriptionRequest, ModifierProfilRequest,
    SupprimerCompteRequest,
};
use crate::domain::utilisateur::UtilisateurPublic;
use crate::error::AppError;
use crate::state::AppState;

use super::extracteurs::Json;
use super::middleware::UtilisateurAuthentifie;

pub async fn refresh(
    State(state): State<AppState>,
    Json(requete): Json<crate::application::dto::RefreshRequest>,
) -> Result<ReponseJson<AuthResponse>, AppError> {
    Ok(ReponseJson(state.auth_service.renouveler(requete).await?))
}

pub async fn logout(
    State(state): State<AppState>,
    Json(requete): Json<crate::application::dto::RefreshRequest>,
) -> Result<StatusCode, AppError> {
    state.auth_service.deconnecter(requete).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(requete): Json<InscriptionRequest>,
) -> Result<(StatusCode, ReponseJson<AuthResponse>), AppError> {
    let reponse = state.auth_service.inscrire(requete).await?;
    Ok((StatusCode::CREATED, ReponseJson(reponse)))
}

/// POST /api/auth/login
pub async fn login(
    State(state): State<AppState>,
    Json(requete): Json<ConnexionRequest>,
) -> Result<ReponseJson<AuthResponse>, AppError> {
    let reponse = state.auth_service.connecter(requete).await?;
    Ok(ReponseJson(reponse))
}

/// GET /api/auth/me — retourne l'utilisateur courant à partir du jeton.
/// Utile pour le desktop : au démarrage, valider un jeton déjà stocké
/// localement sans redemander le mot de passe.
pub async fn me(session: UtilisateurAuthentifie) -> ReponseJson<UtilisateurPublic> {
    ReponseJson(session.utilisateur)
}

/// PATCH /api/auth/me — modifie son propre profil (pseudo, courriel
/// et/ou mot de passe).
pub async fn modifier_moi(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Json(requete): Json<ModifierProfilRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    let utilisateur = state
        .auth_service
        .modifier_profil(session.id_utilisateur, requete)
        .await?;
    Ok(ReponseJson(utilisateur))
}

/// DELETE /api/auth/me — supprime son propre compte.
pub async fn supprimer_moi(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Json(requete): Json<SupprimerCompteRequest>,
) -> Result<StatusCode, AppError> {
    state
        .auth_service
        .supprimer_compte(session.id_utilisateur, requete)
        .await?;
    state
        .notificateur_suppression
        .compte_supprime(session.id_utilisateur);
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/auth/logout-all — invalide tous les jetons de l'utilisateur,
/// sur tous ses appareils (celui de la requête compris). Les autres
/// services le constatent au plus tard après leur cache de validation
/// (`AUTH_CACHE_TTL_MS`).
pub async fn deconnexion_partout(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
) -> Result<StatusCode, AppError> {
    let invalide = state
        .depot_utilisateurs
        .invalider_sessions(session.id_utilisateur)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;
    if invalide {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Supprimé entre la validation du jeton et cet appel.
        Err(AppError::NonAutorise("jeton invalide ou expiré".into()))
    }
}
