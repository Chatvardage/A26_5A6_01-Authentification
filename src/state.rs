//! `AppState` : tout ce dont les contrôleurs HTTP ont besoin, regroupé
//! dans une seule structure partagée entre toutes les requêtes (axum la
//! clone à chaque requête, mais grâce à `Arc`/`Clone` bon marché sur nos
//! services, ce n'est pas coûteux).

use std::sync::Arc;

use crate::application::auth_service::AuthService;
use crate::domain::ports::{NotificateurSuppressionPort, UtilisateurRepositoryPort};
use crate::infrastructure::jwt::JwtService;

#[derive(Clone)]
pub struct AppState {
    pub auth_service: AuthService,
    /// `dyn UtilisateurRepositoryPort` : `main.rs` y injecte
    /// `PgUtilisateurRepository` au démarrage — les contrôleurs ne
    /// dépendent que du trait, jamais de PostgreSQL directement.
    pub depot_utilisateurs: Arc<dyn UtilisateurRepositoryPort>,
    pub jwt_service: Arc<JwtService>,
    /// Prévient les autres services d'une suppression de compte (voir
    /// `infrastructure::notificateur_suppression`).
    pub notificateur_suppression: Arc<dyn NotificateurSuppressionPort>,
}
