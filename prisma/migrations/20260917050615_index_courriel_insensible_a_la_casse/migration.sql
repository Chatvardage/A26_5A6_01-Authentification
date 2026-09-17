-- Défense en profondeur : `trouver_par_identifiant` (pg_utilisateur_repository.rs)
-- compare `courriel = LOWER($1)` en s'appuyant sur l'invariant applicatif
-- que la colonne est toujours stockée en minuscules (normalisée dans
-- AuthService avant chaque écriture). L'index UNIQUE btree classique créé
-- par `@unique` dans schema.prisma est sensible à la casse : sans ce
-- correctif, un script de correction manuelle, un seed ou un futur
-- endpoint qui oublierait la normalisation pourrait créer deux comptes
-- avec le même courriel à la casse près, cassant silencieusement cet
-- invariant. Un index UNIQUE fonctionnel sur LOWER(courriel) fait
-- respecter la même règle directement par PostgreSQL, indépendamment de
-- la discipline applicative.
--
-- ⚠️ Cette migration remplace l'index généré par `@unique` sur `courriel`
-- (voir schema.prisma) par un équivalent que Prisma ne sait pas modéliser
-- nativement (pas d'index fonctionnel/expression dans son DSL). Si
-- `prisma migrate dev` est relancé après une future modification du
-- modèle `Utilisateur`, vérifier le SQL généré avant de l'appliquer : la
-- détection de dérive de Prisma pourrait proposer de recréer l'index
-- standard sur `courriel` (annulant cette migration) si elle réintrospecte
-- la base sans tenir compte de ce fichier.

-- DropIndex
DROP INDEX "utilisateurs_courriel_key";

-- CreateIndex (fonctionnel, insensible à la casse)
CREATE UNIQUE INDEX "utilisateurs_courriel_key" ON "utilisateurs" (LOWER("courriel"));
