# AGENTS.md — Chatvardage Authentification

Ce fichier contient les règles de travail pour tout agent IA ou développeur qui modifie ce dépôt.

## Instructions importantes

- Ne crée pas de commits
- Fait seulement des modifications dans les fichiers sans commits
- Explique moi un plan et demande mon approbation avant de modifier des fichiers


## Contexte du projet

Ce dépôt est le service d'**Authentification** de Chatvardage pour le Contrôle 2 du cours `420-5A6-ST – A26`.

Architecture d'équipe :

- `Frontend` : Electron et ChatGateway réel.
- `Serveur-API` : API Gateway / point d'entrée unique.
- `Authentification` : utilisateurs, authentification et rôles (ce dépôt).
- `Room-Service` : gestion et administration des salons.
- `Message-Service` : messages privés, historique et temps réel.
- `User-Config-Service` : alias, thème et préférences.
- `Dashboard` : panneau d'administration / AdminJS.
- `Infra` : Docker Compose, PostgreSQL, Redis et orchestration globale — voir la section [Docker](#docker) pour ce qu'`infra` fait exactement pour ce service.

Ce dépôt ne doit **pas** prendre en charge les salons, les messages privés, Redis, le WebSocket ni les configurations utilisateur. Ces responsabilités appartiennent aux autres microservices.

## Stack

- Rust (édition 2021, 1.88+)
- Axum 0.7 (framework HTTP) + Tokio (runtime async)
- SQLx 0.8 (client PostgreSQL async — requêtes/lecture-écriture au runtime)
- Prisma (schéma + migrations de la table `utilisateurs`, versionnées dans `prisma/migrations/` — CLI Node.js seulement, version figée par `package.json`/`package-lock.json`, aucun client Prisma utilisé au runtime)
- PostgreSQL (base `chatvardage_auth` en environnement d'équipe, provisionnée par `infra` — voir [Docker](#docker) ; base dédiée dans `docker-compose.yml` de ce dépôt seul. Pas de mode mock/en mémoire, ni en production ni dans les tests d'intégration — voir [Couverture obligatoire](#couverture-obligatoire))
- Argon2 (hachage des mots de passe, exécuté via `tokio::task::spawn_blocking`) + JWT via `jsonwebtoken`
- Docker / Docker Compose

## Responsabilités de l'authentification

Le service doit conserver la responsabilité de :

- créer, lire, modifier et supprimer des utilisateurs;
- protéger la sécurité des routes;
- permettre à un admin connecté de lire, créer, modifier et supprimer des utilisateurs;
- prévenir Room-Service, Message-Service et User-Config-Service d'une suppression de compte (`infrastructure::notificateur_suppression`, port `NotificateurSuppressionPort`), après **chaque** suppression réussie, qu'elle vienne de `DELETE /api/auth/me` ou de `DELETE /api/users/:id`;
- gestion des rôles.

Ne pas déplacer ici les responsabilités des autres microservices.

## Contrat HTTP

Le service expose une API REST JSON sous `/api/auth` (inscription, connexion, profil) et `/api/users` (administration) — voir [docs/API.md](docs/API.md) pour le détail des routes, requêtes et réponses ; c'est la seule référence à jour du contrat réellement implémenté. Les routes protégées attendent un jeton JWT dans l'en-tête `Authorization: Bearer <jeton>` (voir `interface/http/middleware.rs`), jamais des en-têtes `x-*` personnalisés ni un préfixe `/http/routes` — une version antérieure de ce fichier décrivait un contrat par en-têtes qui n'a jamais été implémenté (probablement copié d'un autre service de l'équipe) ; ne pas s'y fier.

Le service est destiné à être appelé derrière `Serveur-API` (le Gateway de l'équipe), qui doit relayer les mêmes routes/corps JSON tels quels plutôt que traduire vers un contrat différent. Le Gateway doit aussi appliquer la limitation de débit sur `/api/auth/login` et `/api/auth/register` avant que ces routes publiques ne lancent un calcul Argon2. Dans l'architecture finale, l'authentification doit rester un service interne, jamais exposé directement à Internet.

## Base de données et Prisma

Schéma principal :

- `Utilisateurs`

Les migrations Prisma sont versionnées dans `prisma/migrations/` — c'est la seule source de vérité du schéma de ce service. Il n'y a pas de dépôt en mémoire de secours : `DATABASE_URL` doit toujours pointer vers une base PostgreSQL migrée, en local comme en Docker.

Dans l'environnement d'équipe (`infra`), `infra` provisionne la base `chatvardage_auth` et l'utilisateur `auth_service` (voir [Docker](#docker)), mais **ne crée pas** la table `utilisateurs` — c'est `npx prisma migrate deploy`, exécuté par ce dépôt, qui le fait.

Après une modification de `prisma/schema.prisma` :

```bash
npx prisma generate
npx prisma migrate dev --name <nom_de_la_migration>
```

Ne jamais modifier une migration déjà appliquée pour masquer un changement. Créer une nouvelle migration.

Dans Docker/déploiement, utiliser :

```bash
npx prisma migrate deploy
```

Le code Rust (`src/infrastructure/pg_utilisateur_repository.rs`) continue d'utiliser SQLx pour lire/écrire dans PostgreSQL — Prisma n'est jamais appelé comme client au runtime. Toute modification du schéma Prisma doit être répercutée dans ce fichier (et vice-versa), Prisma ne générant pas de code Rust.

⚠️ Piège Prisma-sans-client-Prisma à connaître avant de modifier `prisma/schema.prisma` : `@default(uuid())` et `@updatedAt` sont des comportements appliqués par le **client** Prisma, jamais appelés ici — un champ avec l'un de ces attributs n'aurait donc aucune valeur par défaut réelle en base, et l'`INSERT`/`UPDATE` de SQLx (qui ne fournit pas ces colonnes explicitement) échouerait. Utiliser `@default(dbgenerated("..."))` pour un défaut évalué côté PostgreSQL (ex. `gen_random_uuid()`, natif depuis PostgreSQL 13). Voir les commentaires dans `prisma/schema.prisma` pour l'exemple complet, y compris `@db.Timestamptz` (requis pour que `chrono::DateTime<Utc>` décode correctement la colonne côté SQLx — le défaut Prisma, `timestamp` sans fuseau, ne fonctionne pas).

⚠️ Deuxième piège, similaire : `courriel` reste annoté `@unique` dans `schema.prisma` (pour la documentation), mais la migration `index_courriel_insensible_a_la_casse` a remplacé l'index btree standard que cette annotation générerait par un index UNIQUE fonctionnel sur `LOWER(courriel)` — Prisma ne modélise pas les index fonctionnels/expression, donc son DSL ne peut pas représenter fidèlement ce qui existe réellement en base. Défense en profondeur : `trouver_par_identifiant` compare déjà `LOWER(courriel) = LOWER($1)` ; cet index permet cette recherche insensible à la casse tout en faisant respecter la même règle directement par PostgreSQL. Avant de relancer `prisma migrate dev` sur le modèle `Utilisateur`, relire le SQL généré — la détection de dérive de Prisma pourrait proposer de recréer l'index standard sur `courriel`, ce qui annulerait cette protection.

## Commandes obligatoires avant de terminer une modification

`cargo test`/`cargo llvm-cov` exigent une vraie base PostgreSQL migrée
(pas de mode mock, voir [Stack](#stack)) : `infrastructure::pg_utilisateur_repository`
teste le vrai SQL contre elle. Démarrer/migrer cette base une seule fois :

```bash
docker compose up -d postgres
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth \
  npx prisma migrate deploy
```

puis exporter `DATABASE_URL` (même valeur) dans le shell utilisé pour les commandes ci-dessous — ou préfixer chaque commande, comme dans les exemples.

Compiler le projet (télécharge et vérifie les dépendances) :

```bash
cargo build
```

Puis exécuter au minimum :

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth cargo test
```

Pour une modification importante de logique métier, exécuter aussi :

```bash
DATABASE_URL=postgres://chatvardage:chatvardage@localhost:5432/chatvardage_auth \
  cargo llvm-cov --ignore-filename-regex 'main\.rs' --fail-under-lines 99
```

(nécessite `rustup component add llvm-tools-preview` puis `cargo install cargo-llvm-cov`, une seule fois par machine.)

Pour vérifier que le build de production (celui utilisé par le `Dockerfile`) passe aussi :

```bash
cargo build --release
```

`cargo fmt` (sans `--check`) modifie les fichiers; ne l'utiliser que lorsqu'un formatage automatique est souhaité.

Un agent ne doit pas annoncer que le travail est terminé si une de ces vérifications échoue. Il doit corriger l'erreur ou indiquer clairement ce qui bloque.

## Couverture obligatoire

- `cargo llvm-cov --ignore-filename-regex 'main\.rs' --fail-under-lines 99` doit passer — couverture des lignes ≥ 99 % (hors `main.rs`, voir ci-dessous).
- `main.rs` est exclu du calcul : c'est l'assemblage final (config, pool PostgreSQL, serveur HTTP) — un point d'entrée d'intégration vérifié manuellement (`docker compose up`, voir README) plutôt que par des tests unitaires, comme c'est l'usage pour ce genre de fonction `main` en Rust.
- Le reste du code atteint 100 % de couverture des lignes à quelques branches défensives près : les chemins d'erreur de `jsonwebtoken::encode` (`infrastructure/jwt.rs`), d'`Argon2::hash_password` et de jointure de `tokio::task::spawn_blocking` (`infrastructure/mot_de_passe.rs`). Aucune entrée valide et raisonnable ne les déclenche ; les forcer artificiellement exigerait d'injecter une fausse implémentation uniquement pour gonfler le chiffre, sans garantie réelle en échange — ne pas ajouter une telle abstraction juste pour la couverture.
- Toute nouvelle branche de logique **atteignable** (succès, refus, erreurs métier) doit être accompagnée de tests plutôt que de faire baisser artificiellement le seuil.
- Les tests HTTP de bout en bout (`interface/http/routes.rs`) utilisent `infrastructure::fake_utilisateur_repository::FakeUtilisateurRepository` — un dépôt en mémoire compilé uniquement en mode test (`#[cfg(test)]`), jamais exposé en production. Il expose `provoquer_erreur_interne(bool)` pour tester la branche 500 sans dépendre d'une vraie panne PostgreSQL.
- Pour un rapport détaillé (voir quelles lignes manquent) : `cargo llvm-cov --ignore-filename-regex 'main\.rs' --html`, puis ouvrir `target/llvm-cov/html/index.html`.
- Avant une PR : `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, `cargo llvm-cov --ignore-filename-regex 'main\.rs' --fail-under-lines 99`, `cargo build --release`.

## Lint, style et qualité

- Respecter `cargo fmt` et `cargo clippy` — aucun warning clippy ignoré sans justification en commentaire.
- Éviter `unwrap()`/`expect()` dans le code de production en dehors de `main.rs` (démarrage) — utiliser `Result`/`?` et `AppError` (voir `error.rs`). Exceptions documentées, toutes sur des chemins garantis sûrs par construction plutôt que par simple probabilité (à ne pas confondre avec les branches d'erreur `Result`/`AppError` difficiles à couvrir listées dans [Couverture obligatoire](#couverture-obligatoire) — celles-ci gèrent déjà l'erreur proprement, sans `panic!`) :
  - `infrastructure/jwt.rs`, `generer_token` : `.expect(...)` sur `Duration::hours(n).checked_add_signed(...)` — exécuté à chaque connexion/inscription, mais `n` (`JWT_DUREE_DE_VIE_HEURES`) est bornée à `[1, 720]` par `config.rs` au démarrage (voir `JWT_DUREE_MAX_HEURES`), ce qui rend un débordement arithmétique impossible en pratique, pas seulement improbable.
  - `infrastructure/mot_de_passe.rs`, `argon2()` : `.expect(...)` sur `Params::new(...)` — tous les arguments sont des constantes validées par le test `parametres_argon2_sont_explicitement_calibres`, donc la construction ne peut pas échouer à l'exécution.
  - `config.rs`, `Config::depuis_variables_environnement()` : les `.expect(...)` sur `JWT_SECRET` et `DATABASE_URL`, ainsi que les `assert!(...)` sur la longueur du secret et les bornes de `JWT_DUREE_DE_VIE_HEURES`, arrêtent volontairement le processus avant de démarrer avec une configuration obligatoire absente ou dangereuse. Cette configuration est fournie au démarrage, jamais par une requête utilisateur, et chaque refus est couvert par les tests de `config.rs`.
  - Toute nouvelle exception doit être ajoutée à cette liste, pas laissée implicite dans un commentaire local seulement.
- Garder les contrôleurs (`interface/http/*_controller.rs`) minces : extraction JSON/en-têtes et délégation au service.
- Toujours extraire le corps JSON et les segments d'URL via `super::extracteurs::{Json, Path}` (jamais `axum::extract::Json`/`Path` directement) : ce sont eux qui garantissent que toute erreur d'extraction (JSON malformé, valeur d'énumération invalide, UUID invalide dans l'URL) respecte l'enveloppe `{ "erreur": ... }` documentée dans `docs/API.md`, plutôt que le corps en texte brut par défaut d'axum.
- Garder la logique métier dans `application::auth_service::AuthService`. Un contrôleur peut appeler `state.depot_utilisateurs` directement **seulement** pour du CRUD pur sans règle métier (ex. `auth_controller::me`, `utilisateurs_controller::lister`/`obtenir`/`supprimer`/`changer_role` : une lecture, une suppression ou un changement de rôle sans validation ni effet de bord) — dès qu'une opération implique de la validation, une normalisation (ex. courriel), une vérification de mot de passe ou toute autre règle, elle passe par une méthode d'`AuthService` (voir `inscrire`/`connecter`/`modifier_profil`/`modifier_utilisateur`). C'est un choix assumé, pas une incohérence : ajouter des méthodes `AuthService` qui ne feraient que déléguer telles quelles à `depot_utilisateurs` serait de l'indirection sans bénéfice.
- Garder l'accès aux données derrière `domain::ports::UtilisateurRepositoryPort` — pas d'appel SQLx direct depuis `application/` ou `interface/`. Prisma n'est utilisé dans ce dépôt que pour le schéma et les migrations (`prisma/schema.prisma`) : aucun client Prisma n'est appelé au runtime, l'accès PostgreSQL passe uniquement par SQLx dans `infrastructure/pg_utilisateur_repository.rs`.
- Réutiliser `exiger_admin` (`middleware.rs`) au lieu de dupliquer les règles de permission par rôle.
- Valider les entrées avec `validator` sur les DTO (`application/dto.rs`) — toujours donner une longueur **maximale** à un champ texte fourni par le client (pas seulement un minimum), sans quoi un client peut envoyer des valeurs arbitrairement volumineuses qui seront quand même traitées (ex. hachées avec Argon2, délibérément coûteux en CPU) avant d'être rejetées ailleurs.
- Exécuter toute opération CPU-bound (notamment Argon2) via `tokio::task::spawn_blocking` plutôt que directement sur un worker Tokio.
- Retourner des erreurs HTTP explicites via `AppError`, jamais un `panic!` qui ferait planter le service.
- Ne pas laisser de code mort, de `println!` de debug (utiliser `tracing::info!/warn!/error!`) ou de dépendances inutilisées dans `Cargo.toml`.

## Docker

Ce dépôt suit le guide Docker/Prisma de l'équipe (`infra`) : chaque microservice possède son propre `Dockerfile`, son propre schéma/migrations Prisma, exécute `npx prisma generate` **pendant le build** et `npx prisma migrate deploy` **au démarrage du conteneur**, puis démarre l'application. `infra` ne crée que les bases PostgreSQL et les utilisateurs — jamais les tables métier.

### Séquence attendue du Dockerfile

Le `Dockerfile` de ce dépôt utilise 3 stages (pas 2) pour profiter du cache Docker : le build Rust et l'installation de la CLI Prisma ne dépendent pas des mêmes fichiers (`src/` vs `package.json`/`prisma/`), les séparer évite de réinstaller les dépendances npm à chaque changement de code Rust, et inversement.

1. **`builder`** (`rust:1.88.0-slim-bookworm`) : compile le binaire Rust (`cargo build --release`).
2. **`node-deps`** (`node:22-bookworm-slim`) : installe la CLI Prisma (`npm ci`, version figée par `package.json`/`package-lock.json`) et exécute `npx prisma generate`.
3. **Image finale** (`node:22-bookworm-slim`) : embarque le binaire Rust compilé (copié depuis `builder`) **et** de quoi exécuter le CLI Prisma (`node_modules/`, `prisma/`, copiés depuis `node-deps`) au démarrage.
4. **`CMD`** : `npx prisma migrate deploy && exec ./chatvardage-auth` (migrer avant de démarrer, jamais l'inverse ; `exec` transmet les signaux au binaire).

Le résultat (migration au démarrage, generate avant) est équivalent au patron à 2 étapes décrit plus haut — c'est une optimisation de cache, pas un écart de comportement.

### Test en solo (ce dépôt seul)

```bash
docker compose up --build
```

Doit lancer :
- une base PostgreSQL de développement (service `postgres` du `docker-compose.yml` de ce dépôt);
- les migrations Prisma;
- le service d'authentification sur `3001`.

Healthcheck attendu :

```bash
curl http://localhost:3001/health
```

### Intégration dans `infra`

`infra` démarre PostgreSQL (service Docker `postgres`, réseau `chatvardage-network`) et provisionne la base `chatvardage_auth` + l'utilisateur `auth_service` (variables `AUTH_DB`, `AUTH_DB_USER`, `AUTH_DB_PASSWORD` du compose d'`infra`). Ce dépôt est alors ajouté comme bloc dans le `docker-compose.yml` d'`infra` :

```yaml
auth-service:
  build:
    context: ../A26_5A6_01-Authentification
    dockerfile: Dockerfile
  environment:
    DATABASE_URL: postgresql://auth_service:auth_service_dev@postgres:5432/chatvardage_auth
    JWT_SECRET: ${JWT_SECRET}
    JWT_DUREE_DE_VIE_HEURES: 24
    PORT: 3001
    CORS_ORIGINS: ${CORS_ORIGINS}
  depends_on:
    postgres:
      condition: service_healthy
  networks:
    - chatvardage
```

Règles à respecter :

- utiliser le nom du service Docker (`postgres`), jamais `localhost`, pour joindre la base depuis l'intérieur du conteneur;
- ne jamais committer de vrais secrets — `JWT_SECRET` et les mots de passe de dev restent dans `.env`/`.env.example`, jamais en dur dans un fichier versionné; `JWT_SECRET` doit toujours faire au moins 32 caractères (256 bits, voir `config.rs`) — le service refuse de démarrer sinon — une variable présente mais vide (ex. `.env.example` copié sans être rempli) signerait sinon tous les jetons avec une clé triviale;
- les dépôts doivent être clonés côte à côte (`.../A26_5A6_01-Infra` et `.../A26_5A6_01-Authentification`) pour que le `context: ../A26_5A6_01-Authentification` fonctionne;
- le `docker-compose.yml` de **ce** dépôt sert uniquement au développement local autonome — ce n'est pas celui utilisé par `infra`.

Ne pas ajouter Redis à ce service : Redis sert au flux temps réel du service de messagerie, pas à l'authentification, même si `infra` le démarre pour l'équipe.

## Documentation

Toute modification d'une route, d'un DTO, d'une permission, d'une variable d'environnement, d'un port ou d'une procédure de lancement doit mettre à jour :

- `docs/API.md`;
- `README.md`;
- `.env.example` lorsque pertinent;
- ce fichier `AGENTS.md` si les règles de développement changent.

Le README doit rester suffisant pour démarrer et tester le service sans connaissance implicite du projet.

## Avant une Pull Request

Vérifier :

- [ ] le changement reste dans la responsabilité du service d'authentification (comptes, rôles, JWT — pas les salons, messages privés, Redis/WebSocket ni la configuration utilisateur);
- [ ] aucun secret ou `.env` réel n'est commité;
- [ ] le schéma Prisma (`prisma/schema.prisma`) et les requêtes SQLx (`pg_utilisateur_repository.rs`) restent synchronisés si le schéma change;
- [ ] les migrations Prisma nécessaires sont présentes (`prisma/migrations/`);
- [ ] `package-lock.json` est à jour si `package.json` (version de la CLI Prisma) a changé;
- [ ] `cargo fmt --check` passe;
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passe;
- [ ] `cargo test` passe;
- [ ] `cargo llvm-cov --ignore-filename-regex 'main\.rs' --fail-under-lines 99` passe (≥ 99 % de couverture, `main.rs` exclu — voir [Couverture obligatoire](#couverture-obligatoire));
- [ ] `cargo build --release` passe;
- [ ] `docs/API.md` et le README sont à jour;
- [ ] `docker compose up --build` fonctionne si Docker/DB/migrations ont été modifiés, et le Dockerfile respecte la séquence `prisma generate` (build) / `prisma migrate deploy` (démarrage) du guide `infra`;
- [ ] les changements incompatibles avec le client desktop, `infra` ou les autres services (ex. format du jeton JWT, variables d'environnement attendues) sont documentés.

Dans la description de PR, résumer les changements, indiquer les tests exécutés et signaler tout impact sur les autres dépôts.