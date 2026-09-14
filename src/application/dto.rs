//! DTO (Data Transfer Objects) : la forme exacte du JSON échangé avec le
//! frontend. On les sépare volontairement des entités du domaine
//! (`Utilisateur`) : le JSON public ne doit jamais exposer
//! `mot_de_passe_hache`, et peut évoluer indépendamment du schéma de la
//! base de données.

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::domain::utilisateur::{Role, UtilisateurPublic};

#[derive(Debug, Deserialize, Validate)]
pub struct InscriptionRequest {
    #[validate(length(min = 3, max = 20, message = "le pseudo doit contenir entre 3 et 20 caractères"))]
    pub pseudo: String,
    #[validate(email(message = "adresse courriel invalide"))]
    pub courriel: String,
    #[validate(length(min = 8, message = "le mot de passe doit contenir au moins 8 caractères"))]
    pub mot_de_passe: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ConnexionRequest {
    /// Accepte un courriel OU un pseudo, comme demandé dans l'énoncé.
    #[validate(length(min = 1, message = "identifiant requis"))]
    pub identifiant: String,
    #[validate(length(min = 1, message = "mot de passe requis"))]
    pub mot_de_passe: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub jeton: String,
    pub utilisateur: UtilisateurPublic,
}

#[derive(Debug, Deserialize)]
pub struct ChangerRoleRequest {
    pub role: Role,
}

/// Forme standard des erreurs renvoyées par l'API (voir docs/API.md).
#[derive(Debug, Serialize)]
pub struct ErreurResponse {
    pub erreur: String,
}
