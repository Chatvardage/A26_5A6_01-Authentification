# Documentation du contrat

La référence est maintenue dans [Infra](https://github.com/Chatvardage/A26_5A6_01-Infra/blob/main/docs/api/authentification.md). Mettre cette référence à jour avec le code du service.

Le contrat comprend aussi les sessions renouvelables :
`POST /api/auth/refresh` et `POST /api/auth/logout`, corps JSON `{ "refresh_token": "<opaque>" }`,
sans Bearer. Réponse refresh : `jeton`, `refresh_token`, `utilisateur`. Logout : 204.
Voir la référence centrale liée ci-dessus pour rotation, expiration et révocation.
