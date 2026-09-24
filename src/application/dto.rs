//! DTO (Data Transfer Objects) : la forme exacte du JSON échangé avec le
//! frontend. On les sépare volontairement des entités du domaine
//! (`Utilisateur`) : le JSON public ne doit jamais exposer
//! `mot_de_passe_hache`, et peut évoluer indépendamment du schéma de la
//! base de données.

use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

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

/// Valide le jeu de caractères d'un pseudo : lettres/chiffres ASCII,
/// `_` et `-` uniquement. Liste blanche plutôt que liste noire (ex. le
/// `does_not_contain("@")` d'une version précédente, désormais rendu
/// redondant par cette règle plus stricte) : un pseudo est affiché tel
/// quel dans le panneau d'administration de l'équipe (AdminJS/Dashboard,
/// voir AGENTS.md) — sans cette liste blanche, rien n'empêcherait des
/// caractères de contrôle ou du HTML/JS (`<script>...`) d'y transiter, un
/// vecteur de XSS stockée si ce panneau ne les échappe pas correctement.
fn valider_pseudo(pseudo: &str) -> Result<(), ValidationError> {
    let valide = pseudo
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valide {
        Ok(())
    } else {
        Err(ValidationError::new("pseudo_charset"))
    }
}

#[derive(Debug, Deserialize, Validate)]
pub struct InscriptionRequest {
    #[validate(
        length(
            min = 3,
            max = 20,
            message = "le pseudo doit contenir entre 3 et 20 caractères"
        ),
        custom(
            function = "valider_pseudo",
            message = "le pseudo ne peut contenir que des lettres, chiffres, '_' et '-'"
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
    /// Accepte un courriel OU un pseudo, comme demandé dans l'énoncé —
    /// plafonné à la même longueur que `courriel` (254, le plus grand des
    /// deux formats possibles) : sans borne, cette route non authentifiée
    /// accepterait un identifiant arbitrairement volumineux. Pas de
    /// validation de charset ici (contrairement à `pseudo` à
    /// l'inscription) : un courriel valide contient légitimement des
    /// caractères qu'un pseudo n'a pas le droit d'avoir (`@`, `.`).
    #[validate(length(
        min = 1,
        max = "LONGUEUR_MAX_COURRIEL",
        message = "identifiant requis (254 caractères maximum)"
    ))]
    pub identifiant: String,
    /// Plafonné comme `InscriptionRequest::mot_de_passe` : `connecter`
    /// (voir `AuthService`) appelle Argon2 sur ce champ à *chaque*
    /// tentative de connexion, y compris pour un identifiant inconnu
    /// (hachage factice) — sans borne, `/login`, route non authentifiée,
    /// serait un vecteur de déni de service encore plus direct que
    /// `/register`.
    #[validate(length(
        min = 1,
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "mot de passe requis (128 caractères maximum)"
    ))]
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

/// `PATCH /api/auth/me` : modification de son propre compte (pseudo et/ou
/// mot de passe). Tous les champs sont optionnels (au moins un requis,
/// vérifié dans `AuthService::modifier_profil`, pas exprimable avec
/// `validator` seul). Le courriel n'est PAS modifiable par l'utilisateur
/// lui-même (décision produit) : `deny_unknown_fields` fait refuser (400)
/// une requête qui en contiendrait un, plutôt que de l'ignorer en silence.
/// Seul un administrateur peut le changer (`PATCH /api/users/{id}`).
/// Note : `validator` valide correctement `Option<String>`, y compris
/// pour un validateur `custom` comme `valider_pseudo` (`None` est
/// toujours considéré valide, seul un `Some` est déballé puis vérifié).
#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ModifierProfilRequest {
    #[validate(
        length(
            min = 3,
            max = 20,
            message = "le pseudo doit contenir entre 3 et 20 caractères"
        ),
        custom(
            function = "valider_pseudo",
            message = "le pseudo ne peut contenir que des lettres, chiffres, '_' et '-'"
        )
    )]
    pub pseudo: Option<String>,
    #[validate(length(
        min = 8,
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "le mot de passe doit contenir entre 8 et 128 caractères"
    ))]
    pub mot_de_passe: Option<String>,
    /// Requis (vérifié dans `AuthService::modifier_profil`, pas
    /// exprimable avec `validator` seul) dès que `mot_de_passe` est
    /// modifié — jamais pour un changement de `pseudo` seul. Empêche qu'un jeton volé/intercepté (fenêtre
    /// d'exploitation limitée à son expiration) ne se transforme en
    /// prise de contrôle permanente du compte (changement du mot de
    /// passe ou du courriel sans reconfirmer le mot de passe actuel) —
    /// pratique recommandée par l'OWASP pour les actions sensibles sur
    /// un compte.
    #[validate(length(
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "mot de passe actuel trop long"
    ))]
    pub mot_de_passe_actuel: Option<String>,
}

/// `DELETE /api/auth/me` : confirmation requise avant une suppression
/// irréversible de son propre compte.
#[derive(Debug, Deserialize, Validate)]
pub struct SupprimerCompteRequest {
    #[validate(length(
        min = 1,
        max = "LONGUEUR_MAX_MOT_DE_PASSE",
        message = "mot de passe actuel requis pour supprimer le compte"
    ))]
    pub mot_de_passe_actuel: String,
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
        custom(
            function = "valider_pseudo",
            message = "le pseudo ne peut contenir que des lettres, chiffres, '_' et '-'"
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
