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

/// POST /api/auth/register
pub async fn register(
    State(state): State<AppState>,
    Json(requete): Json<InscriptionRequest>,
) -> Result<ReponseJson<AuthResponse>, AppError> {
    let reponse = state.auth_service.inscrire(requete).await?;
    Ok(ReponseJson(reponse))
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
pub async fn me(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    let utilisateur = state
        .depot_utilisateurs
        .trouver_par_id(session.id_utilisateur)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?
        .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

    Ok(ReponseJson(UtilisateurPublic::from(utilisateur)))
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
    Ok(StatusCode::NO_CONTENT)
}
