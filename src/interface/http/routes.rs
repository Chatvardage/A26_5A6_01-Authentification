//! Déclaration de toutes les routes HTTP exposées par le service.
//! Regroupées ici : quelqu'un qui veut savoir "quelles routes existent"
//! n'a qu'un seul fichier à lire (voir aussi docs/API.md pour le détail
//! des requêtes/réponses).

use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

use super::{auth_controller, utilisateurs_controller};

pub fn construire(state: AppState) -> Router {
    let api_auth = Router::new()
        .route("/register", post(auth_controller::register))
        .route("/login", post(auth_controller::login))
        .route("/me", get(auth_controller::me));

    let api_users = Router::new()
        .route("/", get(utilisateurs_controller::lister))
        .route("/:id", delete(utilisateurs_controller::supprimer))
        .route("/:id/role", patch(utilisateurs_controller::changer_role));

    // CORS permissif pour le développement (l'app desktop Electron et
    // le panneau d'administration web appellent l'API depuis des
    // origines différentes). À restreindre en production si nécessaire.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/health", get(sante))
        .nest("/api/auth", api_auth)
        .nest("/api/users", api_users)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

/// GET /health — utilisé par le `HEALTHCHECK` du Dockerfile et par
/// docker-compose pour savoir si le service est prêt (voir consigne :
/// "Vérifier la santé de la base de données et de Redis").
async fn sante() -> &'static str {
    "ok"
}
