//! Émission et validation des JSON Web Tokens (JWT).
//!
//! Pourquoi un JWT ? L'énoncé demande une "authentification simple" et
//! une API réutilisée par plusieurs clients (desktop, éventuellement
//! mobile). Un JWT est un jeton signé que le client garde et renvoie à
//! chaque requête (header `Authorization: Bearer <jeton>`) : le serveur
//! n'a pas besoin de garder l'état de session en mémoire (contrairement
//! à des sessions classiques), ce qui simplifie beaucoup l'API.

use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::utilisateur::Role;
use crate::error::AppError;

/// Contenu ("claims") encodé dans le jeton. `sub` et `exp` sont des noms
/// imposés par la norme JWT elle-même (RFC 7519) — on ne les traduit PAS
/// en français, contrairement au reste : n'importe quel outil/bibliothèque
/// JWT (même dans un autre langage) s'attend à ces noms précis.
/// - `sub` (subject) : l'identifiant du sujet du jeton — ici, l'id de
///   l'utilisateur.
/// - `exp` (expiration) : vérifié automatiquement par la crate
///   `jsonwebtoken` au décodage — un jeton expiré est rejeté.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub pseudo: String,
    pub role: Role,
    pub exp: usize,
}

pub struct JwtService {
    secret: String,
    duree_de_vie_heures: i64,
}

impl JwtService {
    pub fn new(secret: String, duree_de_vie_heures: i64) -> Self {
        Self {
            secret,
            duree_de_vie_heures,
        }
    }

    pub fn generer_token(&self, id_utilisateur: Uuid, pseudo: &str, role: Role) -> Result<String, AppError> {
        let expiration = Utc::now()
            .checked_add_signed(Duration::hours(self.duree_de_vie_heures))
            .expect("le calcul de l'expiration ne devrait jamais déborder")
            .timestamp() as usize;

        let claims = Claims {
            sub: id_utilisateur.to_string(),
            pseudo: pseudo.to_string(),
            role,
            exp: expiration,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|e| AppError::Interne(format!("échec de génération du jeton: {e}")))
    }

    pub fn valider_token(&self, jeton: &str) -> Result<Claims, AppError> {
        decode::<Claims>(
            jeton,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )
        .map(|data| data.claims)
        .map_err(|_| AppError::NonAutorise("jeton invalide ou expiré".into()))
    }
}
