//! Lecture de la configuration depuis les variables d'environnement.
//! Ces variables sont fournies par `.env` en local, ou directement par
//! `docker-compose.yml` / le repo infra en conteneur — le code ne change
//! pas selon l'environnement, seule la source des valeurs change.

pub struct Config {
    /// `None` uniquement possible quand `use_mock_db` est `true` (aucune
    /// connexion PostgreSQL n'est alors nécessaire).
    pub database_url: Option<String>,
    /// Si `true`, le service utilise `MockUtilisateurRepository` (données
    /// en mémoire, aucune connexion à PostgreSQL) au lieu de la vraie
    /// base — utile pour développer sans attendre que le service qui
    /// gère les migrations ait créé la table `utilisateurs`. Voir README.
    pub use_mock_db: bool,
    pub jwt_secret: String,
    pub jwt_duree_de_vie_heures: i64,
    pub port: u16,
}

impl Config {
    pub fn depuis_env() -> Self {
        // `.env` est optionnel : en conteneur Docker, les variables sont
        // déjà injectées par docker-compose, donc on ignore l'erreur si
        // le fichier n'existe pas.
        let _ = dotenvy::dotenv();

        let use_mock_db = std::env::var("USE_MOCK_DB")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let database_url = std::env::var("DATABASE_URL").ok();
        if !use_mock_db && database_url.is_none() {
            panic!(
                "DATABASE_URL doit être définie quand USE_MOCK_DB n'est pas \"true\" \
                 (voir .env.example) — ou lancez avec USE_MOCK_DB=true pour développer \
                 sans PostgreSQL."
            );
        }

        Self {
            database_url,
            use_mock_db,
            jwt_secret: std::env::var("JWT_SECRET")
                .expect("JWT_SECRET doit être définie (voir .env.example)"),
            jwt_duree_de_vie_heures: std::env::var("JWT_DUREE_DE_VIE_HEURES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(24),
            port: std::env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3001),
        }
    }
}
