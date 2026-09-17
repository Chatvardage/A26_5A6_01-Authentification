-- CreateTable
CREATE TABLE "utilisateurs" (
    "id" UUID NOT NULL DEFAULT gen_random_uuid(),
    "pseudo" TEXT NOT NULL,
    "courriel" TEXT NOT NULL,
    "mot_de_passe_hache" TEXT NOT NULL,
    "role" TEXT NOT NULL DEFAULT 'utilisateur',
    "cree_le" TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "modifie_le" TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "utilisateurs_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "utilisateurs_pseudo_key" ON "utilisateurs"("pseudo");

-- CreateIndex
CREATE UNIQUE INDEX "utilisateurs_courriel_key" ON "utilisateurs"("courriel");
