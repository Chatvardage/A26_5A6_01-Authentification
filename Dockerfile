# --- Étape 1 : compilation -------------------------------------------
# On compile dans une image complète (avec le compilateur Rust), puis on
# ne garde que le binaire final dans une image minimale (étape 2) :
# l'image livrée ne contient ni le code source, ni le compilateur,
# seulement l'exécutable — quelques Mo au lieu de ~1.5 Go.
#
# Le tag "1" (au lieu d'un numéro précis comme "1.82") pointe toujours
# vers la dernière version stable de Rust 1.x. C'est important ici :
# certaines dépendances transitives (ex. icu_provider, via la crate
# `url`) exigent une édition Cargo ("edition2024") qui demande Rust
# 1.85 ou plus récent — une version trop ancienne fait échouer le build
# avec une erreur du genre "feature `edition2024` is required".
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Dépendances système nécessaires pour compiler sqlx (TLS) et lier les
# bibliothèques C standards.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Astuce de cache Docker : on copie d'abord uniquement les fichiers de
# dépendances et on compile un projet vide. Tant que Cargo.toml ne
# change pas, Docker réutilise ce cache même si le code source (src/)
# change — évite de re-télécharger/recompiler toutes les dépendances à
# chaque petite modification.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release || true

# Maintenant le vrai code source. (Ce service ne gère pas les migrations
# de la base de données — un autre service de l'équipe s'en charge — donc
# aucun dossier migrations/ à copier ici.)
COPY src ./src

# On force la recompilation du binaire (le "touch" invalide le cache
# incrémental laissé par le build "à vide" ci-dessus).
RUN touch src/main.rs && cargo build --release

# --- Étape 2 : image finale --------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/chatvardage-auth ./chatvardage-auth

EXPOSE 3001

# Utilisé par docker-compose (condition "service_healthy") pour savoir
# quand l'API est prête à recevoir des requêtes.
HEALTHCHECK --interval=5s --timeout=3s --start-period=10s --retries=5 \
    CMD curl -f http://localhost:3001/health || exit 1

CMD ["./chatvardage-auth"]