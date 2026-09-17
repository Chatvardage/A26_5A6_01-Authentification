//! Contrôleurs HTTP pour `/api/users/*` — réservés au panneau
//! d'administration (voir énoncé : "Voir la liste des utilisateurs
//! inscrits", "Supprimer un utilisateur"). Chaque handler vérifie
//! `exiger_admin` avant de faire quoi que ce soit.

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::application::dto::{ChangerRoleRequest, ModifierUtilisateurRequest};
use crate::domain::utilisateur::UtilisateurPublic;
use crate::error::AppError;
use crate::state::AppState;

use super::middleware::{exiger_admin, UtilisateurAuthentifie};

/// GET /api/users — liste tous les comptes (admin uniquement).
pub async fn lister(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
) -> Result<Json<Vec<UtilisateurPublic>>, AppError> {
    exiger_admin(session.role)?;

    let utilisateurs = state
        .depot_utilisateurs
        .lister()
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    Ok(Json(
        utilisateurs
            .into_iter()
            .map(UtilisateurPublic::from)
            .collect(),
    ))
}

/// GET /api/users/:id — retourne un compte précis (admin uniquement).
pub async fn obtenir(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
) -> Result<Json<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;

    let utilisateur = state
        .depot_utilisateurs
        .trouver_par_id(id)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?
        .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

    Ok(Json(UtilisateurPublic::from(utilisateur)))
}

/// PATCH /api/users/:id — modifie le pseudo/courriel d'un autre compte
/// (admin uniquement, sans toucher au mot de passe ni au rôle).
pub async fn modifier(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
    Json(requete): Json<ModifierUtilisateurRequest>,
) -> Result<Json<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;

    let utilisateur = state.auth_service.modifier_utilisateur(id, requete).await?;
    tracing::info!(admin = %session.pseudo, cible = %id, "compte modifié par un administrateur");
    Ok(Json(utilisateur))
}

/// DELETE /api/users/:id — supprime un compte (admin uniquement).
pub async fn supprimer(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    exiger_admin(session.role)?;

    let supprime = state
        .depot_utilisateurs
        .supprimer(id)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    if supprime {
        tracing::info!(admin = %session.pseudo, cible = %id, "compte supprimé par un administrateur");
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::Introuvable("utilisateur introuvable".into()))
    }
}

/// PATCH /api/users/:id/role — change le rôle d'un utilisateur (admin
/// uniquement). Permet de promouvoir un utilisateur en modérateur/admin.
pub async fn changer_role(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
    Json(requete): Json<ChangerRoleRequest>,
) -> Result<Json<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;

    let utilisateur = state
        .depot_utilisateurs
        .changer_role(id, requete.role)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?
        .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

    let utilisateur = UtilisateurPublic::from(utilisateur);
    let nouveau_role = utilisateur.role.as_str();
    tracing::info!(
        admin = %session.pseudo,
        cible = %id,
        nouveau_role,
        "rôle modifié par un administrateur"
    );
    Ok(Json(utilisateur))
}
