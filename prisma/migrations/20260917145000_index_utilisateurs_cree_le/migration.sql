-- Accélère GET /api/users, qui retourne les comptes les plus récents
-- avec ORDER BY cree_le DESC LIMIT/OFFSET.
CREATE INDEX "utilisateurs_cree_le_idx" ON "utilisateurs" ("cree_le" DESC);
