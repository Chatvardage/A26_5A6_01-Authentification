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

use crate::error::AppError;

/// Contenu ("claims") encodé dans le jeton. `sub` et `exp` sont des noms
/// imposés par la norme JWT elle-même (RFC 7519) — on ne les traduit PAS
/// en français, contrairement au reste : n'importe quel outil/bibliothèque
/// JWT (même dans un autre langage) s'attend à ces noms précis.
/// - `sub` (subject) : l'identifiant du sujet du jeton — ici, l'id de
///   l'utilisateur.
/// - `exp` (expiration) : vérifié automatiquement par la crate
///   `jsonwebtoken` au décodage — un jeton expiré est rejeté.
///
/// Le rôle n'est volontairement pas un claim : une autorisation doit
/// toujours s'appuyer sur le rôle courant stocké par ce service, jamais sur
/// une valeur figée lors de l'émission du jeton.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub version_auth: i32,
    pub exp: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<Uuid>,
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
        version_auth: i32,
        famille: Option<Uuid>,
    ) -> Result<String, AppError> {
        let expiration = Utc::now()
            .checked_add_signed(if famille.is_some() {
                Duration::minutes(15)
            } else {
                Duration::hours(self.duree_de_vie_heures)
            })
            .expect("le calcul de l'expiration ne devrait jamais déborder")
            .timestamp() as usize;

        let claims = Claims {
            sub: id_utilisateur.to_string(),
            version_auth,
            exp: expiration,
            sid: famille,
            jti: famille.map(|_| Uuid::new_v4()),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|e| AppError::Interne(format!("échec de génération du jeton: {e}")))
    }

    pub fn valider_token(&self, jeton: &str) -> Result<Claims, AppError> {
        let mut validation = Validation::default();
        validation.leeway = 0;
        decode::<Claims>(
            jeton,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &validation,
        )
        .map(|data| data.claims)
        .map_err(|_| AppError::NonAutorise("jeton invalide ou expiré".into()))
    }

    /// Les nouvelles sessions utilisent un accès de 15 minutes; les anciens JWT restent compatibles.
    pub fn generer_session(
        &self,
        id: Uuid,
        version: i32,
        famille: Uuid,
    ) -> Result<String, AppError> {
        self.generer_token(id, version, Some(famille))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiration_stricte_et_session_unique() {
        let service = JwtService::new("secret-fixture".into(), 24);
        let id = Uuid::new_v4();
        let famille = Uuid::new_v4();
        let a = service.generer_session(id, 0, famille).unwrap();
        let b = service.generer_session(id, 0, famille).unwrap();
        assert_ne!(a, b);
        let claims = service.valider_token(&a).unwrap();
        assert_eq!(claims.sid, Some(famille));
        assert!(claims.exp <= (Utc::now() + Duration::minutes(15)).timestamp() as usize);
        let expire = Claims {
            sub: id.to_string(),
            version_auth: 0,
            exp: (Utc::now().timestamp() - 1) as usize,
            sid: None,
            jti: None,
        };
        let token = encode(
            &Header::default(),
            &expire,
            &EncodingKey::from_secret(b"secret-fixture"),
        )
        .unwrap();
        assert!(service.valider_token(&token).is_err());
    }

    #[test]
    fn genere_puis_valide_un_jeton() {
        let service = JwtService::new("secret".into(), 24);
        let id = Uuid::new_v4();

        let jeton = service.generer_token(id, 0, None).unwrap();
        let claims = service.valider_token(&jeton).unwrap();

        assert_eq!(claims.sub, id.to_string());
        assert_eq!(claims.version_auth, 0);
    }

    #[test]
    fn rejette_un_jeton_expire() {
        // Durée de vie négative : le jeton est déjà expiré au moment où
        // il est émis.
        let service = JwtService::new("secret".into(), -1);
        let jeton = service.generer_token(Uuid::new_v4(), 0, None).unwrap();

        assert!(service.valider_token(&jeton).is_err());
    }

    #[test]
    fn rejette_un_jeton_signe_avec_un_autre_secret() {
        let service_a = JwtService::new("secret-a".into(), 24);
        let service_b = JwtService::new("secret-b".into(), 24);
        let jeton = service_a.generer_token(Uuid::new_v4(), 0, None).unwrap();

        assert!(service_b.valider_token(&jeton).is_err());
    }

    #[test]
    fn rejette_un_jeton_mal_forme() {
        let service = JwtService::new("secret".into(), 24);
        assert!(service.valider_token("pas-un-jwt").is_err());
    }
}
