//! Hachage et vérification des mots de passe avec Argon2.
//!
//! RÈGLE D'OR : on ne stocke JAMAIS un mot de passe en clair. On stocke un
//! "hash" (empreinte à sens unique) : impossible de retrouver le mot de
//! passe à partir du hash, mais facile de vérifier qu'un mot de passe
//! donné correspond au hash stocké.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use tokio::task;

use crate::error::AppError;

const COUT_MEMOIRE_ARGON2_KIO: u32 = 19 * 1024;
const ITERATIONS_ARGON2: u32 = 2;
const PARALLELISME_ARGON2: u32 = 1;

fn argon2() -> Argon2<'static> {
    // Paramètres Argon2id par défaut explicités : 19 Mio, deux itérations
    // et une voie. Ce compromis ralentit une attaque hors ligne tout en
    // restant raisonnable pour une authentification interactive.
    let params = Params::new(
        COUT_MEMOIRE_ARGON2_KIO,
        ITERATIONS_ARGON2,
        PARALLELISME_ARGON2,
        None,
    )
    .expect("les paramètres Argon2 constants sont valides");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hache un mot de passe en clair sans bloquer un worker Tokio. Le "sel"
/// (salt) est généré aléatoirement à chaque appel : deux utilisateurs avec
/// le même mot de passe auront des hashs différents en base, ce qui empêche
/// une attaque par table précalculée (rainbow table).
pub async fn hacher_mot_de_passe(clair: &str) -> Result<String, AppError> {
    let clair = clair.to_owned();
    task::spawn_blocking(move || {
        let sel = SaltString::generate(&mut OsRng);
        let argon2 = argon2();
        let hache = argon2
            .hash_password(clair.as_bytes(), &sel)
            .map_err(|e| AppError::Interne(format!("échec du hachage du mot de passe: {e}")))?;
        Ok(hache.to_string())
    })
    .await
    .map_err(|e| AppError::Interne(format!("tâche de hachage du mot de passe interrompue: {e}")))?
}

/// Vérifie qu'un mot de passe en clair correspond bien à un hash stocké en
/// base sans bloquer un worker Tokio. Retourne `false` pour un mot de passe
/// incorrect ou un hash invalide : ce sont des cas normaux.
pub async fn verifier_mot_de_passe(clair: &str, hache: &str) -> Result<bool, AppError> {
    let clair = clair.to_owned();
    let hache = hache.to_owned();
    task::spawn_blocking(move || {
        let Ok(hache_analyse) = PasswordHash::new(&hache) else {
            return Ok(false);
        };
        Ok(argon2()
            .verify_password(clair.as_bytes(), &hache_analyse)
            .is_ok())
    })
    .await
    .map_err(|e| {
        AppError::Interne(format!(
            "tâche de vérification du mot de passe interrompue: {e}"
        ))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn un_mot_de_passe_haché_se_vérifie_correctement() {
        let hache = hacher_mot_de_passe("motdepasse123")
            .await
            .expect("le hachage doit réussir");
        assert!(verifier_mot_de_passe("motdepasse123", &hache)
            .await
            .expect("la vérification doit réussir"));
    }

    #[test]
    fn parametres_argon2_sont_explicitement_calibres() {
        let argon2 = argon2();
        let params = argon2.params();
        assert_eq!(params.m_cost(), COUT_MEMOIRE_ARGON2_KIO);
        assert_eq!(params.t_cost(), ITERATIONS_ARGON2);
        assert_eq!(params.p_cost(), PARALLELISME_ARGON2);
    }

    #[tokio::test]
    async fn un_mauvais_mot_de_passe_est_rejeté() {
        let hache = hacher_mot_de_passe("motdepasse123")
            .await
            .expect("le hachage doit réussir");
        assert!(!verifier_mot_de_passe("autre-mot-de-passe", &hache)
            .await
            .expect("la vérification doit réussir"));
    }

    #[tokio::test]
    async fn deux_hachages_du_même_mot_de_passe_sont_différents() {
        // Grâce au sel aléatoire — voir le commentaire sur hacher_mot_de_passe.
        let a = hacher_mot_de_passe("motdepasse123").await.unwrap();
        let b = hacher_mot_de_passe("motdepasse123").await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn un_hache_mal_forme_est_rejeté_sans_paniquer() {
        // Ex. une donnée corrompue en base : ne doit jamais paniquer, ni
        // être traité comme un mot de passe valide.
        assert!(
            !verifier_mot_de_passe("motdepasse123", "pas-un-hachage-argon2")
                .await
                .expect("la vérification doit réussir")
        );
    }
}
