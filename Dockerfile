# --- Étape 1 : compilation Rust ----------------------------------------
# On compile dans une image complète (avec le compilateur Rust), puis on
# ne garde que le binaire final dans une image finale basée sur Node.js
# (étape 3, requise pour exécuter les migrations Prisma au démarrage).
#
# Rust 1.88.0 satisfait le MSRV du projet (`rust-version = "1.88"`) et
# les dépendances qui exigent l'édition Cargo 2024. Le tag précis rend le
# build Rust stable. Alpine produit un binaire musl statique, sans glibc
# Debian à embarquer dans l'image finale.
FROM rust:1.88.0-alpine AS builder
RUN apk add --no-cache musl-dev

WORKDIR /app

# Seul musl-dev est requis pour la compilation : sqlx est configuré avec
# `runtime-tokio-rustls` (TLS en Rust pur, sans OpenSSL) et aucune autre
# dépendance du projet ne lie de bibliothèque native (`cargo tree`
# confirme l'absence d'`openssl-sys`/`native-tls`) — `libssl-dev`/
# `pkg-config` ont été retirés parce qu'inutiles, pas oubliés.

# Astuce de cache Docker : on copie d'abord uniquement les fichiers de
# dépendances et on compile un projet vide. Tant que Cargo.toml ne
# change pas, Docker réutilise ce cache même si le code source (src/)
# change — évite de re-télécharger/recompiler toutes les dépendances à
# chaque petite modification.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Maintenant le vrai code source.
COPY src ./src

# On force la recompilation du binaire (le "touch" invalide le cache
# incrémental laissé par le build "à vide" ci-dessus).
RUN touch src/main.rs && cargo build --release

# --- Étape 2 : dépendances Node.js (CLI Prisma) -------------------------
# Étape séparée du build Rust : le cache Docker ne réinstalle les
# dépendances npm que si package.json/package-lock.json changent, pas à
# chaque modification du code Rust.
FROM node:22-alpine AS node-deps
RUN apk add --no-cache openssl

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci

# `npx prisma generate` : symbolique ici (ce service n'utilise jamais de
# client Prisma au runtime — voir prisma/schema.prisma et
# infrastructure/pg_utilisateur_repository.rs, qui utilisent SQLx), mais
# suit la même séquence de build que les autres microservices Node de
# l'équipe (voir AGENTS.md, section Docker).
COPY prisma ./prisma
# Prisma valide la datasource durant `generate`. Cette URL factice n'est
# utilisée qu'au build et ne contient aucun secret; l'URL réelle est fournie
# au démarrage par l'environnement du conteneur.
RUN DATABASE_URL=postgresql://placeholder:placeholder@localhost:5432/placeholder \
    npx prisma generate

# --- Étape 3 : image finale ----------------------------------------------
# Basée sur Node.js (pas une image Debian minimale) : le conteneur doit
# exécuter `./node_modules/.bin/prisma migrate deploy` au démarrage, avant de lancer le
# binaire Rust compilé à l'étape 1.
FROM node:22-alpine

# Prendre les correctifs disponibles à chaque rebuild : ce choix de sécurité
# rend volontairement les paquets système dépendants de la date du build.
RUN apk upgrade --no-cache && apk add --no-cache ca-certificates openssl

WORKDIR /app

COPY --from=builder /app/target/release/chatvardage-auth ./chatvardage-auth
COPY --from=node-deps /app/node_modules ./node_modules
COPY package.json ./
COPY prisma ./prisma

# Ne pas tourner en root dans le conteneur final (principe du moindre
# privilège) : `node:22-alpine` fournit déjà un utilisateur non
# privilégié prêt à l'emploi (uid 1000), inutile d'en créer un autre.
# Code et dépendances restent la propriété de root, lisibles/exécutables
# par node : le service n'a pas à réécrire son application au runtime.
# Les migrations utilisent le CLI local : aucun gestionnaire npm au runtime.
RUN rm -rf /usr/local/lib/node_modules/npm /usr/local/bin/npm /usr/local/bin/npx
RUN test -x node_modules/.bin/prisma

USER node

EXPOSE 3001

# Utilisé par docker-compose (condition "service_healthy") pour savoir
# quand l'API est prête à recevoir des requêtes.
HEALTHCHECK --interval=5s --timeout=3s --start-period=10s --retries=5 \
    CMD node -e "fetch('http://localhost:'+ (process.env.PORT || '3001') + '/health').then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"

# Migrer AVANT de démarrer, jamais l'inverse (voir AGENTS.md, section
# Docker) : si la migration échoue, le conteneur s'arrête plutôt que de
# démarrer une API pointant vers un schéma obsolète ou incomplet.
CMD ["sh", "-c", "./node_modules/.bin/prisma migrate deploy && exec ./chatvardage-auth"]
