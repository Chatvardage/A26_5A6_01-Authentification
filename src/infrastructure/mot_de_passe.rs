//! Hachage et vérification des mots de passe avec Argon2.
//!
//! RÈGLE D'OR : on ne stocke JAMAIS un mot de passe en clair. On stocke un
//! "hash" (empreinte à sens unique) : impossible de retrouver le mot de
//! passe à partir du hash, mais facile de vérifier qu'un mot de passe
//! donné correspond au hash stocké.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

use crate::error::AppError;

/// Hache un mot de passe en clair. Le "sel" (salt) est généré
/// aléatoirement à chaque appel : deux utilisateurs avec le même mot de
/// passe auront des hashs différents en base, ce qui empêche une attaque
/// par table précalculée (rainbow table).
pub fn hacher_mot_de_passe(clair: &str) -> Result<String, AppError> {
    let sel = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hache = argon2
        .hash_password(clair.as_bytes(), &sel)
        .map_err(|e| AppError::Interne(format!("échec du hachage du mot de passe: {e}")))?;
    Ok(hache.to_string())
}

/// Vérifie qu'un mot de passe en clair correspond bien à un hash stocké
/// en base. Retourne `true`/`false` plutôt qu'une erreur : un mot de
/// passe incorrect n'est pas un bug, c'est un cas normal.
pub fn verifier_mot_de_passe(clair: &str, hache: &str) -> bool {
    let Ok(hache_analyse) = PasswordHash::new(hache) else {
        return false;
    };
    Argon2::default()
        .verify_password(clair.as_bytes(), &hache_analyse)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_mot_de_passe_haché_se_vérifie_correctement() {
        let hache = hacher_mot_de_passe("motdepasse123").expect("le hachage doit réussir");
        assert!(verifier_mot_de_passe("motdepasse123", &hache));
    }

    #[test]
    fn un_mauvais_mot_de_passe_est_rejeté() {
        let hache = hacher_mot_de_passe("motdepasse123").expect("le hachage doit réussir");
        assert!(!verifier_mot_de_passe("autre-mot-de-passe", &hache));
    }

    #[test]
    fn deux_hachages_du_même_mot_de_passe_sont_différents() {
        // Grâce au sel aléatoire — voir le commentaire sur hacher_mot_de_passe.
        let a = hacher_mot_de_passe("motdepasse123").unwrap();
        let b = hacher_mot_de_passe("motdepasse123").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn un_hache_mal_forme_est_rejeté_sans_paniquer() {
        // Ex. une donnée corrompue en base : ne doit jamais paniquer, ni
        // être traité comme un mot de passe valide.
        assert!(!verifier_mot_de_passe(
            "motdepasse123",
            "pas-un-hachage-argon2"
        ));
    }
}
