# Chatvardage Authentification


Service d'authentification et de gestion des comptes pour **ChatVardage**,
écrit en Rust. Ce dépôt correspond à la portion « authentification »
(courriel/pseudo + mot de passe, rôles) — il fait partie d'une architecture **multiservice** : le
client desktop (dépôt `A26_5A6_01-Frontend`) et un dépôt
`infra` séparé (docker-compose global de l'équipe) vivent ailleurs.

> Convention de nommage : toutes les données (champs JSON, colonnes de
> base de données, champs des structures Rust) sont nommées en français
> (`pseudo`, `courriel`, `jeton`, `cree_le`...). Les mots-clés techniques
> propres à un standard externe (`sub`/`exp` du JWT) ou à un patron
> d'architecture largement utilisé en anglais même dans du code français
> (`Service`, `Repository`, `Controller`, `Request`/`Response` — voir
> `Architecture.md` du frontend, qui fait pareil) restent en anglais. Les
> routes HTTP (`/api/auth/register`...) restent en anglais aussi, comme
> c'est l'usage pour des URL d'API REST.

## Sommaire

- [Objectif de ce service](#objectif-de-ce-service)
- [Choix techniques](#choix-techniques-et-justification)
- [Architecture du service](#architecture-du-service)
- [Migrations de la base de données (Prisma)](#migrations-de-la-base-de-données-prisma)
- [Prérequis](#prérequis)
- [Installation et lancement](#installation-et-lancement)
- [Variables d'environnement](#variables-denvironnement)
- [Vérifier que ça fonctionne](#vérifier-que-ça-fonctionne)
- [Lancer les tests](#lancer-les-tests)
- [Endpoints de l'API](#endpoints-de-lapi)
- [Créer un premier compte admin](#créer-un-premier-compte-admin)
- [Intégration avec le reste de l'équipe](#intégration-avec-le-reste-de-léquipe)
- [Versions (tags Git)](#versions-tags-git)
- [Limites connues et pistes d'amélioration](#limites-connues-et-pistes-damélioration)

## Objectif de ce service

Fournit une API REST pour :

- l'inscription et la connexion (courriel **ou** pseudo + mot de passe) ;
- l'émission de jetons JWT utilisés ensuite par le client desktop pour
  s'authentifier auprès des autres services (messages privés, etc.) ;
- la gestion des comptes et des rôles (`admin`, `moderateur`,
  `utilisateur`) pour le panneau d'administration de l'équipe.

La persistance des comptes se fait dans PostgreSQL (source de vérité, comme
exigé par l'énoncé). Ce service n'a pas besoin de Redis : Redis sert au
service de messagerie temps réel pour diffuser les événements, pas à
l'authentification elle-même.

**Ce service possède et gère la table `utilisateurs`** : le schéma est
défini et versionné avec des migrations Prisma dans ce dépôt (voir
[Migrations de la base de données (Prisma)](#migrations-de-la-base-de-données-prisma)).
Prisma sert uniquement à définir le schéma et à exécuter les migrations —
l'API elle-même continue d'interroger PostgreSQL avec SQLx au runtime (voir
[Choix techniques](#choix-techniques-et-justification)) ; aucun client
Prisma n'est utilisé dans le code Rust. Il n'y a **pas de mode mock/en
mémoire** : une base PostgreSQL migrée est requise dès le développement
local (voir [Migrations de la base de données (Prisma)](#migrations-de-la-base-de-données-prisma)
et [Installation et lancement](#installation-et-lancement)).

## Choix techniques et justification

| Choix | Pourquoi |
|-------|----------|
| **Rust** | Imposé par la répartition des tâches de l'équipe (voir énoncé : "vous êtes libres de choisir les technologies"). |
| **Axum** | Framework web asynchrone, API simple, bonne documentation, très utilisé pour des services HTTP en Rust — un bon choix pour un premier projet Rust. |
| **PostgreSQL + SQLx** | Base relationnelle robuste pour des données structurées (comptes, rôles). SQLx est asynchrone (compatible avec Axum/Tokio) et n'exige pas d'ORM lourd. |
| **Prisma (migrations uniquement)** | Standardise la gestion des migrations de schéma avec le reste de l'équipe (le Room-Service l'utilise aussi) — la CLI Prisma est indépendante du langage du service qui l'utilise. Aucun client Prisma généré n'est utilisé au runtime : SQLx reste le seul point d'accès aux données depuis le code Rust. |
| **Port** (`UtilisateurRepositoryPort`) | Le code métier ne dépend que d'une interface (`domain::ports::UtilisateurRepositoryPort`), pas directement de PostgreSQL — même principe que `ChatGateway` côté frontend. Une seule implémentation en production, `PgUtilisateurRepository` (SQLx) ; il n'y a pas de mode mock exposé à l'exécution. Un second dépôt en mémoire (`FakeUtilisateurRepository`) existe, mais uniquement compilé en mode test (`#[cfg(test)]`) pour les tests HTTP de bout en bout — voir [Lancer les tests](#lancer-les-tests). |
| **Argon2id** | Algorithme de hachage de mot de passe recommandé actuellement (résiste mieux aux attaques par GPU que bcrypt/MD5/SHA simple), calibré explicitement à 19 Mio, 2 itérations et 1 voie. Ses calculs CPU-bound sont exécutés dans le pool bloquant de Tokio afin de préserver les workers HTTP. |
| **JWT (jsonwebtoken)** | Chaque requête porte un jeton signé, dont la signature et l'expiration sont vérifiées avant de relire l'utilisateur en base. Le rôle ne fait pas partie du jeton et doit toujours être lu depuis ce service; une suppression ou un changement de rôle prend donc effet immédiatement, sans stockage de session. |
| **Docker multi-stage** | Une étape compile le binaire Rust avec `rust:1.88.0-slim-bookworm`, une autre installe la CLI Prisma avec `node:22-bookworm-slim` (Node.js 22, comme les autres services ; Prisma figé à la version exacte) et exécute `npx prisma generate` ; l'image finale embarque le binaire Rust compilé, Node.js et `prisma/` pour exécuter `npx prisma migrate deploy` au démarrage avant de lancer le binaire — même séquence que les microservices Node de l'équipe (voir [Installation et lancement](#installation-et-lancement)), adaptée à un runtime Rust. |

## Architecture du service

Le code suit les mêmes couches que `Architecture.md` du frontend, pour
rester cohérent avec le reste de l'équipe :

```
src/
├── interface/http/     # Couche HTTP : routes, contrôleurs, extracteur d'auth
│   ├── routes.rs        #   déclare toutes les routes
│   ├── auth_controller.rs
│   ├── utilisateurs_controller.rs
│   └── middleware.rs     #   lit et valide le JWT (Authorization: Bearer ...)
├── application/         # Cas d'utilisation (inscrire, connecter) + DTO
│   ├── auth_service.rs
│   └── dto.rs
├── domain/              # Entités et RÈGLES métier — aucune dépendance HTTP/DB
│   ├── utilisateur.rs    #   entités Utilisateur, Role
│   └── ports.rs           #   UtilisateurRepositoryPort : ce dont le métier a
│                           #   besoin (pas comment c'est implémenté — voir
│                           #   infrastructure/)
├── infrastructure/      # Implémentations techniques : Postgres, Argon2, JWT
│   ├── db.rs
│   ├── pg_utilisateur_repository.rs    # implémente le port avec PostgreSQL
│   ├── mot_de_passe.rs
│   └── jwt.rs
├── config.rs            # Lecture des variables d'environnement
├── state.rs             # État partagé injecté dans les contrôleurs
└── main.rs               # Point d'entrée : assemble PgUtilisateurRepository
                           #   et les services, démarre le serveur
```

Schéma simplifié du flux d'une requête protégée :

```
Client (desktop)
   │  Authorization: Bearer <jeton>
   ▼
interface/http (route + middleware : JWT validé, utilisateur relu en base)
   ▼
application (auth_service : logique métier)
   ▼
domain::ports::UtilisateurRepositoryPort (interface)
   ▼
infrastructure::PgUtilisateurRepository
   ▼
   PostgreSQL (schéma géré par les migrations Prisma de ce dépôt)
```

Un schéma d'architecture **global** (client desktop + ce service + les
autres services + conteneurs) est de la responsabilité collective de
l'équipe et doit vivre dans le dépôt `infra` — ce README documente
uniquement l'architecture interne de ce service.

## Migrations de la base de données (Prisma)

Ce service possède la source de vérité du schéma de la table
`utilisateurs`, défini dans [`prisma/schema.prisma`](prisma/schema.prisma)
et versionné avec des migrations dans `prisma/migrations/` — même
mécanisme que le Room-Service de l'équipe (voir son `AGENTS.md`). Prisma
ne sert **qu'à** définir le schéma et exécuter les migrations : le code
Rust (`src/infrastructure/pg_utilisateur_repository.rs`) continue
d'utiliser SQLx pour lire/écrire dans PostgreSQL — aucun client Prisma
n'est appelé au runtime.

Dans l'environnement d'équipe (dépôt `infra`), la base et l'utilisateur
PostgreSQL de ce service (`chatvardage_auth` / `auth_service`) sont
provisionnés par `infra` lui-même — voir le guide Docker/Prisma de
l'équipe. `infra` crée la base et le rôle, **mais pas les tables** :
c'est `npx prisma migrate deploy`, exécuté par **ce** dépôt au démarrage
de son conteneur, qui crée la table `utilisateurs`.

```prisma
model Utilisateur {
  id                 String   @id @default(dbgenerated("gen_random_uuid()")) @db.Uuid
  pseudo             String   @unique
  courriel           String   @unique
  mot_de_passe_hache String
  version_auth       Int      @default(0)
  role               String   @default("utilisateur")
  cree_le            DateTime @default(now()) @db.Timestamptz
  modifie_le         DateTime @default(now()) @db.Timestamptz

  @@map("utilisateurs")
}
```

Deux détails délibérément différents des exemples Prisma habituels (voir
les commentaires dans `prisma/schema.prisma` pour le détail) :

- `dbgenerated("gen_random_uuid()")`, pas `@default(uuid())` : ce dernier
  génère l'UUID côté client Prisma — jamais appelé au runtime ici — donc
  la colonne n'aurait aucune valeur par défaut côté PostgreSQL et
  l'`INSERT` de SQLx (qui ne fournit pas `id`) échouerait. `gen_random_uuid()`
  est une fonction native de PostgreSQL depuis la version 13.
- `@db.Timestamptz` sur les deux colonnes de date : le code Rust les lit
  en `chrono::DateTime<Utc>` (type PostgreSQL `timestamptz`), pas
  `NaiveDateTime` (`timestamp`, le défaut Prisma sans cette annotation) —
  sans elle, SQLx échouerait au décodage à l'exécution. Pas de
  `@updatedAt` sur `modifie_le` non plus, pour la même raison que
  `@default(uuid())` : c'est le client Prisma qui l'appliquerait, jamais
  utilisé ici — `pg_utilisateur_repository.rs` met à jour cette colonne
  explicitement (`SET modifie_le = now()`) à chaque `UPDATE`.

| Colonne | Type | Contraintes |
|---------|------|-------------|
| `id` | UUID | clé primaire |
| `pseudo` | texte | unique, non nul |
| `courriel` | texte | unique, non nul |
| `mot_de_passe_hache` | texte | non nul (hash Argon2, jamais le mot de passe en clair) |
| `version_auth` | entier | non nul, incrémenté à chaque changement de mot de passe ou de rôle pour révoquer les JWT antérieurs |
| `role` | texte | non nul, valeurs attendues : `admin`, `moderateur`, `utilisateur` (défaut : `utilisateur`) |
| `cree_le` | horodatage avec fuseau | non nul |
| `modifie_le` | horodatage avec fuseau | non nul |

Ces colonnes doivent rester synchronisées avec les requêtes SQL de
`src/infrastructure/pg_utilisateur_repository.rs` : toute modification du
schéma Prisma doit être répercutée dans ce fichier (et vice-versa), Prisma
ne générant pas le code Rust.

Après une modification de `prisma/schema.prisma` :

```bash
npx prisma generate
npx prisma migrate dev --name <nom_de_la_migration>
```

Ne jamais modifier une migration déjà appliquée pour masquer un
changement — créer une nouvelle migration.

Une migration (`index_courriel_insensible_a_la_casse`) remplace l'index
`UNIQUE` standard sur `courriel` par un index fonctionnel sur
`LOWER(courriel)` — défense en profondeur pour que PostgreSQL fasse
respecter la même règle de casse que `trouver_par_identifiant` (voir
AGENTS.md, section « Base de données et Prisma », pour le piège Prisma
associé : cette contrainte n'est plus modélisable par la seule annotation
`@unique` de `schema.prisma`).

Dans Docker/déploiement, appliquer les migrations avec :

```bash
npx prisma migrate deploy
```

⚠️ Prisma n'est utilisé ici que comme outil de migration (CLI Node.js) —
il ne remplace pas SQLx et n'est pas une dépendance du binaire Rust.

## Prérequis

> **Note sur `zmij` dans `Cargo.lock` :** la commande
> `cargo tree --invert zmij` confirme que `zmij v1.0.23` est une dépendance
> transitive de `serde_json v1.0.151`, résolue et vérifiée par Cargo avec la
> somme de contrôle du lockfile. Son entrée est générée par Cargo; ne jamais
> modifier le lockfile manuellement.

**Rien à installer localement pour faire tourner le service** à part
Docker (conforme à la consigne du contrôle) — les migrations Prisma
peuvent aussi être appliquées dans un conteneur (voir
[Migrations de la base de données (Prisma)](#migrations-de-la-base-de-données-prisma)).
Pour développer/apprendre en dehors de Docker, ou pour créer/modifier une
migration Prisma en local, il faut en plus :

- [Rust](https://www.rust-lang.org/tools/install), **version 1.88 ou plus
  récente** — installe `cargo` et `rustc`. Vérifier avec `rustc --version` ;
  si c'est plus vieux, lancer `rustup update stable` (voir dépannage
  ci-dessous si `rustup` n'est pas installé).
- [Node.js](https://nodejs.org/) (version LTS) et npm — pour développer
  ou modifier une migration en local (`npx prisma migrate dev`). **Pas
  requis pour lancer le service avec Docker** : l'image finale embarque
  déjà Node.js et exécute `npx prisma migrate deploy` elle-même au
  démarrage du conteneur (voir Dockerfile) ; le binaire Rust lui-même
  n'appelle jamais de client Prisma, seul le CLI (Node.js) sert à migrer.
- [Docker](https://www.docker.com/) et Docker Compose (v2, la commande
  `docker compose`, pas `docker-compose`).

> **Pourquoi 1.88 précisément ?** Ce projet est écrit pour l'édition 2021
> (rien à voir avec ça), mais certaines dépendances *transitives* (des
> crates utilisées par nos propres dépendances, pas ajoutées par nous —
> ici `icu_provider`, tirée par la crate `url`) exigent depuis peu
> l'édition Cargo `edition2024`, stabilisée seulement à partir de Rust
> 1.85. Avec une version plus ancienne, `cargo build`/`cargo run` échoue
> avec une erreur du type `feature 'edition2024' is required`. Le
> `Dockerfile` utilise l'image précisément versionnée
> `rust:1.88.0-slim-bookworm`, qui satisfait ce prérequis et rend les builds
> Docker reproductibles. C'est seulement en local, hors Docker, que ta
> version installée compte.

## Installation et lancement

### Avec Docker (méthode utilisée pour la remise)

```bash
git clone <url-de-ce-dépôt>
cd A26_5A6_01-Authentification
cp .env.example .env
# Générer puis renseigner JWT_SECRET dans .env (64 caractères)
openssl rand -hex 32
docker compose up --build
```

Pour appeler l'API depuis un navigateur, renseigner
`CORS_ORIGINS` dans `.env` avant le démarrage (par exemple
`CORS_ORIGINS=http://localhost:5173`). Sans cette liste blanche,
le service refuse les requêtes cross-origin.

Ce `docker-compose.yml` sert à développer/tester **ce service seul**, avec
sa propre base PostgreSQL de développement. Il démarre le conteneur
`auth-api` sur `http://localhost:3001` en trois étapes automatiques (voir
`Dockerfile`) :

1. build de l'image : compilation du binaire Rust **et** `npx prisma generate` ;
2. démarrage du conteneur : `npx prisma migrate deploy` (crée/met à jour la table `utilisateurs`) ;
3. lancement du binaire `chatvardage-auth`.

Aucune installation manuelle de PostgreSQL ni commande Prisma séparée
n'est nécessaire pour cette utilisation en solo.

Dans l'environnement d'équipe complet (dépôt `infra`), ce service n'a pas
son propre PostgreSQL : `infra` démarre un PostgreSQL **partagé** (service
Docker `postgres`) et crée la base `chatvardage_auth` + l'utilisateur
`auth_service`. Ce service est alors lancé comme un bloc du
`docker-compose.yml` d'`infra`, avec un `DATABASE_URL` du type :

```
postgresql://auth_service:auth_service_dev@postgres:5432/chatvardage_auth
```

⚠️ À l'intérieur du réseau Docker d'`infra`, l'hôte est `postgres` (le nom
du service Docker), **jamais** `localhost` — `localhost` dans le conteneur
de ce service désigne le conteneur lui-même, pas la base de données. Voir
[Intégration avec le reste de l'équipe](#intégration-avec-le-reste-de-léquipe).

Pour arrêter : `Ctrl+C` puis `docker compose down` (ajouter `-v` pour
aussi supprimer les données PostgreSQL locales — irréversible).

```
$ docker compose ps
NAME                   STATUS
chatvardage-auth-api   Up (healthy)
chatvardage-auth-db    Up (healthy)

$ docker exec chatvardage-auth-db pg_isready -U chatvardage -d chatvardage_auth
/var/run/postgresql:5432 - accepting connections

$ curl http://localhost:3001/health
ok
```

Le conteneur `auth-api` applique la migration Prisma automatiquement au
démarrage (`Applying migration ... All migrations have been successfully
applied.` dans `docker compose logs auth-api` au premier démarrage ;
`No pending migrations to apply.` aux démarrages suivants — les données
survivent aux redémarrages grâce au volume nommé `auth-db-data`).

#### Dépannage : `feature 'edition2024' is required`

Cette erreur veut dire que ton `rustc` est trop ancien (voir
[Prérequis](#prérequis) plus haut) — c'est fréquent quand Rust a été
installé via le gestionnaire de paquets du système (`apt install cargo`
sur Debian/Ubuntu, par exemple) plutôt que via l'installeur officiel, car
ces paquets système sont souvent en retard de plusieurs versions.

- Si Rust a été installé via [rustup](https://rustup.rs/) (la méthode
  recommandée) : `rustup update stable`, puis relancer `cargo run`.
- Si `rustup` n'est pas installé (erreur "command not found") : Rust a
  probablement été installé via `apt`/le gestionnaire du système. Le plus
  simple est d'installer rustup par-dessus (il prendra le dessus sur le
  Rust système) via le script officiel sur
  [rust-lang.org/tools/install](https://www.rust-lang.org/tools/install),
  puis d'ouvrir un nouveau terminal et de vérifier `rustc --version`
  (doit afficher 1.88 ou plus).

## Variables d'environnement

Voir `.env.example` pour la liste complète et les valeurs par défaut.
Résumé :

| Variable | Rôle |
|----------|------|
| `DATABASE_URL` | Chaîne de connexion PostgreSQL — **toujours requise**, aucune valeur par défaut (lue à la fois par Prisma pour les migrations et par SQLx au runtime). Format : `postgresql://<user>:<mot_de_passe>@<hôte>:5432/<base>`. Ex. local : `postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth` ; dans le réseau Docker d'`infra` : `postgresql://auth_service:auth_service_dev@postgres:5432/chatvardage_auth`. Le suffixe `?schema=public` (convention Prisma) est **volontairement omis** : testé et accepté par SQLx, qui l'ignore avec un simple avertissement (`ignoring unrecognized connect parameter`) plutôt qu'une erreur — inutile de toute façon puisque `public` est déjà le schéma par défaut de PostgreSQL. |
| `JWT_SECRET` | Secret de signature des JWT — **toujours requis, au moins 32 caractères (256 bits)** (le service refuse de démarrer sinon, y compris si la variable existe mais est vide — voir `config.rs`). Générer avec `openssl rand -hex 32` (64 caractères). **À changer avant la remise finale**, ne jamais committer un vrai secret. |
| `JWT_DUREE_DE_VIE_HEURES` | Durée de validité d'un jeton en heures (défaut : 24h, doit être comprise entre 1 et 720 — 30 jours). Le rôle et l'existence de l'utilisateur sont revérifiés en base à chaque route protégée, donc une durée élevée ne conserve pas les droits d'un compte supprimé ou rétrogradé. |
| `PORT` | Port HTTP du service et du mapping du Compose autonome (défaut : `3001`). Avec `PORT=3099`, l'API est exposée sur `http://localhost:3099` et son healthcheck interroge ce même port. |
| `RUST_LOG` | Niveau de journalisation (`tracing-subscriber`) — `error`/`warn`/`info`/`debug`/`trace` (défaut : `info`) |
| `SERVICES_COMPTE_SUPPRIME_URLS` | URL de base des services à prévenir d'une suppression de compte, séparées par des virgules (ex. `http://room-service:3003,http://message-service:3004,http://user-config-service:3005`). Absente : aucun service prévenu (avertissement journalisé à chaque suppression). |
| `INTERNAL_SERVICE_TOKEN` | Secret partagé entre microservices, envoyé dans `x-internal-token` à ces services. Même valeur que dans l'Infra. |
| `CORS_ORIGINS` | Liste blanche CORS, origines séparées par des virgules (ex. `http://localhost:5173,https://admin.exemple.com`). Absente, vide ou invalide : aucune origine n'est autorisée. L'infrastructure doit fournir les origines réellement utilisées par l'application Electron et le panneau d'administration. |

### En production (`NODE_ENV=production`)

Même variable que les services Node. Le service **refuse de démarrer** si `JWT_SECRET` vaut la valeur de développement de l'Infra (`dev_secret_change_me_before_prod`, qui fait pourtant 32 caractères et passerait la règle de longueur) ou si le mot de passe de `DATABASE_URL` est une valeur de développement (`chatvardage`, `auth_service_dev`, `postgres`) — voir `config.rs`, `problemes_secrets_production`. Si `SERVICES_COMPTE_SUPPRIME_URLS` est renseignée, `INTERNAL_SERVICE_TOKEN` doit aussi être présent, d'au moins 32 caractères et différent de la valeur de développement (`chatvardage_internal_dev`). En développement, rien ne change.

## Vérifier que ça fonctionne

Dans le Compose autonome, `DATABASE_URL_DOCKER` est l'URI complète utilisée
par `auth-api`; elle doit employer l'hôte `postgres`. Les caractères réservés
dans un nom ou mot de passe (`@`, `:`, `/`, `?`, `#`, etc.) doivent être
encodés en pourcentage dans cette URI, afin qu'ils ne soient jamais
interprétés comme de la syntaxe d'URL.

```bash
# Santé du service (confirme aussi indirectement que la connexion à
# PostgreSQL a réussi — le service ne démarre pas sinon)
curl http://localhost:3001/health

# Inscription
curl -X POST http://localhost:3001/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"pseudo":"alice","courriel":"alice@example.com","mot_de_passe":"motdepasse123"}'

# Connexion (avec le pseudo OU le courriel)
curl -X POST http://localhost:3001/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"identifiant":"alice","mot_de_passe":"motdepasse123"}'

# Route protégée (remplacer <JETON> par le jeton reçu ci-dessus)
curl http://localhost:3001/api/auth/me -H "Authorization: Bearer <JETON>"
```

Les commandes ci-dessus permettent de vérifier le démarrage, l'inscription,
la connexion et la consultation du profil sur une installation locale.

## Lancer les tests

Comme le service lui-même, les tests n'ont **pas de mode mock** pour la
base de données : `infrastructure::pg_utilisateur_repository` teste le
vrai SQL contre une vraie base PostgreSQL migrée (les autres couches —
contrôleurs, middleware, service applicatif — utilisent, elles, un dépôt
en mémoire écrit spécifiquement pour les tests, jamais compilé dans le
binaire de production : voir `infrastructure::fake_utilisateur_repository`,
`#[cfg(test)]`).

```bash
# 1. Démarrer uniquement PostgreSQL (sans l'API)
docker compose up -d postgres

# 2. Appliquer les migrations sur cette base
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth \
  npx prisma migrate deploy

# 3. Lancer les tests avec la même variable d'environnement
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth \
  cargo test
```

Couverture (voir aussi `AGENTS.md`) :

```bash
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth \
  cargo llvm-cov --ignore-filename-regex 'main\.rs' --fail-under-lines 99
```

`main.rs` est exclu du calcul : c'est l'assemblage final (charge la
config, ouvre le pool PostgreSQL, démarre le serveur) — un point d'entrée
d'intégration vérifié manuellement (voir plus haut, `docker compose up`)
plutôt que par des tests unitaires, comme c'est l'usage pour ce genre de
fonction `main` dans l'écosystème Rust. Le reste du code atteint 100 % de
couverture des lignes à une exception près, documentée directement dans
le code : le chemin d'erreur de `jsonwebtoken::encode` (signature JWT) et
celui d'`Argon2::hash_password` sont défensifs — aucune entrée valide et
raisonnable ne les déclenche, les forcer artificiellement en test
exigerait d'injecter une fausse implémentation uniquement pour gonfler ce
chiffre, ce qui n'apporterait aucune garantie réelle.

## Endpoints de l'API

Documentation complète (requêtes, réponses, codes d'erreur) dans
[`docs/API.md`](docs/API.md). Résumé :

| Méthode | Route | Protégée | Description |
|---------|-------|----------|--------------|
| GET | `/health` | non | Vérification de santé |
| POST | `/api/auth/register` | non | Créer un compte |
| POST | `/api/auth/login` | non | Se connecter (courriel ou pseudo) |
| GET | `/api/auth/me` | oui | Utilisateur courant |
| PATCH | `/api/auth/me` | oui | Modifier son propre profil (pseudo/mot de passe ; le courriel n'est pas modifiable) |
| DELETE | `/api/auth/me` | oui | Supprimer son propre compte (les autres services sont prévenus) |
| POST | `/api/auth/logout-all` | oui | Se déconnecter partout (invalide tous ses jetons) |
| GET | `/api/users/lookup` | oui (tout utilisateur, pas seulement admin) | Résoudre `{id -> pseudo}` en lot pour d'autres comptes |
| GET | `/api/users` | admin | Lister les comptes |
| POST | `/api/users` | admin | Créer un compte |
| GET | `/api/users/:id` | admin | Consulter un compte |
| PATCH | `/api/users/:id` | admin | Modifier le pseudo/courriel d'un compte |
| DELETE | `/api/users/:id` | admin | Supprimer un compte |
| PATCH | `/api/users/:id/role` | admin | Changer le rôle d'un compte |

## Créer un premier compte admin

Aucun compte n'est admin par défaut : `POST /api/auth/register` attribue
toujours le rôle `utilisateur` (par sécurité — sinon n'importe qui
pourrait s'inscrire directement comme admin). Il n'existe pas non plus de
route pour s'auto-promouvoir : `PATCH /api/users/{id}/role` est réservée
aux admins, ce qui crée un problème de démarrage (« œuf et poule ») pour
le tout premier admin. Marche à suivre :

1. **S'inscrire normalement** via l'API :

   ```bash
   curl -X POST http://localhost:3001/api/auth/register \
     -H "Content-Type: application/json" \
     -d '{"pseudo":"tonpseudo","courriel":"toi@example.com","mot_de_passe":"motdepasse123"}'
   ```

2. **Promouvoir ce compte manuellement en base** — la seule étape qui
   sort de l'API, volontairement (voir plus haut) :

   ```bash
docker exec chatvardage-postgres \
  psql -U postgres -d chatvardage_auth \
  -c "UPDATE utilisateurs SET role='admin' WHERE pseudo='tonpseudo';"
   ```

3. **Utiliser le jeton existant ou se reconnecter.** Le middleware relit
   le rôle en base à chaque route protégée : la promotion est effective
   immédiatement, même avec un jeton émis avant le changement. Une
   reconnexion reste utile pour renouveler le jeton avant son expiration.

   ```bash
   curl -X POST http://localhost:3001/api/auth/login \
     -H "Content-Type: application/json" \
     -d '{"identifiant":"tonpseudo","mot_de_passe":"motdepasse123"}'
   ```

Une fois ce premier admin en place, il peut promouvoir n'importe quel
autre compte (y compris vers `admin`) via `PATCH /api/users/{id}/role` —
voir [Endpoints de l'API](#endpoints-de-lapi) — sans repasser par SQL.

## Limites connues et pistes d'amélioration

- Pas de renouvellement de jeton (refresh token) : un jeton expiré oblige
  à se reconnecter. Acceptable pour la portée du contrôle (24h de durée de
  vie par défaut).
- La limitation de débit de `/api/auth/login` et `/api/auth/register`
  relève du `Serveur-API` : le Gateway, point d'entrée unique, doit
  limiter ces routes publiques avant qu'elles ne déclenchent un hachage
  Argon2 coûteux. Une limite locale ne protégerait pas les ressources du
  Gateway et ne serait pas partagée entre plusieurs instances du service.
- Le tout premier compte admin doit être créé manuellement en base de
  données (voir [Créer un premier compte admin](#créer-un-premier-compte-admin)).
  Une commande CLI ou un script de "seed" (ex. variable d'environnement
  `ADMIN_BOOTSTRAP_PSEUDO` appliquée au démarrage) pourrait remplacer
  cette étape manuelle dans une prochaine itération.
- Aucune limite sur le nombre d'admins ni garde-fou du genre « il doit
  rester au moins un admin ». En revanche, un admin ne peut changer ni son
  propre rôle ni celui d'un autre admin : seules les cibles non-admin sont
  modifiables via `PATCH /api/users/{id}/role`.
- **Énumération de comptes possible via `POST /api/auth/register`** :
  `connecter` investit un effort réel pour empêcher l'énumération à la
  connexion (hachage factice à temps constant, message générique — voir
  `AuthService::connecter`), mais `inscrire` renvoie un `409` explicite si
  le pseudo ou le courriel existe déjà, ce qui permet de tester si un
  courriel est déjà enregistré en essayant de s'y inscrire. C'est un
  compromis courant (accepté par beaucoup de services pour l'UX à
  l'inscription — un message "courriel déjà utilisé" immédiat plutôt
  qu'un envoi de courriel de vérification systématique), assumé
  consciemment ici plutôt que corrigé, faute d'un flux de vérification par
  courriel qui serait hors de la portée de ce contrôle.

### Vérifications à exécuter avant une remise

- `cargo build`, `cargo build --release`, `cargo fmt --check`,
  `cargo clippy --all-targets --all-features -- -D warnings` et
  `cargo test` passent tous sans erreur ni avertissement ;
- `docker compose up --build` démarre PostgreSQL et l'API depuis une
  installation propre, applique la migration Prisma automatiquement, et
  le scénario complet (inscription, connexion, `/me`, persistance après
  redémarrage du conteneur) a été rejoué avec succès (voir
  [Installation et lancement](#installation-et-lancement) et
  [Vérifier que ça fonctionne](#vérifier-que-ça-fonctionne) pour le détail) ;
- aucune macro `query!` de SQLx n'est utilisée (l'API "query builder" à
  l'exécution est employée partout), donc aucune base de données n'est
  requise au moment de la compilation elle-même — seulement à
  l'exécution et pour les tests de `pg_utilisateur_repository` (voir
  [Lancer les tests](#lancer-les-tests)).
