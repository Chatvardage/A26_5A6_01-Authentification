-- Invalide les JWT émis avant un changement de mot de passe.
ALTER TABLE "utilisateurs"
ADD COLUMN "version_auth" INTEGER NOT NULL DEFAULT 0;
