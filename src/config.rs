//! Lecture de la configuration depuis les variables d'environnement.
//! Ces variables sont fournies par `.env` en local, ou directement par
//! `docker-compose.yml` en conteneur — le code ne change pas selon
//! l'environnement, seule la source des valeurs change.

/// Longueur minimale acceptée pour `JWT_SECRET` (128 bits si le secret
/// est aussi aléatoire que long). `std::env::var` réussit dès que la
/// variable existe, même vide (ex. `.env.example` copié sans être
/// rempli) : sans ce plancher, le service démarrerait normalement mais
/// signerait tous les jetons avec une clé HMAC vide ou trivialement
/// devinable — un secret faible compromet silencieusement TOUTE
/// l'authentification, contrairement à une `DATABASE_URL` invalide (qui
/// empêche déjà le démarrage en échouant la connexion PostgreSQL).
const LONGUEUR_MIN_JWT_SECRET: usize = 16;

pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_duree_de_vie_heures: i64,
    pub port: u16,
    /// Liste blanche CORS (voir `interface::http::routes::construire`).
    /// `None` = pas de restriction (`Access-Control-Allow-Origin: *`) —
    /// c'est la valeur par défaut, pratique en développement solo, mais
    /// ce service est censé rester interne (voir AGENTS.md) : `infra`
    /// devrait fournir la vraie liste des origines via
    /// `CORS_ALLOWED_ORIGINS` en production.
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

        Self {
            database_url: std::env::var("DATABASE_URL")
                .expect("DATABASE_URL doit être définie (voir .env.example) — PostgreSQL est requis, il n'y a pas de mode mock"),
            jwt_secret,
            jwt_duree_de_vie_heures: std::env::var("JWT_DUREE_DE_VIE_HEURES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(24),
            port: std::env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3001),
            cors_allowed_origins: std::env::var("CORS_ALLOWED_ORIGINS").ok().map(|v| {
                v.split(',')
                    .map(|origine| origine.trim().to_string())
                    .filter(|origine| !origine.is_empty())
                    .collect()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // `std::env` est un état global partagé par tout le process : sans
    // ce verrou, les tests de ce module (exécutés en parallèle par
    // défaut par `cargo test`) se marcheraient dessus.
    static VERROU_ENV: Mutex<()> = Mutex::new(());

    /// Fixe les variables d'environnement listées (`None` = retirée) le
    /// temps d'exécuter `f`, puis restaure exactement leur valeur
    /// précédente — même si `f` panique (`#[should_panic]`).
    fn avec_env<F>(vars: &[(&str, Option<&str>)], f: F)
    where
        F: FnOnce() + std::panic::UnwindSafe,
    {
        let _garde = VERROU_ENV.lock().unwrap_or_else(|e| e.into_inner());

        let precedentes: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(cle, _)| (cle.to_string(), std::env::var(cle).ok()))
            .collect();

        for (cle, valeur) in vars {
            match valeur {
                Some(v) => std::env::set_var(cle, v),
                None => std::env::remove_var(cle),
            }
        }

        let resultat = std::panic::catch_unwind(f);

        for (cle, valeur) in precedentes {
            match valeur {
                Some(v) => std::env::set_var(&cle, v),
                None => std::env::remove_var(&cle),
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

    /// 16 caractères pile — la longueur minimale acceptée
    /// (`LONGUEUR_MIN_JWT_SECRET`) — pour que les tests "valeur valide"
    /// restent un cas limite significatif, pas une valeur confortablement
    /// au-dessus du seuil qui ne le testerait pas vraiment.
    const SECRET_VALIDE: &str = "0123456789abcdef";

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
    #[should_panic(expected = "JWT_SECRET doit contenir au moins 16 caractères")]
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
    #[should_panic(expected = "JWT_SECRET doit contenir au moins 16 caractères")]
    fn panique_si_jwt_secret_trop_court() {
        avec_env(
            &[
                ("DATABASE_URL", Some("postgres://x/y")),
                // 15 caractères : un de moins que le minimum accepté.
                ("JWT_SECRET", Some("012345678901234")),
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
