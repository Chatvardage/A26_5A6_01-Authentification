# chatvardage-auth

Service d'authentification et de gestion des comptes pour **ChatVardage**,
écrit en Rust. Ce dépôt correspond à la portion « authentification »
(courriel/pseudo + mot de passe, rôles) du **Contrôle 2** du cours
420-5A6-ST — il fait partie d'une architecture **multiservice** : le
client desktop (dépôt `A26_5A6_01-chatvardage`, Contrôle 1) et un dépôt
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
- [Mode mock (développer sans PostgreSQL)](#mode-mock-développer-sans-postgresql)
- [Schéma de données attendu](#schéma-de-données-attendu)
- [Prérequis](#prérequis)
- [Installation et lancement](#installation-et-lancement)
- [Variables d'environnement](#variables-denvironnement)
- [Vérifier que ça fonctionne](#vérifier-que-ça-fonctionne)
- [Endpoints de l'API](#endpoints-de-lapi)
- [Intégration avec le reste de l'équipe](#intégration-avec-le-reste-de-léquipe)
- [Versions (tags Git)](#versions-tags-git)
- [Limites connues et pistes d'amélioration](#limites-connues-et-pistes-damélioration)
- [Pour aller plus loin (je débute en Rust)](#pour-aller-plus-loin-je-débute-en-rust)

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

**Ce service ne crée pas la table `utilisateurs` lui-même** : un autre
service de l'équipe gère les migrations de la base de données. Voir
[Schéma de données attendu](#schéma-de-données-attendu) pour ce que ce
service suppose déjà présent, et [Mode mock](#mode-mock-développer-sans-postgresql)
pour développer en attendant que ce soit prêt.

## Choix techniques et justification

| Choix | Pourquoi |
|-------|----------|
| **Rust** | Imposé par la répartition des tâches de l'équipe (voir énoncé : "vous êtes libres de choisir les technologies"). |
| **Axum** | Framework web asynchrone, API simple, bonne documentation, très utilisé pour des services HTTP en Rust — un bon choix pour un premier projet Rust. |
| **PostgreSQL + SQLx** | Base relationnelle robuste pour des données structurées (comptes, rôles). SQLx est asynchrone (compatible avec Axum/Tokio) et n'exige pas d'ORM lourd. |
| **Port + Mock** (`UtilisateurRepositoryPort`) | Le code métier ne dépend que d'une interface (`domain::ports::UtilisateurRepositoryPort`), pas directement de PostgreSQL — même principe que `ChatGateway`/`MockGateway` côté frontend. Permet de développer avec un dépôt en mémoire (`USE_MOCK_DB=true`) sans attendre le service qui gère les migrations. |
| **Argon2** | Algorithme de hachage de mot de passe recommandé actuellement (résiste mieux aux attaques par GPU que bcrypt/MD5/SHA simple). |
| **JWT (jsonwebtoken)** | Authentification "sans état" (stateless) : chaque requête porte son propre jeton signé, pas besoin de stocker les sessions côté serveur — plus simple à faire évoluer vers plusieurs instances de l'API. |
| **Docker multi-stage** | L'image finale ne contient que le binaire compilé (quelques Mo), pas la chaîne de compilation Rust (~1,5 Go). |

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
├── infrastructure/      # Implémentations techniques : Postgres, mock, Argon2, JWT
│   ├── db.rs
│   ├── pg_utilisateur_repository.rs    # implémente le port avec PostgreSQL
│   ├── mock_utilisateur_repository.rs  # implémente le port en mémoire
│   ├── mot_de_passe.rs
│   └── jwt.rs
├── config.rs            # Lecture des variables d'environnement
├── state.rs             # État partagé injecté dans les contrôleurs
└── main.rs               # Point d'entrée : choisit pg_utilisateur_repository
                           #   ou mock_utilisateur_repository selon USE_MOCK_DB,
                           #   assemble tout, démarre le serveur
```

Schéma simplifié du flux d'une requête protégée :

```
Client (desktop)
   │  Authorization: Bearer <jeton>
   ▼
interface/http (route + middleware d'auth)
   ▼
application (auth_service : logique métier)
   ▼
domain::ports::UtilisateurRepositoryPort (interface)
   ▼                                   ▼
infrastructure::PgUtilisateurRepository   infrastructure::MockUtilisateurRepository
   ▼                                   ▼
   PostgreSQL                          mémoire (RAM)
   (USE_MOCK_DB=false)                 (USE_MOCK_DB=true)
```

Un schéma d'architecture **global** (client desktop + ce service + les
autres services + conteneurs) est de la responsabilité collective de
l'équipe et doit vivre dans le dépôt `infra` — ce README documente
uniquement l'architecture interne de ce service.

## Mode mock (développer sans PostgreSQL)

Un autre service de l'équipe gère les migrations (la création de la table
`utilisateurs`). Plutôt que d'attendre que ce soit prêt, ce service peut
tourner avec `MockUtilisateurRepository` : un dépôt d'utilisateurs **en
mémoire**, avec exactement le même comportement que la vraie base côté API
(mêmes routes, mêmes codes d'erreur, ex. `409` si le pseudo est déjà pris)
— seule la donnée change d'endroit. C'est le même principe que
`MockGateway` dans le dépôt desktop.

Activé par défaut dans `docker-compose.yml` et dans `.env.example` :

```bash
USE_MOCK_DB=true
```

Avec ce réglage, `DATABASE_URL` n'est même pas nécessaire — le service ne
tente aucune connexion à PostgreSQL.

⚠️ **Limites du mode mock**, à garder en tête :
- Rien n'est persisté : un redémarrage du service efface tous les comptes.
- Une seule instance : deux instances du service en mode mock ne
  partagent pas leurs données.
- **Ne pas utiliser ce mode pour la démonstration finale du contrôle** —
  l'énoncé exige explicitement que PostgreSQL soit la source de vérité.
  Dès que la vraie base est prête, repasser à `USE_MOCK_DB=false` (ou
  retirer la ligne) et fournir `DATABASE_URL`.

## Schéma de données attendu

Ce service ne crée pas la table `utilisateurs` (voir plus haut) : il
suppose qu'elle existe déjà, avec au minimum ces colonnes (voir les
requêtes SQL dans `src/infrastructure/pg_utilisateur_repository.rs`, qui
en sont la source de vérité exacte) :

| Colonne | Type | Contraintes |
|---------|------|-------------|
| `id` | UUID | clé primaire |
| `pseudo` | texte | unique, non nul |
| `courriel` | texte | unique, non nul |
| `mot_de_passe_hache` | texte | non nul (hash Argon2, jamais le mot de passe en clair) |
| `role` | texte | non nul, valeurs attendues : `admin`, `moderateur`, `utilisateur` (défaut : `utilisateur`) |
| `cree_le` | horodatage avec fuseau | non nul |
| `modifie_le` | horodatage avec fuseau | non nul |

À partager avec la personne/le service qui gère les migrations, pour que
son schéma corresponde à ce que ce service attend.

## Prérequis

**Rien à installer localement à part Docker** (conforme à la consigne du
contrôle). Pour développer/apprendre en dehors de Docker, il faut en plus :

- [Rust](https://www.rust-lang.org/tools/install), **version 1.85 ou plus
  récente** — installe `cargo` et `rustc`. Vérifier avec `rustc --version` ;
  si c'est plus vieux, lancer `rustup update stable` (voir dépannage
  ci-dessous si `rustup` n'est pas installé).
- [Docker](https://www.docker.com/) et Docker Compose (v2, la commande
  `docker compose`, pas `docker-compose`).

> **Pourquoi 1.85 précisément ?** Ce projet est écrit pour l'édition 2021
> (rien à voir avec ça), mais certaines dépendances *transitives* (des
> crates utilisées par nos propres dépendances, pas ajoutées par nous —
> ici `icu_provider`, tirée par la crate `url`) exigent depuis peu
> l'édition Cargo `edition2024`, stabilisée seulement à partir de Rust
> 1.85. Avec une version plus ancienne, `cargo build`/`cargo run` échoue
> avec une erreur du type `feature 'edition2024' is required`. Le
> `Dockerfile` utilise l'image `rust:1-slim-bookworm` (toujours la
> dernière version stable) pour ne jamais avoir ce problème en Docker ;
> c'est seulement en local, hors Docker, que ta version installée compte.

## Installation et lancement

### Avec Docker (méthode utilisée pour la remise)

```bash
git clone <url-de-ce-dépôt>
cd chatvardage-auth
docker compose up --build
```

Cela démarre le conteneur `auth-api` sur `http://localhost:3001`, **en mode
mock** par défaut (voir [Mode mock](#mode-mock-développer-sans-postgresql)) —
aucune base de données à installer ou attendre. Pour tester avec une vraie
base PostgreSQL, voir cette même section.

Pour arrêter : `Ctrl+C` puis `docker compose down`.

### En local, sans Docker (pour développer plus vite pendant que tu apprends Rust)

```bash
# 1. Copier la config d'exemple (USE_MOCK_DB=true par défaut : pas besoin
#    de base de données pour cette étape)
cp .env.example .env

# 2. Compiler et lancer — cargo télécharge les dépendances la première fois
cargo run
```

`cargo run` recompile automatiquement à chaque changement de code si tu
relances la commande. Pour de la recompilation automatique à la sauvegarde,
tu peux installer `cargo install cargo-watch` puis lancer `cargo watch -x run`.

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
  (doit afficher 1.85 ou plus).

## Variables d'environnement

Voir `.env.example` pour la liste complète et les valeurs par défaut.
Résumé :

| Variable | Rôle |
|----------|------|
| `USE_MOCK_DB` | `true` = dépôt en mémoire, pas de PostgreSQL requis (voir [Mode mock](#mode-mock-développer-sans-postgresql)). `false` = vraie base, `DATABASE_URL` alors requise. |
| `DATABASE_URL` | Chaîne de connexion PostgreSQL (ignorée si `USE_MOCK_DB=true`) |
| `JWT_SECRET` | Secret de signature des JWT — **à changer avant la remise finale**, ne jamais committer un vrai secret |
| `JWT_DUREE_DE_VIE_HEURES` | Durée de validité d'un jeton (défaut : 24h) |
| `PORT` | Port HTTP du service (défaut : 3001) |

## Vérifier que ça fonctionne

```bash
# Santé du service (en mode réel, ça confirme aussi indirectement que la
# connexion à PostgreSQL a réussi — le service ne démarre pas sinon)
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

> À consigner dans ce README avant la remise (voir consigne) : le résultat
> de ces commandes depuis une installation propre (`git clone` +
> `docker compose up --build`), en mode réel (`USE_MOCK_DB=false`) une fois
> la base de données prête, ainsi qu'un `docker exec` ou `pg_isready`
> confirmant que PostgreSQL est en bonne santé.

## Endpoints de l'API

Documentation complète (requêtes, réponses, codes d'erreur) dans
[`docs/API.md`](docs/API.md). Résumé :

| Méthode | Route | Protégée | Description |
|---------|-------|----------|--------------|
| GET | `/health` | non | Vérification de santé |
| POST | `/api/auth/register` | non | Créer un compte |
| POST | `/api/auth/login` | non | Se connecter (courriel ou pseudo) |
| GET | `/api/auth/me` | oui | Utilisateur courant |
| GET | `/api/users` | admin | Lister les comptes |
| DELETE | `/api/users/:id` | admin | Supprimer un compte |
| PATCH | `/api/users/:id/role` | admin | Changer le rôle d'un compte |

## Intégration avec le reste de l'équipe

- **Client desktop** : le `ChatGateway` actuel (Contrôle 1) ne connaît que
  le pseudo (`Utilisateur { id, pseudo }`), sans mot de passe. Pour
  brancher ce service, l'équipe doit étendre l'écran de connexion pour
  collecter courriel + mot de passe, appeler `POST /api/auth/login` (ou
  `/register`), conserver le `jeton` reçu (ex. dans le stockage local déjà
  présent — voir `StockageUtilisateur.ts`), et l'envoyer dans l'en-tête
  `Authorization` des futurs appels à l'API (messages privés,
  configuration).
- **Dépôt infra** : ce dépôt fournit un `Dockerfile` autonome — l'infra n'a
  qu'à construire l'image (`docker build .` ou via `build:` dans son propre
  `docker-compose.yml`) et lui fournir `USE_MOCK_DB=false`, `DATABASE_URL`,
  `JWT_SECRET`, `JWT_DUREE_DE_VIE_HEURES` et `PORT` en variables
  d'environnement. Le `docker-compose.yml` de **ce** dépôt sert uniquement
  au développement local de ce service (en mode mock) ; il n'est pas
  destiné à être utilisé tel quel par l'infra globale (qui aura
  probablement un seul PostgreSQL partagé entre services plutôt qu'un par
  service — à discuter en équipe).
- **Service de migrations** : voir
  [Schéma de données attendu](#schéma-de-données-attendu) — à partager
  avec la personne responsable des migrations pour que la table
  `utilisateurs` corresponde à ce que ce service attend. Une fois cette
  table créée dans un environnement partagé, ce service peut y être
  pointé avec `USE_MOCK_DB=false` + `DATABASE_URL`.
- **Secret JWT partagé** : si un autre service doit vérifier les jetons
  émis ici (plutôt que de toujours passer par `/api/auth/me`), il doit
  connaître le même `JWT_SECRET`. À coordonner avec l'équipe.

## Versions (tags Git)

L'énoncé demande une gestion de version avec branches/tags Git et des
numéros de version à jour "dans les fichiers package.json" — ce dépôt
étant en Rust, l'équivalent est le champ `version` de `Cargo.toml`. Avant
chaque tag de remise, mettre à jour `Cargo.toml` :

```toml
[package]
version = "0.1.0"   # -> incrémenter selon les changements (SemVer)
```

puis :

```bash
git tag -a v0.1.0 -m "Contrôle 2 - service d'authentification"
git push origin v0.1.0
```

## Limites connues et pistes d'amélioration

- Pas de renouvellement de jeton (refresh token) : un jeton expiré oblige
  à se reconnecter. Acceptable pour la portée du contrôle (24h de durée de
  vie par défaut).
- Pas de limitation de débit (rate limiting) sur `/login` — à ajouter si le
  temps le permet, pour limiter les tentatives de force brute.
- Le rôle `admin` doit actuellement être attribué manuellement en base de
  données pour le tout premier compte administrateur (aucun compte n'est
  admin par défaut à l'inscription, par sécurité) :
  ```sql
  UPDATE utilisateurs SET role = 'admin' WHERE pseudo = 'ton_pseudo';
  ```
  Une commande CLI ou un script de "seed" pourrait remplacer cette étape
  manuelle dans une prochaine itération.
- Pas encore de suppression de compte par l'utilisateur lui-même (seulement
  par un admin) — à ajouter si exigé.

## Pour aller plus loin (je débute en Rust)

Quelques repères pour naviguer dans ce projet si Rust est nouveau pour toi :

- **`Result<T, E>` et `?`** : au lieu d'exceptions, une fonction qui peut
  échouer retourne `Result<T, E>`. L'opérateur `?` (ex. `let x =
  fonction_qui_peut_echouer()?;`) propague l'erreur automatiquement si elle
  survient, sinon donne la valeur `T`. Cherche "Rust error handling ?
  operator" si un `?` te semble mystérieux.
- **`async fn` et `.await`** : les fonctions réseau (HTTP, base de données)
  sont asynchrones. `.await` "attend" le résultat sans bloquer tout le
  programme. `#[tokio::main]` sur `main()` démarre le runtime qui exécute
  ce code asynchrone.
- **`Arc<T>`** : partage une valeur entre plusieurs tâches asynchrones sans
  la dupliquer (ex. `jwt_service` dans `state.rs`). C'est un compteur de
  références partagé, similaire à un pointeur intelligent.
- **`trait`** : proche d'une interface. `FromRequestParts` (voir
  `middleware.rs`) est un trait d'Axum qu'on implémente pour créer notre
  propre "extracteur" (`UtilisateurAuthentifie`). `UtilisateurRepositoryPort`
  (voir `domain/ports.rs`) en est un autre, qu'on a écrit nous-mêmes.
- **`Arc<dyn Trait>`** : un pointeur partagé (`Arc`, voir plus haut) vers
  "n'importe quel type qui implémente ce trait" (`dyn Trait`), choisi au
  moment de l'exécution plutôt qu'à la compilation. C'est ce qui permet à
  `main.rs` de décider, selon `USE_MOCK_DB`, d'injecter soit
  `PgUtilisateurRepository` soit `MockUtilisateurRepository` dans
  `AuthService` — le reste du code ne voit que
  `dyn UtilisateurRepositoryPort` et ne sait pas lequel des deux tourne
  réellement.
- Ressources recommandées : [The Rust Book](https://doc.rust-lang.org/book/)
  (gratuit, en ligne), la [documentation d'Axum](https://docs.rs/axum) et
  ses [exemples officiels](https://github.com/tokio-rs/axum/tree/main/examples)
  (notamment `jwt` et `error-handling`, très proches de ce que fait ce
  dépôt).

### Note importante sur la vérification de ce projet

Ce projet a été écrit avec soin (aucune macro `query!` de SQLx nécessitant
une base de données au moment de la compilation, structure simple, chaque
fichier commenté) mais **n'a pas pu être compilé dans l'environnement où il
a été généré** (accès réseau à crates.io bloqué par une politique réseau).
La toute première chose à faire est donc :

```bash
cargo build
```

S'il y a des erreurs de compilation, elles seront probablement mineures
(un nom importé qui a légèrement changé de place entre deux versions
d'une dépendance, etc.). Le message d'erreur de `cargo` est généralement
très explicite en Rust et indique souvent lui-même la correction à
apporter — c'est une bonne première occasion de se familiariser avec le
compilateur.