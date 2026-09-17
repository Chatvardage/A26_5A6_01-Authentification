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
    pub version_auth: i32,
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

    pub fn generer_token(
        &self,
        id_utilisateur: Uuid,
        pseudo: &str,
        role: Role,
        version_auth: i32,
    ) -> Result<String, AppError> {
        let expiration = Utc::now()
            .checked_add_signed(Duration::hours(self.duree_de_vie_heures))
            .expect("le calcul de l'expiration ne devrait jamais déborder")
            .timestamp() as usize;

        let claims = Claims {
            sub: id_utilisateur.to_string(),
            pseudo: pseudo.to_string(),
            role,
            version_auth,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genere_puis_valide_un_jeton() {
        let service = JwtService::new("secret".into(), 24);
        let id = Uuid::new_v4();

        let jeton = service
            .generer_token(id, "alice", Role::Utilisateur, 0)
            .unwrap();
        let claims = service.valider_token(&jeton).unwrap();

        assert_eq!(claims.sub, id.to_string());
        assert_eq!(claims.pseudo, "alice");
        assert_eq!(claims.role, Role::Utilisateur);
        assert_eq!(claims.version_auth, 0);
    }

    #[test]
    fn rejette_un_jeton_expire() {
        // Durée de vie négative : le jeton est déjà expiré au moment où
        // il est émis.
        let service = JwtService::new("secret".into(), -1);
        let jeton = service
            .generer_token(Uuid::new_v4(), "alice", Role::Utilisateur, 0)
            .unwrap();

        assert!(service.valider_token(&jeton).is_err());
    }

    #[test]
    fn rejette_un_jeton_signe_avec_un_autre_secret() {
        let service_a = JwtService::new("secret-a".into(), 24);
        let service_b = JwtService::new("secret-b".into(), 24);
        let jeton = service_a
            .generer_token(Uuid::new_v4(), "alice", Role::Utilisateur, 0)
            .unwrap();

        assert!(service_b.valider_token(&jeton).is_err());
    }

    #[test]
    fn rejette_un_jeton_mal_forme() {
        let service = JwtService::new("secret".into(), 24);
        assert!(service.valider_token("pas-un-jwt").is_err());
    }
}
