//! Contrôleurs HTTP pour `/api/users/*` — réservés au panneau
//! d'administration (voir énoncé : "Voir la liste des utilisateurs
//! inscrits", "Supprimer un utilisateur"). Chaque handler vérifie
//! `exiger_admin` avant de faire quoi que ce soit.

use axum::{extract::State, http::StatusCode, Json as ReponseJson};
use uuid::Uuid;

use crate::application::dto::{ChangerRoleRequest, InscriptionRequest, ModifierUtilisateurRequest};
use crate::domain::utilisateur::UtilisateurPublic;
use crate::error::AppError;
use crate::state::AppState;

use super::extracteurs::{Json, Path};
use super::middleware::{exiger_admin, UtilisateurAuthentifie};

/// GET /api/users — liste tous les comptes (admin uniquement).
pub async fn lister(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
) -> Result<ReponseJson<Vec<UtilisateurPublic>>, AppError> {
    exiger_admin(session.role)?;

    let utilisateurs = state
        .depot_utilisateurs
        .lister()
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    Ok(ReponseJson(
        utilisateurs
            .into_iter()
            .map(UtilisateurPublic::from)
            .collect(),
    ))
}

/// POST /api/users — crée un compte standard (admin uniquement).
pub async fn creer(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Json(requete): Json<InscriptionRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;

    let utilisateur = state.auth_service.creer_utilisateur(requete).await?;
    tracing::info!(
        admin = %session.pseudo,
        cible = %utilisateur.id,
        "compte créé par un administrateur"
    );
    Ok(ReponseJson(utilisateur))
}

/// GET /api/users/:id — retourne un compte précis (admin uniquement).
pub async fn obtenir(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;

    let utilisateur = state
        .depot_utilisateurs
        .trouver_par_id(id)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?
        .ok_or_else(|| AppError::Introuvable("utilisateur introuvable".into()))?;

    Ok(ReponseJson(UtilisateurPublic::from(utilisateur)))
}

/// PATCH /api/users/:id — modifie le pseudo/courriel d'un autre compte
/// (admin uniquement, sans toucher au mot de passe ni au rôle).
pub async fn modifier(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
    Json(requete): Json<ModifierUtilisateurRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    exiger_admin(session.role)?;
    exiger_un_autre_utilisateur(session.id_utilisateur, id)?;

    let utilisateur = state.auth_service.modifier_utilisateur(id, requete).await?;
    tracing::info!(admin = %session.pseudo, cible = %id, "compte modifié par un administrateur");
    Ok(ReponseJson(utilisateur))
}

/// DELETE /api/users/:id — supprime un compte (admin uniquement).
pub async fn supprimer(
    State(state): State<AppState>,
    session: UtilisateurAuthentifie,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    exiger_admin(session.role)?;
    exiger_un_autre_utilisateur(session.id_utilisateur, id)?;

    let supprime = state
        .depot_utilisateurs
        .supprimer(id)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    if supprime {
        tracing::info!(admin = %session.pseudo, cible = %id, "compte supprimé par un administrateur");
        Ok(StatusCode::NO_CONTENT)
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
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
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
    Ok(ReponseJson(utilisateur))
}

fn exiger_un_autre_utilisateur(
    id_utilisateur_connecte: Uuid,
    id_utilisateur_cible: Uuid,
) -> Result<(), AppError> {
    if id_utilisateur_cible == id_utilisateur_connecte {
        Err(AppError::Interdit(
            "utilisez /api/auth/me pour modifier ou supprimer votre propre compte".into(),
        ))
    } else {
        Ok(())
    }
}
