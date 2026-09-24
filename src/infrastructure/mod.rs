//! Couche `infrastructure` : détails techniques (base de données, hachage,
//! JWT). C'est la seule couche qui "sait" qu'on utilise PostgreSQL, Argon2
//! ou JWT — le reste de l'application ne dépend que des ports définis
//! dans `domain::ports`.

pub mod db;
#[cfg(test)]
pub mod fake_utilisateur_repository;
pub mod jwt;
pub mod mot_de_passe;
pub mod notificateur_suppression;
pub mod pg_utilisateur_repository;
