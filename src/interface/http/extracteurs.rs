//! Remplacements de `axum::extract::Json`/`axum::extract::Path` qui
//! respectent le contrat documenté dans `docs/API.md` : *toute* erreur
//! renvoyée par ce service a la forme `{ "erreur": "..." }`, avec un code
//! de statut parmi ceux effectivement documentés (400/401/403/404/409/500
//! — plus 413 pour un corps trop volumineux, seul ajout, voir plus bas).
//!
//! Les extracteurs nus d'axum ne le garantissent pas : un JSON
//! syntaxiquement invalide, un champ du mauvais type (ex. `{"role":
//! "superadmin"}`, valeur absente de l'énumération `Role`) ou un segment
//! d'URL qui ne parse pas (ex. un `id` qui n'est pas un UUID) produisent
//! une `JsonRejection`/`PathRejection` qu'axum transforme lui-même en
//! réponse — avec un corps en texte brut (pas notre enveloppe JSON) et,
//! selon la sous-catégorie technique interne, un code de statut hors de
//! notre contrat documenté (ex. 422 pour un champ manquant/mal typé, 415
//! pour un `Content-Type` incorrect). Ces deux types enveloppent
//! l'extracteur d'axum, reformatent sa rejection via `AppError` (déjà
//! garant de l'enveloppe pour toutes les autres erreurs, voir `error.rs`)
//! et normalisent son statut vers 400 (voir `mapper_rejet_json`) — sauf
//! pour un corps trop volumineux, gardé en 413, seul cas assez distinct
//! d'un problème de contenu pour mériter son propre code.
//!
//! Utilisation : remplacer `axum::extract::Json<T>`/`Path<T>` par
//! `super::extracteurs::Json<T>`/`Path<T>` dans la signature des
//! handlers — le reste (désérialisation, contrainte `Deserialize`) est
//! identique.

use async_trait::async_trait;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use serde::de::DeserializeOwned;

use crate::error::AppError;

pub struct Json<T>(pub T);

#[async_trait]
impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(valeur)) => Ok(Json(valeur)),
            Err(rejet) => Err(mapper_rejet_json(rejet)),
        }
    }
}

fn mapper_rejet_json(rejet: JsonRejection) -> AppError {
    // Le statut natif d'axum varie selon la sous-catégorie technique :
    // JSON syntaxiquement invalide -> 400, mais type incorrect/champ
    // manquant (`JsonDataError`) -> 422, `Content-Type` manquant/incorrect
    // -> 415. Le contrat documenté dans docs/API.md ne connaît qu'un seul
    // panier "requête invalide" (400) pour tous ces cas — on normalise
    // donc vers 400, à une exception près : un corps trop volumineux
    // (voir `DefaultBodyLimit` dans `routes.rs`) reste 413, un cas
    // suffisamment distinct (limite de taille, pas un problème de
    // contenu) pour mériter son propre code plutôt que d'être noyé dans
    // le même panier.
    let statut = match rejet.status() {
        StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
        _ => StatusCode::BAD_REQUEST,
    };
    AppError::Extraction(statut, rejet.body_text())
}

pub struct Path<T>(pub T);

#[async_trait]
impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(valeur)) => Ok(Path(valeur)),
            Err(rejet) => Err(mapper_rejet_path(rejet)),
        }
    }
}

fn mapper_rejet_path(rejet: PathRejection) -> AppError {
    AppError::Extraction(rejet.status(), rejet.body_text())
}
