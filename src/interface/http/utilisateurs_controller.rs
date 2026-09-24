//! Contrôleurs HTTP pour `/api/users/*`. La majorité de ces routes sont
//! réservées au panneau d'administration (voir énoncé : "Voir la liste des
//! utilisateurs inscrits", "Supprimer un utilisateur") — chaque handler
//! admin vérifie `exiger_admin` avant de faire quoi que ce soit.
//!
//! Exceptions : `GET /api/users/lookup` (voir `rechercher` plus bas) et
//! `GET /api/users/search` (voir `rechercher_par_pseudo`), qui n'exigent
//! qu'une authentification valide (`UtilisateurAuthentifie`), pas le rôle
//! admin — résolution `{id -> pseudo}` et recherche par début de pseudo,
//! volontairement ouvertes à tout utilisateur connecté (voir docs/API.md).

use axum::{extract::State, http::StatusCode, Json as ReponseJson};
use serde::Deserialize;
use uuid::Uuid;

use crate::application::dto::{ChangerRoleRequest, InscriptionRequest, ModifierUtilisateurRequest};
use crate::domain::ports::ResultatChangementRole;
use crate::domain::utilisateur::{UtilisateurLookupPublic, UtilisateurPublic};
use crate::error::AppError;
use crate::state::AppState;

use super::extracteurs::{Json, Path, Query};
use super::middleware::{
    AdministrateurAuthentifie, AdministrateurEtAutreUtilisateur,
    AdministrateurEtUtilisateurNonAdmin, UtilisateurAuthentifie,
};

/// Nombre maximal d'ids acceptés par `GET /api/users/lookup` en une seule
/// requête. Ni `Room-Service` ni `Message-Service` n'ont besoin de
/// résoudre plus de comptes que ça en un appel (une liste de membres de
/// salon ou un lot de messages récents) — au-delà, la requête est refusée
/// avec `400` plutôt que d'exécuter une requête SQL `= ANY(...)`
/// arbitrairement volumineuse pour un client (authentifié, mais pas
/// forcément de confiance) qui enverrait une liste d'ids abusive.
const LOOKUP_MAX_IDS: usize = 100;

#[derive(Debug, Deserialize)]
pub struct RechercherQuery {
    pub ids: Option<String>,
}

/// Longueur maximale du texte de `GET /api/users/search` : celle d'un
/// pseudo (voir `application::dto`), au-delà rien ne peut correspondre.
const RECHERCHE_LONGUEUR_MAX: usize = 20;
const RECHERCHE_LIMITE_DEFAUT: i64 = 20;
const RECHERCHE_LIMITE_MAX: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct RechercherParPseudoQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
}

/// GET /api/users/search?q=texte&limit=n — comptes dont le pseudo commence
/// par `q` (insensible à la casse), `{id, pseudo}` seulement. Comme
/// `/lookup`, ouverte à tout utilisateur authentifié : c'est ce qui permet
/// au Frontend de trouver quelqu'un à qui écrire en message privé sans
/// annuaire complet des comptes (voir docs/API.md). L'appelant n'est pas
/// exclu des résultats : c'est au client de le retirer s'il le souhaite.
pub async fn rechercher_par_pseudo(
    State(state): State<AppState>,
    _utilisateur: UtilisateurAuthentifie,
    Query(requete): Query<RechercherParPseudoQuery>,
) -> Result<ReponseJson<Vec<UtilisateurLookupPublic>>, AppError> {
    let texte = requete.q.as_deref().unwrap_or("").trim();
    if texte.is_empty() {
        return Err(AppError::Requete(
            "le paramètre 'q' est requis et ne peut pas être vide".into(),
        ));
    }
    if texte.chars().count() > RECHERCHE_LONGUEUR_MAX {
        return Err(AppError::Requete(format!(
            "le paramètre 'q' accepte au plus {RECHERCHE_LONGUEUR_MAX} caractères"
        )));
    }
    let limite = requete.limit.unwrap_or(RECHERCHE_LIMITE_DEFAUT);
    if !(1..=RECHERCHE_LIMITE_MAX).contains(&limite) {
        return Err(AppError::Requete(format!(
            "le paramètre 'limit' doit être compris entre 1 et {RECHERCHE_LIMITE_MAX}"
        )));
    }

    let utilisateurs = state
        .depot_utilisateurs
        .rechercher_par_pseudo(texte, limite)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    Ok(ReponseJson(utilisateurs))
}

/// GET /api/users/lookup?ids=id1,id2,... — résolution en lot
/// `{id -> pseudo}`, accessible à tout utilisateur authentifié (voir
/// docs/API.md). Un id qui ne correspond à aucun compte est simplement
/// omis du tableau retourné (pas une erreur, pas de 404 partiel) : c'est
/// au Frontend de traiter une entrée absente comme "utilisateur inconnu".
pub async fn rechercher(
    State(state): State<AppState>,
    _utilisateur: UtilisateurAuthentifie,
    Query(requete): Query<RechercherQuery>,
) -> Result<ReponseJson<Vec<UtilisateurLookupPublic>>, AppError> {
    let ids = parser_ids(requete.ids.as_deref())?;

    let utilisateurs = state
        .depot_utilisateurs
        .rechercher_par_ids(&ids)
        .await
        .map_err(|e| AppError::Interne(e.to_string()))?;

    Ok(ReponseJson(utilisateurs))
}

/// Valide et parse le paramètre `ids` brut de `GET /api/users/lookup` :
/// liste non vide, séparée par des virgules, d'au plus `LOOKUP_MAX_IDS`
/// UUID valides. Choix assumé (voir docs/API.md) : un seul id malformé
/// (pas un UUID) rejette la requête entière avec `400`, plutôt que de
/// l'ignorer silencieusement — cohérent avec le reste de ce service, où un
/// id d'URL malformé (`interface::http::extracteurs::Path`) est aussi
/// rejeté plutôt que traité comme "introuvable".
fn parser_ids(ids_brut: Option<&str>) -> Result<Vec<Uuid>, AppError> {
    let ids_brut = ids_brut.unwrap_or("").trim();
    if ids_brut.is_empty() {
        return Err(AppError::Requete(
            "le paramètre 'ids' est requis et ne peut pas être vide".into(),
        ));
    }

    let segments: Vec<&str> = ids_brut.split(',').map(str::trim).collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(AppError::Requete(
            "le paramètre 'ids' contient une valeur vide (virgule superflue ?)".into(),
        ));
    }
    if segments.len() > LOOKUP_MAX_IDS {
        return Err(AppError::Requete(format!(
            "le paramètre 'ids' accepte au plus {LOOKUP_MAX_IDS} identifiants"
        )));
    }

    segments
        .into_iter()
        .map(|s| {
            Uuid::parse_str(s).map_err(|_| {
                AppError::Requete(format!(
                    "'{s}' dans le paramètre 'ids' n'est pas un UUID valide"
                ))
            })
        })
        .collect()
}

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
) -> Result<(StatusCode, ReponseJson<UtilisateurPublic>), AppError> {
    let utilisateur = state.auth_service.creer_utilisateur(requete).await?;
    tracing::info!(
        admin = %session.pseudo,
        cible = %utilisateur.id,
        "compte créé par un administrateur"
    );
    Ok((StatusCode::CREATED, ReponseJson(utilisateur)))
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
        state.notificateur_suppression.compte_supprime(id);
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
