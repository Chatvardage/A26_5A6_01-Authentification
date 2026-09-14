//! Couche `infrastructure` : détails techniques (base de données, hachage,
//! JWT). C'est la seule couche qui "sait" qu'on utilise PostgreSQL (ou un
//! mock en mémoire), Argon2 ou JWT — le reste de l'application ne dépend
//! que des ports définis dans `domain::ports`.

pub mod db;
pub mod jwt;
pub mod mock_utilisateur_repository;
pub mod mot_de_passe;
pub mod pg_utilisateur_repository;
