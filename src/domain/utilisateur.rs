//! Entité métier `Utilisateur` et le rôle qui lui est associé.
//!
//! En Rust, une "entité" comme celle-ci est souvent une simple `struct`
//! (pas de classe, pas d'héritage). Les règles qui doivent TOUJOURS être
//! vraies pour un utilisateur (ex. : un rôle ne peut être que l'une de ces
//! trois valeurs) vivent ici, pas dans les contrôleurs HTTP.
//!
//! Note sur les noms : tous les identifiants Rust ci-dessous évitent les
//! accents (ex. `cree_le` plutôt que `créé_le`) — les accents restent
//! valides en Rust, mais certains outils/éditeurs les gèrent mal ; les
//! commentaires et messages, eux, gardent le français complet.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Rôle d'un utilisateur, utilisé pour les permissions (voir énoncé :
/// "gestion des permissions (ex. : admin, modérateur, utilisateur)").
///
/// `#[derive(...)]` génère automatiquement du code pour nous : ici, la
/// capacité à comparer deux rôles (`PartialEq`), à les dupliquer (`Clone`,
/// `Copy`) et à les convertir en JSON / depuis JSON (`Serialize`,
/// `Deserialize`). C'est l'équivalent Rust de décorateurs ou d'attributs
/// dans d'autres langages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Moderateur,
    Utilisateur,
}

impl Role {
    /// Convertit le rôle vers la représentation texte stockée en base
    /// (colonne `role` de la table `utilisateurs`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Moderateur => "moderateur",
            Role::Utilisateur => "utilisateur",
        }
    }

    /// Chemin inverse : texte venant de la base -> Role. On ne fait
    /// jamais confiance aveuglément à une valeur externe (même si elle
    /// vient de notre propre DB) : si le texte est inattendu, on tombe
    /// sur `Utilisateur` par défaut plutôt que de planter.
    pub fn from_str(value: &str) -> Self {
        match value {
            "admin" => Role::Admin,
            "moderateur" => Role::Moderateur,
            _ => Role::Utilisateur,
        }
    }
}

/// Représentation d'un utilisateur telle que stockée en base de données
/// (table `utilisateurs` — voir README, section "Schéma de données
/// attendu"). `FromRow` permet à sqlx de construire un `Utilisateur`
/// directement à partir d'une ligne PostgreSQL (voir
/// `PgUtilisateurRepository`). On garde volontairement `role` en
/// `String` ici (la colonne SQL brute) plutôt que de faire convertir
/// automatiquement par sqlx : c'est moins "magique" et donc plus facile
/// à suivre pour un premier projet Rust — la conversion en `Role` se
/// fait explicitement via `role()` ci-dessous.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Utilisateur {
    pub id: Uuid,
    pub pseudo: String,
    pub courriel: String,
    /// Jamais exposé dans une réponse HTTP : voir `UtilisateurPublic`
    /// plus bas, qui est le type réellement renvoyé au frontend.
    pub mot_de_passe_hache: String,
    pub role: String,
    pub cree_le: chrono::DateTime<chrono::Utc>,
    pub modifie_le: chrono::DateTime<chrono::Utc>,
}

impl Utilisateur {
    /// Convertit la colonne texte `role` en `Role` métier.
    pub fn role(&self) -> Role {
        Role::from_str(&self.role)
    }
}

/// Vue publique d'un utilisateur : c'est CE type (jamais `Utilisateur`)
/// qu'on sérialise dans les réponses HTTP, pour être certain que
/// `mot_de_passe_hache` ne fuite jamais vers le client.
#[derive(Debug, Clone, Serialize)]
pub struct UtilisateurPublic {
    pub id: Uuid,
    pub pseudo: String,
    pub courriel: String,
    pub role: Role,
    pub cree_le: chrono::DateTime<chrono::Utc>,
}

impl From<Utilisateur> for UtilisateurPublic {
    fn from(u: Utilisateur) -> Self {
        // On calcule `role` AVANT de déplacer les autres champs hors de
        // `u` : `u.role()` a besoin d'emprunter `u` en entier (`&self`),
        // ce qui n'est plus possible une fois qu'un champ (ex. `courriel`)
        // a déjà été "déplacé" (move) dans le struct littéral ci-dessous.
        let role = u.role();
        UtilisateurPublic {
            id: u.id,
            pseudo: u.pseudo,
            courriel: u.courriel,
            role,
            cree_le: u.cree_le,
        }
    }
}

// `cargo test` exécute ce module (compilé seulement en mode test grâce à
// `#[cfg(test)]`). Aucune base de données requise ici : on teste la
// logique pure de conversion du rôle.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_role_vers_texte_et_retour() {
        for role in [Role::Admin, Role::Moderateur, Role::Utilisateur] {
            assert_eq!(Role::from_str(role.as_str()), role);
        }
    }

    #[test]
    fn role_inconnu_retombe_sur_utilisateur() {
        assert_eq!(Role::from_str("valeur-inattendue"), Role::Utilisateur);
    }
}