//! Contrôleurs HTTP pour `/api/users/*` — réservés au panneau
//! d'administration (voir énoncé : "Voir la liste des utilisateurs
//! inscrits", "Supprimer un utilisateur"). Chaque handler vérifie
//! `exiger_admin` avant de faire quoi que ce soit.

use axum::{extract::State, http::StatusCode, Json as ReponseJson};
use uuid::Uuid;

use crate::application::dto::{ChangerRoleRequest, InscriptionRequest, ModifierUtilisateurRequest};
use crate::domain::ports::ResultatChangementRole;
use crate::domain::utilisateur::UtilisateurPublic;
use crate::error::AppError;
use crate::state::AppState;

use super::extracteurs::{Json, Path};
use super::middleware::{
    AdministrateurAuthentifie, AdministrateurEtAutreUtilisateur,
    AdministrateurEtUtilisateurNonAdmin,
};

/// GET /api/users — liste au plus 100 comptes (admin uniquement).
pub async fn lister(
    State(state): State<AppState>,
    AdministrateurAuthentifie(_session): AdministrateurAuthentifie,
) -> Result<ReponseJson<Vec<UtilisateurPublic>>, AppError> {
    let utilisateurs = state
        .depot_utilisateurs
        .lister(100, 0)
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
    AdministrateurAuthentifie(session): AdministrateurAuthentifie,
    Json(requete): Json<InscriptionRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
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
    AdministrateurAuthentifie(_session): AdministrateurAuthentifie,
    Path(id): Path<Uuid>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
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
    AdministrateurEtAutreUtilisateur {
        session,
        id_utilisateur_cible: id,
    }: AdministrateurEtAutreUtilisateur,
    Json(requete): Json<ModifierUtilisateurRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    let utilisateur = state.auth_service.modifier_utilisateur(id, requete).await?;
    tracing::info!(admin = %session.pseudo, cible = %id, "compte modifié par un administrateur");
    Ok(ReponseJson(utilisateur))
}

/// DELETE /api/users/:id — supprime un compte (admin uniquement).
pub async fn supprimer(
    State(state): State<AppState>,
    AdministrateurEtAutreUtilisateur {
        session,
        id_utilisateur_cible: id,
    }: AdministrateurEtAutreUtilisateur,
) -> Result<StatusCode, AppError> {
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
    AdministrateurEtUtilisateurNonAdmin {
        session,
        id_utilisateur_cible: id,
    }: AdministrateurEtUtilisateurNonAdmin,
    Json(requete): Json<ChangerRoleRequest>,
) -> Result<ReponseJson<UtilisateurPublic>, AppError> {
    let utilisateur = match state
        .depot_utilisateurs
        .changer_role_si_non_admin(id, requete.role)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?
    {
        ResultatChangementRole::Modifie(utilisateur) => utilisateur,
        ResultatChangementRole::CibleAdmin => {
            return Err(AppError::Interdit(
                "le rôle d'un administrateur ne peut pas être modifié".into(),
            ));
        }
        ResultatChangementRole::Introuvable => {
            return Err(AppError::Introuvable("utilisateur introuvable".into()));
        }
    };

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
