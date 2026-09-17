//! Couche `application` : orchestration des cas d'utilisation
//! (inscription, connexion, gestion des comptes). Ne contient pas de SQL
//! ni de détails HTTP — seulement de la logique métier qui combine les
//! autres couches.

pub mod auth_service;
pub mod dto;
