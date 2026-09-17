//! Type d'erreur unique pour toute l'application.
//!
//! En Rust, il n'y a pas d'exceptions : une fonction qui peut échouer
//! retourne `Result<T, E>`. `AppError` est notre `E` commun, et
//! `IntoResponse` (plus bas) dit à axum comment le transformer en
//! réponse HTTP (code de statut + JSON), une seule fois pour toute
//! l'appli plutôt que dans chaque contrôleur.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

use crate::application::dto::ErreurResponse;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("non autorisé: {0}")]
    NonAutorise(String),

    #[error("accès interdit: {0}")]
    Interdit(String),

    #[error("ressource introuvable: {0}")]
    Introuvable(String),

    #[error("conflit: {0}")]
    Conflit(String),

    #[error("requête invalide: {0}")]
    Validation(validator::ValidationErrors),

    /// Requête syntaxiquement valide (chaque champ fourni respecte ses
    /// contraintes) mais qui viole une règle métier ad hoc, ex. "au moins
    /// un champ doit être fourni" sur un PATCH partiel — pas exprimable
    /// avec un validateur de champ individuel de `validator`.
    #[error("requête invalide: {0}")]
    Requete(String),

    #[error("erreur interne: {0}")]
    Interne(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (statut, message) = match &self {
            AppError::NonAutorise(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            AppError::Interdit(m) => (StatusCode::FORBIDDEN, m.clone()),
            AppError::Introuvable(m) => (StatusCode::NOT_FOUND, m.clone()),
            AppError::Conflit(m) => (StatusCode::CONFLICT, m.clone()),
            AppError::Validation(erreurs) => (StatusCode::BAD_REQUEST, erreurs.to_string()),
            AppError::Requete(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Interne(m) => {
                // On journalise le détail réel côté serveur, mais on ne
                // le renvoie JAMAIS au client (pourrait exposer des
                // détails d'implémentation ou de sécurité).
                tracing::error!(erreur = %m, "erreur interne");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "une erreur interne est survenue".to_string(),
                )
            }
        };

        (statut, Json(ErreurResponse { erreur: message })).into_response()
    }
}
