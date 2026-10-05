# Chatvardage — Authentification et comptes

API Rust/Axum : inscription, connexion, JWT, comptes et rôles. PostgreSQL conserve les utilisateurs; Prisma gère les migrations et SQLx les requêtes.

## Lancer le service complet

Le [README Infra](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/README.md) est le point de départ : clonage des huit dépôts, secrets, Docker Compose, administration et client desktop.

Depuis le dépôt Infra :

```bash
docker compose up --build -d
./scripts/healthcheck.sh
```

Préparer auparavant `.env` selon Infra. Le healthcheck vérifie PostgreSQL, les quatre bases, Redis, les six services dont Dashboard, puis les dépendances du Gateway. `--infra` limite le diagnostic à PostgreSQL et Redis.

## Développer ce dépôt seul

Prérequis : Docker Compose v2; hors Docker, Rust 1.88+, Node 22 pour Prisma, PostgreSQL migré.

```bash
git clone https://github.com/Chatvardage/A26_5A6_01-Authentification.git
cd A26_5A6_01-Authentification
cp .env.example .env
docker compose up --build
```

Renseigner `JWT_SECRET` avec une valeur générée (`openssl rand -hex 32`). Les tests SQL exigent `DATABASE_URL` vers une base de test migrée. Voir le guide détaillé pour la couverture Rust (99 % minimum).

Service local : `http://localhost:3001`. Santé : `GET /health`. Les Compose individuels utilisent des ports communs à Infra : choisir un seul environnement à la fois.

La configuration complète, les routes, les exemples et les procédures Node/Rust sont dans le [guide du service dans Infra](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/services/Authentification.md). Les valeurs de `.env.example` sont réservées au développement.

## Vérifications

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
npm audit --audit-level=high
```

Les migrations appartiennent à ce dépôt (`prisma/` lorsqu’applicable); Infra crée uniquement les bases et les comptes SQL. Consulter [AGENTS.md](AGENTS.md) pour les règles et vérifications supplémentaires.

## Documentation

- [Architecture complète](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/architecture.md)
- [API et événements](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/api/README.md)
- [Scénario de validation](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/validation.md)
- [Versions et publication](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/versions.md)

## Sessions renouvelables

Login/register ajoutent `refresh_token` à leur réponse existante. Les nouveaux JWT
d'accès durent 15 minutes; les refresh tokens opaques sont hachés (SHA-256) dans
PostgreSQL. `POST /api/auth/refresh` consomme et remplace atomiquement un refresh
token. La famille expire après 30 jours, sans prolongation par rotation.
`POST /api/auth/logout` révoque la famille à partir d'un token courant ou consommé.
Les deux routes utilisent `{ "refresh_token": "<opaque>" }` sans Bearer et passent
par le Gateway. Logout-all et changement de mot de passe invalident aussi les
refresh tokens via `version_auth`. Aucun changement Compose ni nouveau secret
n'est nécessaire : appliquer la nouvelle migration Prisma au démarrage.
Voir le [contrat central](../A26_5A6_01-Infra/docs/api/authentification.md#sessions-renouvelables--register-login-refresh-et-logout).
