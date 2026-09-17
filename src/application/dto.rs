//! DTO (Data Transfer Objects) : la forme exacte du JSON échangé avec le
//! frontend. On les sépare volontairement des entités du domaine
//! (`Utilisateur`) : le JSON public ne doit jamais exposer
//! `mot_de_passe_hache`, et peut évoluer indépendamment du schéma de la
//! base de données.

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::domain::utilisateur::{Role, UtilisateurPublic};

/// Longueur maximale d'un courriel (RFC 5321, §4.5.3.1.3 : 254
/// caractères) — sans plafond, `validator::email` ne limite pas non plus
/// la longueur, et un client pourrait envoyer un courriel de plusieurs Mo
/// (toujours syntaxiquement "valide" pour la regex) juste pour gonfler la
/// charge de travail du serveur (validation, hachage, écriture en base).
const LONGUEUR_MAX_COURRIEL: u64 = 254;

/// Longueur maximale d'un mot de passe. Aucune limite raisonnable
/// n'empêche un utilisateur légitime de choisir un mot de passe très
/// long, mais sans plafond, un client peut envoyer des mots de passe
/// arbitrairement volumineux qui seront tous hachés avec Argon2
/// (délibérément coûteux en CPU) avant même la vérification — un vecteur
/// de déni de service facile à fermer. 128 caractères reste très
/// généreux pour un usage légitime (bien au-delà d'une phrase de passe).
const LONGUEUR_MAX_MOT_DE_PASSE: u64 = 128;

#[derive(Debug, Deserialize, Validate)]
pub struct InscriptionRequest {
    #[validate(
        length(
            min = 3,
            max = 20,
            message = "le pseudo doit contenir entre 3 et 20 caractères"
        ),
        does_not_contain(
            pattern = "@",
            message = "le pseudo ne peut pas contenir '@' (réservé aux courriels, pour éviter toute confusion à la connexion)"
        )
    )]
    pub pseudo: String,
    #[validate(
        email(message = "adresse courriel invalide"),
        length(max = "LONGUEUR_MAX_COURRIEL", message = "courriel trop long")
    )]
    pub courriel: String,
    #[validate(length(
        min = 8,
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "le mot de passe doit contenir entre 8 et 128 caractères"
    ))]
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

/// `PATCH /api/auth/me` : modification de son propre compte. Tous les
/// champs sont optionnels (au moins un requis, vérifié dans
/// `AuthService::modifier_profil`, pas exprimable avec `validator` seul).
/// Note : `validator` valide correctement `Option<String>` (un `None`
/// est toujours considéré valide, seul un `Some` est vérifié) grâce aux
/// implémentations `ValidateLength`/`ValidateEmail`/`ValidateContains`
/// pour `Option<T>`.
#[derive(Debug, Deserialize, Validate)]
pub struct ModifierProfilRequest {
    #[validate(
        length(
            min = 3,
            max = 20,
            message = "le pseudo doit contenir entre 3 et 20 caractères"
        ),
        does_not_contain(
            pattern = "@",
            message = "le pseudo ne peut pas contenir '@' (réservé aux courriels, pour éviter toute confusion à la connexion)"
        )
    )]
    pub pseudo: Option<String>,
    #[validate(
        email(message = "adresse courriel invalide"),
        length(max = "LONGUEUR_MAX_COURRIEL", message = "courriel trop long")
    )]
    pub courriel: Option<String>,
    #[validate(length(
        min = 8,
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "le mot de passe doit contenir entre 8 et 128 caractères"
    ))]
    pub mot_de_passe: Option<String>,
}

/// `PATCH /api/users/{id}` (admin) : modification du pseudo/courriel
/// d'un autre compte, sans mot de passe (voir docs/API.md — changer le
/// mot de passe de quelqu'un d'autre n'est pas une fonctionnalité
/// exposée par ce contrôle).
#[derive(Debug, Deserialize, Validate)]
pub struct ModifierUtilisateurRequest {
    #[validate(
        length(
            min = 3,
            max = 20,
            message = "le pseudo doit contenir entre 3 et 20 caractères"
        ),
        does_not_contain(
            pattern = "@",
            message = "le pseudo ne peut pas contenir '@' (réservé aux courriels, pour éviter toute confusion à la connexion)"
        )
    )]
    pub pseudo: Option<String>,
    #[validate(
        email(message = "adresse courriel invalide"),
        length(max = "LONGUEUR_MAX_COURRIEL", message = "courriel trop long")
    )]
    pub courriel: Option<String>,
}

/// Forme standard des erreurs renvoyées par l'API (voir docs/API.md).
#[derive(Debug, Serialize)]
pub struct ErreurResponse {
    pub erreur: String,
}
