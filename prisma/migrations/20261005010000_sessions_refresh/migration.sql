CREATE TABLE "sessions_refresh" (
  "hache" TEXT PRIMARY KEY,
  "famille" UUID NOT NULL,
  "utilisateur_id" UUID NOT NULL REFERENCES "utilisateurs"("id") ON DELETE CASCADE,
  "version_auth" INTEGER NOT NULL,
  "expire_le" TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '30 days'),
  "consomme" BOOLEAN NOT NULL DEFAULT false,
  "revoque" BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX "sessions_refresh_famille_idx" ON "sessions_refresh"("famille");
CREATE INDEX "sessions_refresh_expire_le_idx" ON "sessions_refresh"("expire_le");
