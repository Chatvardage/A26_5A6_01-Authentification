//! Couche `domain` : le cœur métier, sans dépendance à HTTP ni à une base
//! de données précise (voir Architecture.md du frontend, dont on reprend
//! ici la même idée de couches pour rester cohérent avec le reste de
//! l'équipe : interface -> application -> domain -> infrastructure).

pub mod ports;
pub mod utilisateur;
