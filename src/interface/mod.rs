//! Couche `interface` : point d'entrée du service (ici, HTTP uniquement
//! pour ce contrôle — WebSocket temps réel pour les messages privés est
//! hors du périmètre "authentification" mais pourrait s'ajouter ici plus
//! tard, dans un sous-module `websocket/`, en suivant Architecture.md).

pub mod http;
