# API — Service d'authentification ChatVardage

Base URL (développement) : `http://localhost:3001`

Toutes les requêtes/réponses sont en JSON (`Content-Type: application/json`).
Les routes marquées 🔒 exigent l'en-tête :

```
Authorization: Bearer <jeton>
```

Les routes marquées 🔒👑 exigent en plus que l'utilisateur ait le rôle `admin`.

> Note sur les noms utilisés : les champs de données (JSON, base de données)
> sont en français (`pseudo`, `courriel`, `jeton`, etc.). Les routes/URL
> (`/api/auth/register`, `/api/users`...) restent en anglais, comme c'est
> l'usage courant pour les API REST — seule la donnée qu'elles transportent
> est traduite.

---

## Format des erreurs

Toute erreur renvoie un corps de la forme :

```json
{ "erreur": "description lisible du problème" }
```

| Code | Signification |
|------|----------------|
| 400  | Requête invalide (validation échouée : champ manquant, format incorrect) |
| 401  | Non authentifié (jeton manquant, invalide ou expiré) / identifiants incorrects |
| 403  | Authentifié, mais rôle insuffisant (route admin) |
| 404  | Ressource introuvable |
| 409  | Conflit (pseudo ou courriel déjà utilisé) |
| 500  | Erreur interne du serveur |

---

## GET /health

Vérifie que le service répond (utilisé par le `HEALTHCHECK` Docker et par
docker-compose).

**Réponse `200`** : `ok` (texte brut)

---

## Authentification (`/api/auth`)

### POST /api/auth/register

Crée un nouveau compte utilisateur (rôle `utilisateur` par défaut) et
retourne immédiatement un jeton (l'utilisateur est connecté après
inscription, pas besoin d'un second appel à `/login`).

**Corps de la requête**

```json
{
  "pseudo": "alice",
  "courriel": "alice@example.com",
  "mot_de_passe": "motdepasse123"
}
```

| Champ | Règle |
|-------|-------|
| pseudo | 3 à 20 caractères |
| courriel | format courriel valide |
| mot_de_passe | 8 caractères minimum |

**Réponse `200`**

```json
{
  "jeton": "eyJhbGciOi...",
  "utilisateur": {
    "id": "b3f1...",
    "pseudo": "alice",
    "courriel": "alice@example.com",
    "role": "utilisateur",
    "cree_le": "2026-09-10T12:00:00Z"
  }
}
```

**Erreurs** : `400` (validation), `409` (pseudo ou courriel déjà pris).

---

### POST /api/auth/login

Connecte un utilisateur existant. `identifiant` accepte indifféremment le
pseudo ou le courriel.

**Corps de la requête**

```json
{
  "identifiant": "alice",
  "mot_de_passe": "motdepasse123"
}
```

**Réponse `200`** : identique à `/register` (`jeton` + `utilisateur`).

**Erreurs** : `400` (validation), `401` (identifiant ou mot de passe incorrect —
message volontairement générique, sans préciser lequel des deux est en
cause, pour éviter de révéler quels comptes existent).

---

### GET /api/auth/me 🔒

Retourne l'utilisateur correspondant au jeton fourni. Permet au client
desktop de valider un jeton déjà stocké localement au démarrage, sans
redemander le mot de passe.

**Réponse `200`**

```json
{
  "id": "b3f1...",
  "pseudo": "alice",
  "courriel": "alice@example.com",
  "role": "utilisateur",
  "cree_le": "2026-09-10T12:00:00Z"
}
```

**Erreurs** : `401` (jeton manquant, invalide ou expiré).

---

## Gestion des utilisateurs — panneau d'administration (`/api/users`)

### GET /api/users 🔒👑

Liste tous les comptes inscrits.

**Réponse `200`** : tableau d'objets utilisateur (même forme que ci-dessus).

### DELETE /api/users/{id} 🔒👑

Supprime un compte.

**Réponse `204`** (pas de corps) si supprimé.
**Erreurs** : `404` si l'id n'existe pas.

### PATCH /api/users/{id}/role 🔒👑

Change le rôle d'un utilisateur.

**Corps de la requête**

```json
{ "role": "moderateur" }
```

Valeurs possibles : `admin`, `moderateur`, `utilisateur`.

**Réponse `200`** : l'objet utilisateur mis à jour.
**Erreurs** : `404` si l'id n'existe pas.

---

## Notes d'implémentation (pour le README / la démonstration)

- Mots de passe hachés avec Argon2 (jamais stockés en clair).
- Sessions sans état : un JWT signé (HMAC) contenant `sub` (id utilisateur —
  nom imposé par la norme JWT, non traduit), `pseudo`, `role` et `exp`
  (expiration — aussi imposé par la norme JWT, 24h par défaut, configurable
  via `JWT_DUREE_DE_VIE_HEURES`).
- `identifiant` de connexion accepte courriel OU pseudo, comme demandé dans
  l'énoncé.
- La base de données PostgreSQL est la source de vérité en mode réel
  (`USE_MOCK_DB=false`) — voir le schéma attendu dans le README. En mode
  mock (`USE_MOCK_DB=true`, activé par défaut pour développer sans
  attendre le service qui gère les migrations), les mêmes endpoints
  fonctionnent identiquement, mais les comptes vivent en mémoire et sont
  perdus au redémarrage.
