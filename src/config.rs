//! Lecture de la configuration depuis les variables d'environnement.
//! Ces variables sont fournies par `.env` en local, ou directement par
//! `docker-compose.yml` en conteneur — le code ne change pas selon
//! l'environnement, seule la source des valeurs change.

#[cfg(test)]
pub static VERROU_ENV_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Longueur minimale acceptée pour `JWT_SECRET` (256 bits si le secret
/// est aussi aléatoire que long — la taille de sortie native de HMAC-SHA256,
/// l'algorithme par défaut de `jsonwebtoken`). `std::env::var` réussit dès
/// que la variable existe, même vide (ex. `.env.example` copié sans être
/// rempli) : sans ce plancher, le service démarrerait normalement mais
/// signerait tous les jetons avec une clé HMAC vide ou trivialement
/// devinable — un secret faible compromet silencieusement TOUTE
/// l'authentification, contrairement à une `DATABASE_URL` invalide (qui
/// empêche déjà le démarrage en échouant la connexion PostgreSQL).
const LONGUEUR_MIN_JWT_SECRET: usize = 32;

/// Bornes acceptées pour `JWT_DUREE_DE_VIE_HEURES`. Deux raisons distinctes :
///
/// 1. `JwtService::generer_token` (`infrastructure/jwt.rs`) calcule
///    l'expiration via `Duration::hours(n).checked_add_signed(...)`,
///    protégé par un simple `.expect(...)` (pas une `AppError`) car cette
///    valeur est censée être une constante de configuration validée au
///    démarrage, pas une entrée utilisateur — sans borne haute, une
///    valeur aberrante dans `.env` (une faute de frappe avec trop de
///    chiffres) ferait paniquer `Duration::hours` à *chaque* tentative de
///    connexion/inscription (pas seulement au démarrage) au lieu
///    d'échouer une fois, clairement, ici.
const JWT_DUREE_MIN_HEURES: i64 = 1;
const JWT_DUREE_MAX_HEURES: i64 = 720; // 30 jours

pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_duree_de_vie_heures: i64,
    pub port: u16,
    /// Liste blanche CORS (voir `interface::http::routes::construire`).
    /// `None` ou une liste vide n'autorise aucune origine : l'infrastructure
    /// doit fournir `CORS_ALLOWED_ORIGINS` explicitement.
    pub cors_allowed_origins: Option<Vec<String>>,
}

impl Config {
    pub fn depuis_env() -> Self {
        // `.env` est optionnel : en conteneur Docker, les variables sont
        // déjà injectées par docker-compose, donc on ignore l'erreur si
        // le fichier n'existe pas.
        let _ = dotenvy::dotenv();
        Self::depuis_variables_environnement()
    }

    /// Analyse pure des variables d'environnement déjà présentes dans le
    /// process, séparée de `depuis_env()` (qui charge `.env` au
    /// préalable) uniquement pour que les tests puissent fixer des
    /// variables précises sans dépendre — ni être perturbés — par un
    /// éventuel fichier `.env` local sur la machine qui exécute les
    /// tests.
    fn depuis_variables_environnement() -> Self {
        let jwt_secret =
            std::env::var("JWT_SECRET").expect("JWT_SECRET doit être définie (voir .env.example)");
        assert!(
            jwt_secret.len() >= LONGUEUR_MIN_JWT_SECRET,
            "JWT_SECRET doit contenir au moins {LONGUEUR_MIN_JWT_SECRET} caractères (voir .env.example) \
             — un secret vide ou trop court rendrait tous les jetons falsifiables"
        );

        let jwt_duree_de_vie_heures = lire_var_numerique("JWT_DUREE_DE_VIE_HEURES", 24i64);
        assert!(
            (JWT_DUREE_MIN_HEURES..=JWT_DUREE_MAX_HEURES).contains(&jwt_duree_de_vie_heures),
            "JWT_DUREE_DE_VIE_HEURES doit être comprise entre {JWT_DUREE_MIN_HEURES} et {JWT_DUREE_MAX_HEURES} \
             heures (voir .env.example), reçu {jwt_duree_de_vie_heures}"
        );

        Self {
            database_url: std::env::var("DATABASE_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .expect("DATABASE_URL doit être définie et non vide (voir .env.example) — PostgreSQL est requis, il n'y a pas de mode mock"),
            jwt_secret,
            jwt_duree_de_vie_heures,
            port: lire_var_numerique("PORT", 3001u16),
            cors_allowed_origins: std::env::var("CORS_ALLOWED_ORIGINS").ok().map(|v| {
                v.split(',')
                    .map(|origine| origine.trim().to_string())
                    .filter(|origine| !origine.is_empty())
                    .collect()
            }),
        }
    }
}

/// Lit une variable d'environnement numérique optionnelle. Contrairement
/// à `DATABASE_URL`/`JWT_SECRET` (obligatoires, le démarrage échoue si
/// elles manquent ou sont invalides), une valeur *présente mais
/// mal formée* ici (ex. `PORT=abcd`, une faute de frappe) retombe sur
/// `defaut` plutôt que de bloquer le démarrage — mais pas silencieusement :
/// un avertissement est journalisé pour que l'erreur ne passe pas
/// inaperçue.
fn lire_var_numerique<T: std::str::FromStr>(nom: &str, defaut: T) -> T {
    let Some(valeur) = std::env::var(nom).ok().filter(|v| !v.is_empty()) else {
        return defaut;
    };
    valeur.parse().unwrap_or_else(|_| {
        tracing::warn!(
            variable = nom,
            valeur,
            "valeur invalide, valeur par défaut utilisée"
        );
        defaut
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // `std::env` est un état global partagé par tout le process : sans
    // ce verrou, les tests de ce module (exécutés en parallèle par
    // défaut par `cargo test`) se marcheraient dessus.
    /// Fixe les variables d'environnement listées (`None` = retirée) le
    /// temps d'exécuter `f`, puis restaure exactement leur valeur
    /// précédente — même si `f` panique (`#[should_panic]`).
    fn avec_env<F>(vars: &[(&str, Option<&str>)], f: F)
    where
        F: FnOnce() + std::panic::UnwindSafe,
    {
        let _garde = VERROU_ENV_TEST.lock().unwrap_or_else(|e| e.into_inner());

        let precedentes: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(cle, _)| (cle.to_string(), std::env::var(cle).ok()))
            .collect();

        for (cle, valeur) in vars {
            match valeur {
                Some(v) => unsafe { std::env::set_var(cle, v) },
                None => unsafe { std::env::remove_var(cle) },
            }
        }

        let resultat = std::panic::catch_unwind(f);

        for (cle, valeur) in precedentes {
            match valeur {
                Some(v) => unsafe { std::env::set_var(&cle, v) },
                None => unsafe { std::env::remove_var(&cle) },
            }
        }

        if let Err(panique) = resultat {
            std::panic::resume_unwind(panique);
        }
    }

    // Toutes les variables utilisées par `Config` sont explicitement
    // fixées (à une valeur ou à `None`) dans chaque test ci-dessous via
    // `depuis_variables_environnement()` (pas `depuis_env()`) : un
    // éventuel fichier `.env` local (ignoré par Git, mais souvent présent
    // pendant le développement — voir `.env.example`) ne peut donc pas
    // perturber ces tests en fournissant une valeur de secours.

    /// 32 caractères pile — la longueur minimale acceptée
    /// (`LONGUEUR_MIN_JWT_SECRET`) — pour que les tests "valeur valide"
    /// restent un cas limite significatif, pas une valeur confortablement
    /// au-dessus du seuil qui ne le testerait pas vraiment.
    const SECRET_VALIDE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn utilise_les_valeurs_par_defaut_si_absentes() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("JWT_DUREE_DE_VIE_HEURES", None),
                ("PORT", None),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert_eq!(config.database_url, "postgres://x/y");
                assert_eq!(config.jwt_secret, SECRET_VALIDE);
                assert_eq!(config.jwt_duree_de_vie_heures, 24);
                assert_eq!(config.port, 3001);
            },
        );
    }

    #[test]
    fn lit_les_valeurs_fournies() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("JWT_DUREE_DE_VIE_HEURES", Some("48")),
                ("PORT", Some("4000")),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert_eq!(config.jwt_duree_de_vie_heures, 48);
                assert_eq!(config.port, 4000);
            },
        );
    }

    #[test]
    fn port_invalide_retombe_sur_le_defaut_sans_paniquer() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("PORT", Some("abcd")),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert_eq!(config.port, 3001);
            },
        );
    }

    #[test]
    fn jwt_duree_de_vie_heures_invalide_retombe_sur_le_defaut_sans_paniquer() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("JWT_DUREE_DE_VIE_HEURES", Some("pas-un-nombre")),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert_eq!(config.jwt_duree_de_vie_heures, 24);
            },
        );
    }

    #[test]
    #[should_panic(expected = "JWT_DUREE_DE_VIE_HEURES doit être comprise entre")]
    fn panique_si_jwt_duree_de_vie_heures_trop_grande() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                // Assez grand pour dépasser JWT_DUREE_MAX_HEURES, mais un
                // i64 tout à fait valide (pas de simple échec de parsing).
                ("JWT_DUREE_DE_VIE_HEURES", Some("100000000")),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "JWT_DUREE_DE_VIE_HEURES doit être comprise entre")]
    fn panique_si_jwt_duree_de_vie_heures_nulle_ou_negative() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("JWT_DUREE_DE_VIE_HEURES", Some("0")),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "DATABASE_URL")]
    fn panique_si_database_url_absente() {
        avec_env(
            &[("DATABASE_URL", None), ("JWT_SECRET", Some(SECRET_VALIDE))],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "DATABASE_URL doit être définie et non vide")]
    fn panique_si_database_url_est_vide() {
        avec_env(
            &[
                ("DATABASE_URL", Some("")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "JWT_SECRET")]
    fn panique_si_jwt_secret_absent() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", None),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "JWT_SECRET doit contenir au moins 32 caractères")]
    fn panique_si_jwt_secret_vide() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some("")),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    #[test]
    #[should_panic(expected = "JWT_SECRET doit contenir au moins 32 caractères")]
    fn panique_si_jwt_secret_trop_court() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                // 31 caractères : un de moins que le minimum accepté.
                ("JWT_SECRET", Some("0123456789abcdef0123456789abcde")),
            ],
            || {
                let _ = Config::depuis_variables_environnement();
            },
        );
    }

    /// Couvre `depuis_env()` elle-même (le mince appel à `dotenvy::dotenv()`
    /// avant de déléguer) : toutes les variables sont explicitement
    /// fournies, donc peu importe qu'un `.env` local en fournisse
    /// d'autres — `dotenvy` ne réécrit jamais une variable déjà présente.
    #[test]
    fn depuis_env_delegue_a_lanalyse_pure() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("JWT_DUREE_DE_VIE_HEURES", Some("12")),
                ("PORT", Some("9999")),
            ],
            || {
                let config = Config::depuis_env();
                assert_eq!(config.database_url, "postgres://x/y");
                assert_eq!(config.jwt_duree_de_vie_heures, 12);
                assert_eq!(config.port, 9999);
            },
        );
    }

    #[test]
    fn cors_allowed_origins_absente_donne_none() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                ("CORS_ALLOWED_ORIGINS", None),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert!(config.cors_allowed_origins.is_none());
            },
        );
    }

    #[test]
    fn cors_allowed_origins_est_analysee_et_nettoyee() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                ("JWT_SECRET", Some(SECRET_VALIDE)),
                (
                    "CORS_ALLOWED_ORIGINS",
                    Some("http://a.test, http://b.test ,, "),
                ),
            ],
            || {
                let config = Config::depuis_variables_environnement();
                assert_eq!(
                    config.cors_allowed_origins,
                    Some(vec![
                        "http://a.test".to_string(),
                        "http://b.test".to_string()
                    ])
                );
            },
        );
    }
}
