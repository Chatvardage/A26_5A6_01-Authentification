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

## Permissions — qui peut faire quoi

Trois rôles existent (`admin`, `moderateur`, `utilisateur` — voir
`src/domain/utilisateur.rs`). Un `utilisateur` et un `moderateur` ont
exactement les mêmes droits **sur ce service** : gérer leur propre compte,
rien d'autre. Le rôle `moderateur` donne des droits supplémentaires
ailleurs dans l'architecture (modération des salons/messages), pas ici —
à confirmer/ajuster avec l'équipe si le panneau d'administration doit
finalement distinguer les deux.

| Action | `utilisateur` / `moderateur` | `admin` |
|--------|:---:|:---:|
| Créer un compte (inscription) | ✅ (le sien, `/register`) | ✅ (le sien, `/register`) |
| Lire son propre profil | ✅ `GET /api/auth/me` | ✅ `GET /api/auth/me` |
| Lire le profil d'un autre utilisateur | ❌ | ✅ `GET /api/users/{id}` |
| Lister tous les comptes | ❌ | ✅ `GET /api/users` |
| Modifier son propre profil (pseudo/courriel/mot de passe) | ✅ `PATCH /api/auth/me` | ✅ `PATCH /api/auth/me` |
| Modifier le profil d'un autre utilisateur | ❌ | ✅ `PATCH /api/users/{id}` |
| Changer le rôle d'un compte | ❌ (pas même le sien) | ✅ `PATCH /api/users/{id}/role` |
| Supprimer son propre compte | ✅ `DELETE /api/auth/me` | ✅ `DELETE /api/auth/me` |
| Supprimer le compte d'un autre utilisateur | ❌ | ✅ `DELETE /api/users/{id}` |

Un utilisateur ne peut jamais changer son propre rôle (ni celui de
personne d'autre) — seul un `admin`, via `PATCH /api/users/{id}/role`,
le peut. C'est volontaire : sinon n'importe quel compte pourrait
s'auto-promouvoir `admin`.

---

## Format des erreurs

Toute erreur renvoie un corps de la forme :

```json
{ "erreur": "description lisible du problème" }
```

Cette enveloppe est garantie **sans exception**, y compris pour un JSON
syntaxiquement invalide, un champ manquant ou du mauvais type (ex.
`{"role": "superadmin"}`, qui n'existe pas dans l'énumération) ou un
segment d'URL qui ne parse pas (ex. un `id` qui n'est pas un UUID) : les
handlers utilisent des extracteurs `Json`/`Path` personnalisés
(`src/interface/http/extracteurs.rs`) qui reformatent toute erreur
d'extraction axum vers cette même enveloppe, plutôt que le corps en texte
brut renvoyé par défaut par `axum::extract::Json`/`Path`.

Pour une erreur de validation `validator` (`application/dto.rs`), le
message a la forme `"<champ>: <message>"` (ex. `"pseudo: le pseudo doit
contenir entre 3 et 20 caractères"`), en utilisant le `message`
personnalisé déclaré sur chaque validateur — jamais le format technique
brut de la crate `validator` (noms de types Rust, codes internes). Testé
explicitement dans `interface::http::routes` (pas seulement le code de
statut) pour éviter qu'un message oublié sur un futur validateur ne
dégrade silencieusement ce contrat.

| Code | Signification |
|------|----------------|
| 400  | Requête invalide (validation échouée : champ manquant, format incorrect, JSON malformé) |
| 401  | Non authentifié (jeton manquant, invalide ou expiré) / identifiants incorrects |
| 403  | Authentifié, mais rôle insuffisant (route admin) |
| 404  | Ressource introuvable |
| 409  | Conflit (pseudo ou courriel déjà utilisé) |
| 413  | Corps de la requête trop volumineux (plus de 16 Kio, voir `interface::http::routes`) |
| 500  | Erreur interne du serveur |

---

## GET /health

Vérifie que le service répond (utilisé par le `HEALTHCHECK` Docker et par
docker-compose).

**Réponse `200`** : `ok` (texte brut)

---

## Authentification et compte personnel (`/api/auth`)

Ces routes couvrent la création et la gestion **de son propre compte** —
accessibles à tous les rôles de la même façon (voir
[Permissions](#permissions--qui-peut-faire-quoi)).

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
| pseudo | 3 à 20 caractères, ne peut pas contenir `@` (réservé aux courriels — sinon un pseudo comme `alice@x.co` pourrait être confondu avec un courriel à la connexion) |
| courriel | format courriel valide, 254 caractères maximum (RFC 5321) ; normalisé en minuscules avant stockage |
| mot_de_passe | 8 à 128 caractères |

**Réponse `200`**

```json
{
  "jeton": "eyJhbGciOi...",
  "utilisateur": {
    "id": "b3f1...",
    "pseudo": "alice",
    "courriel": "alice@example.com",
    "role": "utilisateur",
    "cree_le": "2026-09-10T12:00:00Z",
    "modifie_le": "2026-09-10T12:00:00Z"
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
  "cree_le": "2026-09-10T12:00:00Z",
  "modifie_le": "2026-09-10T12:00:00Z"
}
```

**Erreurs** : `401` (jeton manquant, invalide ou expiré), `404` (le
compte du jeton n'existe plus — voir note sur `DELETE /api/auth/me`
ci-dessous).

---

### PATCH /api/auth/me 🔒

Modifie son **propre** compte (pseudo, courriel et/ou mot de passe).
Accessible à tous les rôles — un utilisateur ou modérateur peut ainsi
mettre à jour son profil sans passer par un admin. Ne permet pas de
changer son propre rôle (voir `PATCH /api/users/{id}/role`, réservé aux
admins).

⚠️ **`mot_de_passe_actuel` est requis dès que `courriel` et/ou
`mot_de_passe` sont fournis** (pas pour un changement de `pseudo` seul) :
sans cette confirmation, un jeton volé suffirait à changer le mot de
passe ou le courriel du compte et à en prendre le contrôle de façon
permanente, bien après l'expiration normale du jeton.

**Corps de la requête** (tous les champs optionnels, au moins un requis)

```json
{
  "pseudo": "alice2",
  "courriel": "alice2@example.com",
  "mot_de_passe": "nouveaumotdepasse123",
  "mot_de_passe_actuel": "motdepasse123"
}
```

| Champ | Règle |
|-------|-------|
| pseudo | optionnel, 3 à 20 caractères si fourni, ne peut pas contenir `@` |
| courriel | optionnel, format courriel valide et 254 caractères maximum si fourni ; normalisé en minuscules |
| mot_de_passe | optionnel, 8 à 128 caractères si fourni (rehaché avec Argon2) |
| mot_de_passe_actuel | requis si `courriel` et/ou `mot_de_passe` sont fournis (ignoré sinon), 1 à 128 caractères |

**Réponse `200`** : l'objet utilisateur mis à jour (même forme que `GET /api/auth/me`).

**Erreurs** : `400` (validation, aucun champ fourni, ou `mot_de_passe_actuel` manquant alors que `courriel`/`mot_de_passe` sont fournis), `401` (jeton manquant/invalide, ou `mot_de_passe_actuel` incorrect), `404` (compte déjà supprimé), `409` (pseudo ou courriel déjà pris par un autre compte).

---

### DELETE /api/auth/me 🔒

Supprime son **propre** compte. Accessible à tous les rôles, y compris
`admin` (un admin qui veut fermer son propre compte n'a pas besoin d'en
demander un autre — voir `DELETE /api/users/{id}` pour la suppression du
compte de quelqu'un d'autre).

**Réponse `204`** (pas de corps) si supprimé.

**Erreurs** : `401` (jeton manquant/invalide), `404` (compte déjà
supprimé — le jeton reste cryptographiquement valide jusqu'à son
expiration même après suppression du compte, voir README section
« Limites connues »).

---

## Gestion des utilisateurs — panneau d'administration (`/api/users`)

Réservé aux comptes `admin` : gestion de **n'importe quel** compte, pas
seulement le sien (voir [Permissions](#permissions--qui-peut-faire-quoi)).

### GET /api/users 🔒👑

Liste tous les comptes inscrits.

**Réponse `200`** : tableau d'objets utilisateur (même forme que `GET /api/auth/me`).

---

### GET /api/users/{id} 🔒👑

Retourne un compte précis par son id.

**Réponse `200`** : l'objet utilisateur (même forme que `GET /api/auth/me`).

**Erreurs** : `404` si l'id n'existe pas.

---

### PATCH /api/users/{id} 🔒👑

Modifie le pseudo et/ou le courriel d'un autre compte (équivalent
administrateur de `PATCH /api/auth/me`, sans exiger le mot de passe
actuel). Pour changer le mot de passe d'un compte, ou pour changer un
rôle, voir les routes dédiées ci-dessous.

**Corps de la requête** (au moins un champ requis)

```json
{ "pseudo": "alice2", "courriel": "alice2@example.com" }
```

Mêmes règles que `pseudo`/`courriel` sur `PATCH /api/auth/me` (voir plus
haut) : pseudo 3-20 caractères sans `@`, courriel valide et 254
caractères maximum (normalisé en minuscules).

**Réponse `200`** : l'objet utilisateur mis à jour.

**Erreurs** : `400` (validation, ou aucun champ fourni), `404` (id introuvable), `409` (pseudo ou courriel déjà pris).

---

### DELETE /api/users/{id} 🔒👑

Supprime le compte de quelqu'un d'autre.

**Réponse `204`** (pas de corps) si supprimé.

**Erreurs** : `404` si l'id n'existe pas.

---

### PATCH /api/users/{id}/role 🔒👑

Change le rôle d'un utilisateur. Seule route qui permet de modifier un
rôle — aucun utilisateur ne peut changer le sien (voir
[Permissions](#permissions--qui-peut-faire-quoi)).

**Corps de la requête**

```json
{ "role": "moderateur" }
```

Valeurs possibles : `admin`, `moderateur`, `utilisateur`.

**Réponse `200`** : l'objet utilisateur mis à jour.

**Erreurs** : `400` (valeur de rôle invalide), `404` si l'id n'existe pas.

---

## Notes d'implémentation (pour le README / la démonstration)

- Mots de passe hachés avec Argon2 (jamais stockés en clair).
- Sessions sans état : un JWT signé (HMAC) contenant `sub` (id utilisateur —
  nom imposé par la norme JWT, non traduit), `pseudo`, `role` et `exp`
  (expiration — aussi imposé par la norme JWT, 24h par défaut, configurable
  via `JWT_DUREE_DE_VIE_HEURES`).
- `identifiant` de connexion accepte courriel OU pseudo, comme demandé dans
  l'énoncé.
- Le courriel est normalisé en minuscules avant d'être stocké
  (`Alice@Example.com` et `alice@example.com` désignent le même compte,
  et la connexion fonctionne quelle que soit la casse tapée) — c'est
  pourquoi le champ `courriel` d'un objet utilisateur est toujours en
  minuscules dans les réponses, même si une casse différente a été
  fournie à l'inscription ou lors d'une modification. Le `pseudo` reste
  sensible à la casse.
- PostgreSQL est la source de vérité — il n'y a pas de mode mock/en
  mémoire dans ce service. Le schéma de la table `utilisateurs` est géré
  par les migrations Prisma de ce dépôt (voir README, section
  « Migrations de la base de données (Prisma) »).
- Toutes les routes ci-dessus sont codées et testées (voir README,
  section « Lancer les tests ») — y compris `PATCH`/`DELETE /api/auth/me`
  et `GET`/`PATCH /api/users/{id}`.