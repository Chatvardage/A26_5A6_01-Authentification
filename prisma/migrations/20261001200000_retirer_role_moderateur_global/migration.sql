-- Le modérateur n'est plus un rôle global : il n'existe que dans les
-- serveurs (Room-Service). Les anciens comptes deviennent des utilisateurs.
UPDATE "utilisateurs" SET "role" = 'utilisateur' WHERE "role" = 'moderateur';
