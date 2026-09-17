//! Remplacements de `axum::extract::Json`/`axum::extract::Path` qui
//! respectent le contrat documenté dans `docs/API.md` : *toute* erreur
//! renvoyée par ce service a la forme `{ "erreur": "..." }`.
//!
//! Les extracteurs nus d'axum ne le garantissent pas : un JSON
//! syntaxiquement invalide, un champ du mauvais type (ex. `{"role":
//! "superadmin"}`, valeur absente de l'énumération `Role`) ou un segment
//! d'URL qui ne parse pas (ex. un `id` qui n'est pas un UUID) produisent
//! une `JsonRejection`/`PathRejection` qu'axum transforme lui-même en
//! réponse — avec un corps en texte brut, pas notre enveloppe JSON. Ces
//! deux types enveloppent l'extracteur d'axum et reformatent sa
//! rejection via `AppError` (déjà garant de l'enveloppe pour toutes les
//! autres erreurs, voir `error.rs`).
//!
//! Utilisation : remplacer `axum::extract::Json<T>`/`Path<T>` par
//! `super::extracteurs::Json<T>`/`Path<T>` dans la signature des
//! handlers — le reste (désérialisation, contrainte `Deserialize`) est
//! identique.

use async_trait::async_trait;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
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
    AppError::Requete(rejet.body_text())
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
    AppError::Requete(rejet.body_text())
}
