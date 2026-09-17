-- Invalide les JWT émis avant un changement de mot de passe ou de rôle.
ALTER TABLE "utilisateurs"
ADD COLUMN "version_auth" INTEGER NOT NULL DEFAULT 0;
