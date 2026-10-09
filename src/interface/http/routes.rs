//! Déclaration de toutes les routes HTTP exposées par le service.
//! Regroupées ici : quelqu'un qui veut savoir "quelles routes existent"
//! n'a qu'un seul fichier à lire (voir aussi docs/API.md pour le détail
//! des requêtes/réponses).

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, Method, StatusCode},
    routing::{get, patch, post},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::state::AppState;

use super::{auth_controller, utilisateurs_controller};

/// Plafond explicite pour le corps des requêtes JSON de ce service —
/// bien au-delà de tout besoin légitime (les plus gros champs validés,
/// `mot_de_passe`/`courriel`, sont eux-mêmes plafonnés à quelques
/// centaines de caractères, voir `application/dto.rs`), mais bien en
/// dessous des 2 Mo par défaut d'axum. Réduit la fenêtre pour un client
/// qui enverrait des corps volumineux dans l'espoir de faire travailler
/// inutilement le serveur (désérialisation, puis rejet par `validator`).
const LIMITE_CORPS_REQUETE_OCTETS: usize = 16 * 1024;

/// `origines_autorisees` : liste blanche CORS. Une variable absente, vide
/// ou invalide ne permet aucune origine : l'infrastructure doit fournir
/// explicitement les origines de l'application Electron et du panneau
/// d'administration via `CORS_ORIGINS` (voir `config.rs`).
pub fn construire(state: AppState, origines_autorisees: Option<&[String]>) -> Router {
    let api_auth = Router::new()
        .route("/register", post(auth_controller::register))
        .route("/login", post(auth_controller::login))
        .route("/refresh", post(auth_controller::refresh))
        .route("/logout", post(auth_controller::logout))
        .route("/logout-all", post(auth_controller::deconnexion_partout))
        .route("/logout-others", post(auth_controller::deconnexion_autres))
        .route(
            "/me",
            get(auth_controller::me)
                .patch(auth_controller::modifier_moi)
                .delete(auth_controller::supprimer_moi),
        );

    let api_users = Router::new()
        .route(
            "/",
            get(utilisateurs_controller::lister).post(utilisateurs_controller::creer),
        )
        // Déclarée avant `/:id` : un segment statique (`lookup`) a
        // priorité sur un paramètre nommé dans le routeur d'axum (basé sur
        // `matchit`), donc l'ordre n'a pas d'incidence fonctionnelle ici —
        // mais la garder avant `/:id` documente l'intention et évite toute
        // ambiguïté de lecture avec la route d'administration ci-dessous.
        // Contrairement aux autres routes `/api/users/*`, celle-ci utilise
        // `UtilisateurAuthentifie` (voir utilisateurs_controller::rechercher),
        // pas un extracteur admin : n'importe quel utilisateur connecté
        // peut résoudre `{id -> pseudo}` pour d'autres comptes.
        .route("/lookup", get(utilisateurs_controller::rechercher))
        // Même principe que `/lookup` : segment statique, ouvert à tout
        // utilisateur authentifié (voir utilisateurs_controller::rechercher_par_pseudo).
        .route(
            "/search",
            get(utilisateurs_controller::rechercher_par_pseudo),
        )
        .route(
            "/:id",
            get(utilisateurs_controller::obtenir)
                .patch(utilisateurs_controller::modifier)
                .delete(utilisateurs_controller::supprimer),
        )
        .route("/:id/role", patch(utilisateurs_controller::changer_role));

    let cors_restreint = || {
        CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
    };
    let cors = match origines_autorisees {
        None => {
            tracing::warn!("CORS_ORIGINS est absente — aucune origine n'est autorisée");
            cors_restreint()
        }
        Some([]) => {
            tracing::warn!(
                "CORS_ORIGINS est définie mais ne contient aucune origine (ex. \
                 \"CORS_ORIGINS=,,,\") — aucune origine n'est autorisée"
            );
            cors_restreint()
        }
        Some(origines) => {
            // Une origine qui ne parse pas est écartée (pas de panique
            // pour une simple faute de frappe), mais jamais
            // silencieusement.
            let valeurs: Vec<_> = origines
                .iter()
                .filter_map(|o| match o.parse() {
                    Ok(valeur) => Some(valeur),
                    Err(_) => {
                        tracing::warn!(origine = o, "CORS_ORIGINS: origine invalide, ignorée");
                        None
                    }
                })
                .collect();
            if valeurs.is_empty() {
                tracing::warn!(
                    "CORS_ORIGINS: aucune origine valide après filtrage — aucune \
                     origine n'est autorisée"
                );
                cors_restreint()
            } else {
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(valeurs))
                    .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
            }
        }
    };

    Router::new()
        .route("/health", get(sante))
        .nest("/api/auth", api_auth)
        .nest("/api/users", api_users)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(DefaultBodyLimit::max(LIMITE_CORPS_REQUETE_OCTETS))
        .method_not_allowed_fallback(methode_non_autorisee)
        .fallback(route_inconnue)
        .with_state(state)
}

/// GET /health — utilisé par le `HEALTHCHECK` du Dockerfile et par
/// docker-compose pour savoir si le service est prêt (voir consigne :
/// "Vérifier la santé de la base de données et de Redis"). Interroge
/// réellement PostgreSQL via `verifier_sante` (`SELECT 1`) plutôt que de
/// répondre `ok` inconditionnellement : sans ça, ce endpoint confirmerait
/// seulement que le processus HTTP tourne, pas que sa dépendance
/// critique (la base) est joignable — ce que le `HEALTHCHECK` Docker et
/// le `depends_on: condition: service_healthy` d'`infra` supposent.
async fn sante(State(state): State<AppState>) -> Result<(StatusCode, &'static str), AppError> {
    match state.depot_utilisateurs.verifier_sante().await {
        Ok(()) => Ok((StatusCode::OK, "ok")),
        Err(e) => {
            tracing::error!(erreur = %e, "healthcheck: base de données inaccessible");
            Err(AppError::ServiceIndisponible(
                "base de données inaccessible".into(),
            ))
        }
    }
}

async fn route_inconnue() -> AppError {
    AppError::Introuvable("route introuvable".into())
}

async fn methode_non_autorisee() -> AppError {
    AppError::MethodeNonAutorisee("méthode HTTP non autorisée".into())
}

/// Tests de bout en bout : chaque test construit sa propre application
/// (état + routes) avec un dépôt en mémoire dédié (`FakeUtilisateurRepository`,
/// voir `infrastructure::fake_utilisateur_repository`) et envoie de
/// vraies requêtes HTTP au `Router` via `tower::ServiceExt::oneshot` (pas
/// de vraie socket TCP). C'est le niveau le plus proche de la réalité
/// pour vérifier ensemble le routage, le middleware d'authentification
/// et les contrôleurs — les tests unitaires plus bas dans chaque fichier
/// couvrent la logique fine isolément.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::application::auth_service::AuthService;
    use crate::domain::ports::UtilisateurRepositoryPort;
    use crate::domain::utilisateur::Role;
    use crate::infrastructure::fake_utilisateur_repository::FakeUtilisateurRepository;
    use crate::infrastructure::jwt::{Claims, JwtService};
    use crate::infrastructure::notificateur_suppression::NotificateurEnregistreur;

    #[tokio::test]
    async fn echec_creation_session_et_reprise_login() {
        let ctx = nouveau_contexte();
        ctx.depot_concret.provoquer_erreur_interne_apres(1);
        let body = json!({"pseudo":"recover_user","courriel":"recover@example.com","mot_de_passe":"password-fixture"});
        assert_eq!(
            requete(&ctx.app, "POST", "/api/auth/register", None, Some(body))
                .await
                .0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        // La panne différée du fake reste active jusqu'à reconfiguration.
        // Autoriser les deux opérations de login (lecture + création de session).
        ctx.depot_concret.provoquer_erreur_interne_apres(2);
        let (status, session) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({"identifiant":"recover_user","mot_de_passe":"password-fixture"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        ctx.depot_concret.provoquer_erreur_interne_apres(1);
        assert_eq!(
            requete(
                &ctx.app,
                "GET",
                "/api/auth/me",
                session["jeton"].as_str(),
                None
            )
            .await
            .0,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn session_rotation_logout_et_compatibilite() {
        let ctx = nouveau_contexte();
        let (_, initial) = requete(&ctx.app, "POST", "/api/auth/register", None,
            Some(json!({"pseudo":"refresh_user", "courriel":"refresh@example.com", "mot_de_passe":"password-fixture"}))).await;
        let old = initial["refresh_token"].as_str().unwrap();
        assert_eq!(old.len(), 64);
        let (status, renewed) = requete(
            &ctx.app,
            "POST",
            "/api/auth/refresh",
            None,
            Some(json!({"refresh_token":old})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_ne!(renewed["jeton"], initial["jeton"]);
        assert_ne!(renewed["refresh_token"], initial["refresh_token"]);
        let token = renewed["jeton"].as_str().unwrap();
        assert_eq!(
            requete(&ctx.app, "GET", "/api/auth/me", Some(token), None)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/refresh",
                None,
                Some(json!({"refresh_token":old}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        // A consumed credential can revoke its family, even after a concurrent rotation.
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/logout",
                None,
                Some(json!({"refresh_token":old}))
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            requete(&ctx.app, "GET", "/api/auth/me", Some(token), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/refresh",
                None,
                Some(json!({"refresh_token":renewed["refresh_token"]}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/logout",
                None,
                Some(json!({"refresh_token":old}))
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn refresh_entrees_erreurs_et_revocation_globale() {
        let ctx = nouveau_contexte();
        for body in [
            json!({}),
            json!({"refresh_token":"short"}),
            json!({"refresh_token":"x".repeat(64)}),
            json!({"refresh_token":"a".repeat(64),"extra":true}),
        ] {
            for path in ["/api/auth/refresh", "/api/auth/logout"] {
                assert_eq!(
                    requete(&ctx.app, "POST", path, None, Some(body.clone()))
                        .await
                        .0,
                    StatusCode::BAD_REQUEST
                );
            }
        }
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/refresh",
                None,
                Some(json!({"refresh_token":"a".repeat(64)}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/logout",
                None,
                Some(json!({"refresh_token":"a".repeat(64)}))
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        let (_, initial) = requete(&ctx.app, "POST", "/api/auth/register", None,
            Some(json!({"pseudo":"revocation_user", "courriel":"revocation@example.com", "mot_de_passe":"password-fixture"}))).await;
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/logout-all",
                initial["jeton"].as_str(),
                None
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/refresh",
                None,
                Some(json!({"refresh_token":initial["refresh_token"]}))
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        ctx.depot_concret.provoquer_erreur_interne(true);
        for path in ["/api/auth/refresh", "/api/auth/logout"] {
            assert_eq!(
                requete(
                    &ctx.app,
                    "POST",
                    path,
                    None,
                    Some(json!({"refresh_token":initial["refresh_token"]}))
                )
                .await
                .0,
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
    }

    #[tokio::test]
    async fn refresh_concurrent_consomme_une_seule_fois() {
        let ctx = nouveau_contexte();
        let (_, initial) = requete(&ctx.app, "POST", "/api/auth/register", None,
            Some(json!({"pseudo":"parallel_user", "courriel":"parallel@example.com", "mot_de_passe":"password-fixture"}))).await;
        let body = json!({"refresh_token":initial["refresh_token"]});
        let (a, b) = tokio::join!(
            requete(
                &ctx.app,
                "POST",
                "/api/auth/refresh",
                None,
                Some(body.clone())
            ),
            requete(&ctx.app, "POST", "/api/auth/refresh", None, Some(body))
        );
        assert!([a.0, b.0].contains(&StatusCode::OK));
        assert!([a.0, b.0].contains(&StatusCode::UNAUTHORIZED));
    }

    /// Environnement de test : garde une référence concrète au dépôt
    /// (`depot_concret`) en plus de celle (dynamique) injectée dans
    /// `AppState`, pour pouvoir promouvoir un compte admin directement
    /// (voir README : la promotion admin est une opération manuelle,
    /// hors API) et déclencher `provoquer_erreur_interne` dans les tests
    /// de la branche 500.
    struct Contexte {
        state: AppState,
        app: Router,
        depot_concret: Arc<FakeUtilisateurRepository>,
        secret: String,
        /// Comptes signalés supprimés aux autres services.
        notificateur: Arc<NotificateurEnregistreur>,
    }

    fn nouveau_contexte() -> Contexte {
        // Sans abonné `tracing` actif (celui de `main.rs` n'est jamais
        // exécuté ici), les macros `tracing::info!`/`debug!` considèrent
        // qu'aucun abonné n'est intéressé et n'évaluent même pas leurs
        // arguments — ex. `nouveau_role = utilisateur.role.as_str()` dans
        // `utilisateurs_controller::changer_role` ne s'exécuterait jamais
        // pendant les tests. `try_init` ignore silencieusement le cas où
        // un abonné est déjà en place (appelé une fois par test).
        let _ = tracing_subscriber::fmt::try_init();

        let depot_concret = Arc::new(FakeUtilisateurRepository::new());
        let depot: Arc<dyn UtilisateurRepositoryPort> = depot_concret.clone();
        let secret = "secret-de-test".to_string();
        let jwt_service = Arc::new(JwtService::new(secret.clone(), 24));
        let auth_service = AuthService::new(depot.clone(), jwt_service.clone());
        let notificateur = Arc::new(NotificateurEnregistreur::default());
        let state = AppState {
            auth_service,
            depot_utilisateurs: depot,
            jwt_service,
            notificateur_suppression: notificateur.clone(),
        };
        Contexte {
            app: construire(state.clone(), None),
            state,
            depot_concret,
            secret,
            notificateur,
        }
    }

    #[tokio::test]
    async fn suppression_entre_extraction_et_handler_retourne_une_erreur() {
        use super::auth_controller::deconnexion_partout;
        use crate::domain::utilisateur::UtilisateurPublic;
        use crate::interface::http::middleware::UtilisateurAuthentifie;
        let ctx = nouveau_contexte();
        let user = ctx
            .depot_concret
            .creer("deleted_race", "deleted_race@example.com", "hash")
            .await
            .unwrap();
        let session = UtilisateurAuthentifie {
            id_utilisateur: user.id,
            pseudo: user.pseudo.clone(),
            role: user.role(),
            utilisateur: UtilisateurPublic::from(user.clone()),
        };
        ctx.depot_concret.supprimer(user.id).await.unwrap();
        assert!(matches!(
            ctx.state
                .auth_service
                .modifier_profil(
                    user.id,
                    crate::application::dto::ModifierProfilRequest {
                        pseudo: Some("new_pseudo".into()),
                        mot_de_passe: None,
                        mot_de_passe_actuel: None,
                    }
                )
                .await,
            Err(AppError::Introuvable(_))
        ));
        assert!(matches!(
            deconnexion_partout(State(ctx.state.clone()), session).await,
            Err(AppError::NonAutorise(_))
        ));
        let edit = crate::application::dto::ModifierProfilRequest {
            pseudo: None,
            mot_de_passe: Some("new-password".into()),
            mot_de_passe_actuel: Some("password-fixture".into()),
        };
        assert!(matches!(
            ctx.state.auth_service.modifier_profil(user.id, edit).await,
            Err(AppError::Introuvable(_))
        ));
        assert!(matches!(
            ctx.state
                .auth_service
                .supprimer_compte(
                    user.id,
                    crate::application::dto::SupprimerCompteRequest {
                        mot_de_passe_actuel: "password-fixture".into()
                    }
                )
                .await,
            Err(AppError::Introuvable(_))
        ));
    }

    #[tokio::test]
    async fn changement_concurrent_du_mot_de_passe_empeche_la_modification_de_profil() {
        let ctx = nouveau_contexte();
        let (_, token) = inscrire(
            &ctx.app,
            "profile_race",
            "profile_race@example.com",
            "password-fixture",
        )
        .await;
        ctx.depot_concret
            .provoquer_changement_mot_de_passe_avant_modification();
        let (status, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&token),
            Some(json!({
                "mot_de_passe":"new-password", "mot_de_passe_actuel":"password-fixture"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            requete(&ctx.app, "GET", "/api/auth/me", Some(&token), None)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn pannes_apres_authentification_respectent_le_contrat_500() {
        let ctx = nouveau_contexte();
        let (id, _) = inscrire(
            &ctx.app,
            "admin_errors",
            "admin_errors@example.com",
            "password-fixture",
        )
        .await;
        let token = promouvoir_admin_et_relogin(&ctx, id, "admin_errors", "password-fixture").await;
        let (target, _) = inscrire(
            &ctx.app,
            "target_errors",
            "target_errors@example.com",
            "password-fixture",
        )
        .await;
        // JWT avec famille : deux lectures (utilisateur + session) avant chaque handler.
        for (method, path, body, reads) in [
            (
                "POST",
                "/api/users".to_string(),
                Some(
                    json!({"pseudo":"new_user", "courriel":"new_user@example.com", "mot_de_passe":"password-fixture"}),
                ),
                2,
            ),
            ("GET", "/api/users".to_string(), None, 2),
            ("GET", format!("/api/users/{target}"), None, 2),
            ("GET", format!("/api/users/lookup?ids={target}"), None, 2),
            ("GET", "/api/users/search?q=target".to_string(), None, 2),
            ("DELETE", format!("/api/users/{target}"), None, 2),
            (
                "PATCH",
                format!("/api/users/{target}/role"),
                Some(json!({"role":"admin"})),
                2,
            ),
            (
                "PATCH",
                format!("/api/users/{target}/role"),
                Some(json!({"role":"admin"})),
                3,
            ),
            ("POST", "/api/auth/logout-all".to_string(), None, 2),
            (
                "PATCH",
                "/api/auth/me".to_string(),
                Some(
                    json!({"mot_de_passe":"new-password", "mot_de_passe_actuel":"password-fixture"}),
                ),
                2,
            ),
            (
                "PATCH",
                "/api/auth/me".to_string(),
                Some(
                    json!({"mot_de_passe":"new-password", "mot_de_passe_actuel":"password-fixture"}),
                ),
                3,
            ),
            (
                "DELETE",
                "/api/auth/me".to_string(),
                Some(json!({"mot_de_passe_actuel":"password-fixture"})),
                2,
            ),
            (
                "DELETE",
                "/api/auth/me".to_string(),
                Some(json!({"mot_de_passe_actuel":"password-fixture"})),
                3,
            ),
        ] {
            ctx.depot_concret.provoquer_erreur_interne_apres(reads);
            let (status, response) = requete(&ctx.app, method, &path, Some(&token), body).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{method} {path}");
            assert!(response["erreur"].is_string());
        }
        ctx.depot_concret.provoquer_erreur_interne(false);
        assert_eq!(
            requete(
                &ctx.app,
                "POST",
                "/api/users",
                Some(&token),
                Some(json!({"pseudo":"a","courriel":"invalid","mot_de_passe":"short"}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }

    /// Envoie une requête au `Router` et retourne `(statut, corps_json)`.
    /// `corps_json` vaut `Value::Null` pour une réponse sans corps (ex.
    /// `204 No Content`).
    async fn requete(
        app: &Router,
        methode: &str,
        uri: &str,
        jeton: Option<&str>,
        corps: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(methode).uri(uri);
        if let Some(jeton) = jeton {
            builder = builder.header("Authorization", format!("Bearer {jeton}"));
        }
        let requete = match corps {
            Some(corps) => builder
                .header("content-type", "application/json")
                .body(Body::from(corps.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };

        let reponse = app.clone().oneshot(requete).await.unwrap();
        let statut = reponse.status();
        let octets = reponse.into_body().collect().await.unwrap().to_bytes();
        let corps_json = if octets.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&octets).unwrap_or(Value::Null)
        };
        (statut, corps_json)
    }

    /// Inscrit un compte et retourne `(id, jeton)`.
    async fn inscrire(
        app: &Router,
        pseudo: &str,
        courriel: &str,
        mot_de_passe: &str,
    ) -> (Uuid, String) {
        let (statut, corps) = requete(
            app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": pseudo, "courriel": courriel, "mot_de_passe": mot_de_passe })),
        )
        .await;
        assert_eq!(
            statut,
            StatusCode::CREATED,
            "échec d'inscription inattendu: {corps:?}"
        );
        let id = Uuid::parse_str(corps["utilisateur"]["id"].as_str().unwrap()).unwrap();
        let jeton = corps["jeton"].as_str().unwrap().to_string();
        (id, jeton)
    }

    /// Promeut un compte admin directement via le dépôt (voir README :
    /// c'est une opération manuelle, il n'existe pas de route d'API pour
    /// s'auto-promouvoir), puis se reconnecte pour obtenir un jeton à la
    /// nouvelle `version_auth`.
    async fn promouvoir_admin_et_relogin(
        ctx: &Contexte,
        id: Uuid,
        pseudo: &str,
        mot_de_passe: &str,
    ) -> String {
        ctx.depot_concret
            .changer_role_si_non_admin(id, Role::Admin)
            .await
            .unwrap();
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": pseudo, "mot_de_passe": mot_de_passe })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        corps["jeton"].as_str().unwrap().to_string()
    }

    /// Fabrique un jeton syntaxiquement valide (signé avec le même
    /// secret que `ctx`) mais dont le contenu est choisi par l'appelant —
    /// utile pour atteindre des branches d'erreur du middleware
    /// impossibles à obtenir via `JwtService::generer_token` (ex. un
    /// `sub` qui n'est pas un UUID).
    fn forger_jeton(ctx: &Contexte, claims: Claims) -> String {
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(ctx.secret.as_bytes()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn sante_repond_ok() {
        let ctx = nouveau_contexte();
        let reponse = ctx
            .app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reponse.status(), StatusCode::OK);
        let octets = reponse.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&octets[..], b"ok");
    }

    #[tokio::test]
    async fn sante_signale_une_base_de_donnees_inaccessible() {
        let ctx = nouveau_contexte();
        ctx.depot_concret.provoquer_erreur_interne(true);

        let reponse = ctx
            .app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reponse.status(), StatusCode::SERVICE_UNAVAILABLE);
        let octets = reponse.into_body().collect().await.unwrap().to_bytes();
        let corps: Value = serde_json::from_slice(&octets).unwrap();
        assert_eq!(corps["erreur"], "base de données inaccessible");
    }

    #[tokio::test]
    async fn route_inconnue_et_methode_non_autorisee_respectent_lenveloppe_erreur() {
        let ctx = nouveau_contexte();

        let (statut, corps) = requete(&ctx.app, "GET", "/inconnue", None, None).await;
        assert_eq!(statut, StatusCode::NOT_FOUND);
        assert_eq!(corps["erreur"], "route introuvable");

        let (statut, corps) = requete(&ctx.app, "PUT", "/health", None, None).await;
        assert_eq!(statut, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(corps["erreur"], "méthode HTTP non autorisée");
    }

    #[tokio::test]
    async fn cors_restreint_aux_origines_configurees() {
        let depot: Arc<dyn UtilisateurRepositoryPort> = Arc::new(FakeUtilisateurRepository::new());
        let jwt_service = Arc::new(JwtService::new("secret-de-test".to_string(), 24));
        let auth_service = AuthService::new(depot.clone(), jwt_service.clone());
        let state = AppState {
            auth_service,
            depot_utilisateurs: depot,
            jwt_service,
            notificateur_suppression: Arc::new(NotificateurEnregistreur::default()),
        };
        let origines = vec!["http://exemple-autorise.test".to_string()];
        let app = construire(state, Some(&origines));

        let reponse_autorisee = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("Origin", "http://exemple-autorise.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            reponse_autorisee
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "http://exemple-autorise.test"
        );

        let reponse_refusee = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("Origin", "http://exemple-non-autorise.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(reponse_refusee
            .headers()
            .get("access-control-allow-origin")
            .is_none());
    }

    #[tokio::test]
    async fn cors_ignore_une_origine_invalide_sans_bloquer_les_autres() {
        let depot: Arc<dyn UtilisateurRepositoryPort> = Arc::new(FakeUtilisateurRepository::new());
        let jwt_service = Arc::new(JwtService::new("secret-de-test".to_string(), 24));
        let auth_service = AuthService::new(depot.clone(), jwt_service.clone());
        let state = AppState {
            auth_service,
            depot_utilisateurs: depot,
            jwt_service,
            notificateur_suppression: Arc::new(NotificateurEnregistreur::default()),
        };
        // Un caractère de contrôle (retour à la ligne) ne parse jamais en
        // `HeaderValue` valide — simule une faute de frappe dans
        // `CORS_ORIGINS` (voir le `tracing::warn!` dans `construire`).
        let origines = vec![
            "http://exemple-autorise.test".to_string(),
            "origine\ninvalide".to_string(),
        ];
        let app = construire(state, Some(&origines));

        let reponse = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("Origin", "http://exemple-autorise.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            reponse
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "http://exemple-autorise.test"
        );
    }

    /// Construit un contexte + Router avec une liste d'origines CORS
    /// donnée, sans passer par `nouveau_contexte()` (qui force `None`).
    fn app_avec_origines(origines: &[String]) -> Router {
        let depot: Arc<dyn UtilisateurRepositoryPort> = Arc::new(FakeUtilisateurRepository::new());
        let jwt_service = Arc::new(JwtService::new("secret-de-test".to_string(), 24));
        let auth_service = AuthService::new(depot.clone(), jwt_service.clone());
        let state = AppState {
            auth_service,
            depot_utilisateurs: depot,
            jwt_service,
            notificateur_suppression: Arc::new(NotificateurEnregistreur::default()),
        };
        construire(state, Some(origines))
    }

    /// Retourne `true` si une réponse à `GET /health` avec l'en-tête
    /// `Origin` donné ne porte pas `Access-Control-Allow-Origin`.
    async fn cors_est_refuse(app: Router, origine_testee: &str) -> bool {
        let reponse = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("Origin", origine_testee)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        reponse
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    }

    #[tokio::test]
    async fn cors_refuse_toute_origine_si_la_variable_est_absente() {
        let ctx = nouveau_contexte();
        assert!(cors_est_refuse(ctx.app, "http://nimporte-quoi.test").await);
    }

    #[tokio::test]
    async fn cors_refuse_toute_origine_si_la_liste_est_vide_apres_nettoyage() {
        // Simule `CORS_ORIGINS=,,,` : `config.rs` filtre déjà les
        // entrées vides, donnant `Some(vec![])` — distinct de `None`
        // (variable absente). Les deux doivent refuser toute origine.
        let app = app_avec_origines(&[]);
        assert!(cors_est_refuse(app, "http://nimporte-quoi.test").await);
    }

    #[tokio::test]
    async fn cors_refuse_toute_origine_si_toutes_les_origines_sont_invalides() {
        let origines = vec![
            "origine\ninvalide".to_string(),
            "encore\ninvalide".to_string(),
        ];
        let app = app_avec_origines(&origines);
        assert!(cors_est_refuse(app, "http://nimporte-quoi.test").await);
    }

    #[tokio::test]
    async fn corps_de_requete_trop_volumineux_est_refuse() {
        let ctx = nouveau_contexte();
        // Dépasse LIMITE_CORPS_REQUETE_OCTETS (16 Kio) — un mot de passe
        // de cette taille serait de toute façon rejeté par `validator`
        // (max 128 caractères), mais la limite de taille de corps doit
        // intervenir avant même la désérialisation JSON.
        let corps_volumineux = "a".repeat(20 * 1024);
        let requete = Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                "{{\"pseudo\":\"x\",\"courriel\":\"x@example.com\",\"mot_de_passe\":\"{corps_volumineux}\"}}"
            )))
            .unwrap();
        let reponse = ctx.app.oneshot(requete).await.unwrap();
        assert_eq!(reponse.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let corps = reponse.into_body().collect().await.unwrap().to_bytes();
        let corps: serde_json::Value = serde_json::from_slice(&corps).unwrap();
        assert_eq!(corps["erreur"], "corps de requête trop volumineux");
    }

    #[tokio::test]
    async fn inscription_puis_connexion_par_pseudo_et_par_courriel() {
        let ctx = nouveau_contexte();
        let (_, _jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["utilisateur"]["pseudo"], "alice");

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice@example.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
    }

    #[tokio::test]
    async fn inscription_rejette_un_pseudo_trop_court() {
        let ctx = nouveau_contexte();
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": "ab", "courriel": "ab@example.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        // Vérifie le *contenu*, pas seulement le statut : `validator::ValidationErrors`
        // implémente `Display` en utilisant le `message` personnalisé de
        // chaque champ (voir `application/dto.rs`) — mais retomberait sur un
        // format technique illisible (`Validation error: length [...]`) si
        // un message venait à être retiré par erreur. Ce test empêche une
        // telle régression de passer inaperçue.
        assert_eq!(
            corps["erreur"],
            "pseudo: le pseudo doit contenir entre 3 et 20 caractères"
        );
    }

    #[tokio::test]
    async fn inscription_rejette_un_pseudo_avec_des_caracteres_interdits() {
        let ctx = nouveau_contexte();
        // Liste blanche (alphanumérique + `_`/`-`) plutôt que liste noire :
        // un pseudo est affiché tel quel dans le panneau d'administration
        // de l'équipe — `<script>` doit être rejeté avant même d'atteindre
        // la base de données.
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "pseudo": "<script>",
                "courriel": "xss@example.com",
                "mot_de_passe": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "pseudo: le pseudo ne peut contenir que des lettres, chiffres, '_' et '-'"
        );

        // Alphanumérique + `_`/`-` doit rester accepté.
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({
                "pseudo": "alice_bob-42",
                "courriel": "aliceb@example.com",
                "mot_de_passe": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn inscription_rejette_un_corps_json_malforme() {
        let ctx = nouveau_contexte();
        // Champ `courriel` manquant : erreur de désérialisation Serde, pas
        // une erreur de validation `validator` — passe par un chemin de
        // code différent (voir `interface::http::extracteurs::Json`).
        // Sans cet extracteur personnalisé, axum renverrait un corps en
        // texte brut (422), pas l'enveloppe JSON `{ "erreur": ... }`
        // documentée dans docs/API.md.
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": "alice" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(corps["erreur"], "corps JSON invalide");
    }

    #[tokio::test]
    async fn inscription_est_insensible_a_la_casse_du_courriel() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "Alice@Example.com", "motdepasse123").await;

        // Même courriel, casse différente : doit être refusé comme un
        // doublon, pas créer un second compte.
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": "bob", "courriel": "ALICE@example.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::CONFLICT);

        // Connexion avec une casse différente de celle utilisée à
        // l'inscription : doit réussir (le courriel est normalisé en
        // minuscules avant stockage — voir `AuthService::inscrire`).
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice@EXAMPLE.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["utilisateur"]["courriel"], "alice@example.com");
    }

    #[tokio::test]
    async fn connexion_ignore_les_espaces_de_tete_et_de_fin_de_lidentifiant() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // Copié-collé fréquent d'un courriel : un espace de tête/fin ne
        // doit pas faire échouer la connexion.
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(
                json!({ "identifiant": "  alice@example.com  ", "mot_de_passe": "motdepasse123" }),
            ),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
    }

    #[tokio::test]
    async fn inscription_rejette_un_pseudo_deja_pris() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": "alice", "courriel": "autre@example.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn inscription_erreur_de_stockage_retourne_500_generique() {
        let ctx = nouveau_contexte();
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/register",
            None,
            Some(json!({ "pseudo": "alice", "courriel": "alice@example.com", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
        // Le détail de l'erreur ne doit jamais fuiter au client.
        assert_eq!(corps["erreur"], "une erreur interne est survenue");
    }

    #[tokio::test]
    async fn connexion_rejette_un_mauvais_mot_de_passe() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "mauvais-mot-de-passe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn connexion_rejette_un_identifiant_uniquement_compose_despaces() {
        let ctx = nouveau_contexte();

        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({
                "identifiant": "   ",
                "mot_de_passe": "motdepasse123",
            })),
        )
        .await;

        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "identifiant requis (254 caractères maximum)"
        );
    }

    #[tokio::test]
    async fn connexion_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn connexion_rejette_un_identifiant_inconnu() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "fantome", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn connexion_rejette_des_champs_vides() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "", "mot_de_passe": "" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn me_sans_jeton_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", None, None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_avec_jeton_mal_forme_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some("pas-un-jwt"), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_avec_jeton_expire_est_refuse() {
        let ctx = nouveau_contexte();
        // Durée de vie négative : le jeton est expiré dès sa création.
        let service_expire = JwtService::new(ctx.secret.clone(), -1);
        let jeton = service_expire
            .generer_token(Uuid::new_v4(), 0, None)
            .unwrap();

        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_avec_sub_non_uuid_est_refuse() {
        let ctx = nouveau_contexte();
        let jeton = forger_jeton(
            &ctx,
            Claims {
                sub: "pas-un-uuid".into(),
                sid: None,
                jti: None,
                version_auth: 0,
                exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
            },
        );

        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_retourne_le_profil_courant() {
        let ctx = nouveau_contexte();
        let (id, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["id"], id.to_string());
        assert_eq!(corps["pseudo"], "alice");
    }

    #[tokio::test]
    async fn me_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn me_apres_suppression_du_compte_est_non_autorise() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::NO_CONTENT);

        // Le jeton reste cryptographiquement valide (il n'est pas révoqué
        // — voir limites connues du README) : c'est la recherche en base
        // qui doit maintenant échouer.
        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn modifier_moi_met_a_jour_chaque_champ() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["pseudo"], "alice2");

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(
                json!({ "mot_de_passe": "nouveau-mot-de-passe", "mot_de_passe_actuel": "motdepasse123" }),
            ),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);

        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);

        // Le nouveau mot de passe doit désormais permettre la connexion.
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice2", "mot_de_passe": "nouveau-mot-de-passe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
    }

    #[tokio::test]
    async fn modifier_moi_refuse_tout_changement_de_courriel() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // Même avec le bon mot de passe actuel, et même accompagné d'un
        // champ valide : le courriel n'est pas modifiable par l'utilisateur.
        for corps_requete in [
            json!({ "courriel": "alice2@example.com" }),
            json!({ "courriel": "alice2@example.com", "mot_de_passe_actuel": "motdepasse123" }),
            json!({ "pseudo": "alice2", "courriel": "alice2@example.com" }),
        ] {
            let (statut, _) = requete(
                &ctx.app,
                "PATCH",
                "/api/auth/me",
                Some(&jeton),
                Some(corps_requete),
            )
            .await;
            assert_eq!(statut, StatusCode::BAD_REQUEST);
        }

        let (_, corps) = requete(&ctx.app, "GET", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(corps["courriel"], "alice@example.com");
        assert_eq!(corps["pseudo"], "alice");
    }

    #[tokio::test]
    async fn modifier_moi_mot_de_passe_actuel_vide_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe": "nouveau-mot-de-passe", "mot_de_passe_actuel": "" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert!(corps["erreur"]
            .as_str()
            .unwrap()
            .contains("mot de passe actuel"));
    }

    #[tokio::test]
    async fn modifier_moi_mot_de_passe_sans_mot_de_passe_actuel_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe": "nouveau-mot-de-passe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn modifier_moi_avec_mauvais_mot_de_passe_actuel_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({
                "mot_de_passe": "nouveau-mot-de-passe",
                "mot_de_passe_actuel": "mauvais-mot-de-passe",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn modifier_moi_verification_mot_de_passe_actuel_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({
                "mot_de_passe": "nouveau-mot-de-passe",
                "mot_de_passe_actuel": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn modifier_moi_mot_de_passe_apres_suppression_est_non_autorise() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;

        // Le jeton reste cryptographiquement valide (voir limites connues
        // du README) : c'est la vérification du mot de passe actuel, qui
        // doit d'abord relire le compte, qui échoue ici — pas le
        // `depot.modifier` final (jamais atteint).
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({
                "mot_de_passe": "nouveau-mot-de-passe",
                "mot_de_passe_actuel": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn modifier_moi_pseudo_seul_ne_requiert_pas_le_mot_de_passe_actuel() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["pseudo"], "alice2");
    }

    #[tokio::test]
    async fn modifier_moi_pseudo_seul_ignore_un_mot_de_passe_actuel_vide() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "pseudo": "alice2", "mot_de_passe_actuel": "" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["pseudo"], "alice2");
    }

    #[tokio::test]
    async fn modifier_moi_sans_aucun_champ_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({})),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(corps["erreur"], "au moins un champ doit être fourni");
    }

    #[tokio::test]
    async fn modifier_moi_rejette_un_pseudo_deja_pris() {
        let ctx = nouveau_contexte();
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (_, jeton_bob) = inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton_bob),
            Some(json!({ "pseudo": "alice" })),
        )
        .await;
        assert_eq!(statut, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn modifier_moi_sans_jeton_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            None,
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn modifier_moi_apres_suppression_est_non_autorise() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn supprimer_moi_sans_jeton_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            None,
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn supprimer_moi_sans_mot_de_passe_actuel_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(&ctx.app, "DELETE", "/api/auth/me", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn supprimer_moi_avec_un_mauvais_mot_de_passe_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "mauvais-mot-de-passe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn supprimer_moi_deux_fois_refuse_la_seconde_fois() {
        let ctx = nouveau_contexte();
        let (id, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::NO_CONTENT);
        // Les autres services sont prévenus, une seule fois.
        assert_eq!(*ctx.notificateur.ids.lock().unwrap(), vec![id]);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn supprimer_moi_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn routes_admin_refusees_a_un_utilisateur_normal() {
        let ctx = nouveau_contexte();
        let (id, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, _) = requete(&ctx.app, "GET", "/api/users", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::FORBIDDEN);

        let (statut, _) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/{id}"),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/users",
            Some(&jeton),
            Some(json!({
                "pseudo": "bob",
                "courriel": "bob@example.com",
                "mot_de_passe": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id}"),
            Some(&jeton),
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id}"),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id}/role"),
            Some(&jeton),
            Some(json!({ "role": "admin" })),
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn routes_admin_refusent_un_non_admin_avant_lextraction_du_corps_ou_du_path() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let reponse = ctx
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/users/pas-un-uuid/role")
                    .header("Authorization", format!("Bearer {jeton}"))
                    .header("content-type", "application/json")
                    .body(Body::from("pas-du-json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reponse.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn routes_admin_refusees_sans_jeton() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "GET", "/api/users", None, None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_retrograde_perd_immediatement_lacces_avec_son_ancien_jeton() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        ctx.depot_concret
            .changer_role_pour_test(id_admin, Role::Utilisateur)
            .await;

        let (statut, _) = requete(&ctx.app, "GET", "/api/users", Some(&jeton_admin), None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_ne_peut_pas_modifier_son_propre_compte_via_les_routes_admin() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_admin}"),
            Some(&jeton_admin),
            Some(json!({ "courriel": "admin2@example.com" })),
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_ne_peut_pas_modifier_son_propre_compte_avec_un_corps_invalide() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        let reponse = ctx
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/users/{id_admin}"))
                    .header("Authorization", format!("Bearer {jeton_admin}"))
                    .header("content-type", "application/json")
                    .body(Body::from("pas-du-json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(reponse.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_ne_peut_pas_supprimer_son_propre_compte_via_les_routes_admin() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id_admin}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_liste_et_consulte_les_comptes() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) =
            requete(&ctx.app, "GET", "/api/users", Some(&jeton_admin), None).await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps.as_array().unwrap().len(), 2);

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["pseudo"], "alice");

        let id_inconnu = Uuid::new_v4();
        let (statut, _) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/{id_inconnu}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_cree_un_compte_standard_sans_le_connecter() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/users",
            Some(&jeton_admin),
            Some(json!({
                "pseudo": "alice",
                "courriel": "Alice@Example.com",
                "mot_de_passe": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::CREATED);
        assert_eq!(corps["pseudo"], "alice");
        assert_eq!(corps["courriel"], "alice@example.com");
        assert_eq!(corps["role"], "utilisateur");
        assert!(corps.get("jeton").is_none());
    }

    #[tokio::test]
    async fn admin_modifie_un_autre_compte() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (id_bob, _) = inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            Some(json!({ "courriel": "alice-modifiee@example.com" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["courriel"], "alice-modifiee@example.com");

        // Aucun champ fourni.
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            Some(json!({})),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);

        // Validation (pseudo trop court).
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            Some(json!({ "pseudo": "ab" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);

        // Conflit : bob prend le pseudo d'alice.
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_bob}"),
            Some(&jeton_admin),
            Some(json!({ "pseudo": "alice" })),
        )
        .await;
        assert_eq!(statut, StatusCode::CONFLICT);

        // Compte inexistant.
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{}", Uuid::new_v4()),
            Some(&jeton_admin),
            Some(json!({ "pseudo": "quelquun" })),
        )
        .await;
        assert_eq!(statut, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn changer_role_avec_erreur_de_stockage_sur_la_cible_retourne_500() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        // 1er appel : session de l'admin ; 2e : chargement de la cible.
        ctx.depot_concret.provoquer_erreur_interne_apres(1);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "utilisateur" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn admin_supprime_un_compte_et_change_un_role() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "utilisateur" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["role"], "utilisateur");

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{}/role", Uuid::new_v4()),
            Some(&jeton_admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
        assert_eq!(statut, StatusCode::NOT_FOUND);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::NO_CONTENT);
        assert_eq!(*ctx.notificateur.ids.lock().unwrap(), vec![id_alice]);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::NOT_FOUND);
        // Rien de supprimé : personne n'est prévenu une seconde fois.
        assert_eq!(ctx.notificateur.ids.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn suppression_refusee_ne_previent_personne() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "mauvais-mot-de-passe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
        assert!(ctx.notificateur.ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn deconnexion_partout_invalide_tous_les_jetons() {
        let ctx = nouveau_contexte();
        let (_, jeton_a) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (statut, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        let jeton_b = corps["jeton"].as_str().unwrap().to_string();

        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/logout-all",
            Some(&jeton_a),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::NO_CONTENT);

        for jeton in [&jeton_a, &jeton_b] {
            let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(jeton), None).await;
            assert_eq!(statut, StatusCode::UNAUTHORIZED);
        }

        // Une nouvelle connexion redonne un jeton valide.
        let (_, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        let jeton_c = corps["jeton"].as_str().unwrap();
        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(jeton_c), None).await;
        assert_eq!(statut, StatusCode::OK);
    }

    #[tokio::test]
    async fn deconnexion_partout_exige_un_jeton_et_gere_une_erreur_de_stockage() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "POST", "/api/auth/logout-all", None, None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);

        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        // L'authentification lit le dépôt une fois ; l'invalidation échoue.
        ctx.depot_concret.provoquer_erreur_interne_apres(1);
        let (statut, _) =
            requete(&ctx.app, "POST", "/api/auth/logout-all", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn changer_role_rejette_une_valeur_de_role_inconnue() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // "superadmin" n'existe pas dans l'énumération Role : erreur de
        // désérialisation Serde (voir extracteurs::Json), pas une entrée
        // qui atteint `AuthService`.
        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "superadmin" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(corps["erreur"], "corps JSON invalide");

        // Le modérateur n'est plus un rôle global : il n'existe que dans
        // les serveurs (Room-Service).
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "moderateur" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn admin_ne_peut_pas_changer_le_role_dun_autre_admin() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_autre_admin, _) =
            inscrire(&ctx.app, "admin2", "admin2@example.com", "motdepasse123").await;
        ctx.depot_concret
            .changer_role_si_non_admin(id_autre_admin, Role::Admin)
            .await
            .unwrap();

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_autre_admin}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "utilisateur" })),
        )
        .await;

        assert_eq!(statut, StatusCode::FORBIDDEN);
        assert_eq!(
            corps["erreur"],
            "le rôle d'un administrateur ne peut pas être modifié"
        );
    }

    #[tokio::test]
    async fn changement_role_retourne_404_si_la_cible_est_supprimee_pendant_la_requete() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        ctx.depot_concret
            .provoquer_suppression_avant_changement_role();

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "utilisateur" })),
        )
        .await;

        assert_eq!(statut, StatusCode::NOT_FOUND);
        assert_eq!(corps["erreur"], "utilisateur introuvable");
    }

    #[tokio::test]
    async fn changement_role_retourne_403_si_la_cible_devient_admin_pendant_la_requete() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_alice, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // L'extracteur voit une cible non admin, mais elle est promue admin
        // avant la mutation atomique : celle-ci doit refuser.
        ctx.depot_concret
            .provoquer_promotion_admin_avant_changement_role();

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_alice}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "utilisateur" })),
        )
        .await;

        assert_eq!(statut, StatusCode::FORBIDDEN);
        assert_eq!(
            corps["erreur"],
            "le rôle d'un administrateur ne peut pas être modifié"
        );
    }

    #[tokio::test]
    async fn supprimer_moi_refuse_si_le_mot_de_passe_change_pendant_la_requete() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // Mot de passe actuel correct à la vérification, mais changé (autre
        // appareil) avant la suppression atomique : le compte est gardé.
        ctx.depot_concret
            .provoquer_changement_mot_de_passe_avant_suppression();

        let (statut, corps) = requete(
            &ctx.app,
            "DELETE",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "mot_de_passe_actuel": "motdepasse123" })),
        )
        .await;

        assert_eq!(statut, StatusCode::UNAUTHORIZED);
        assert_eq!(corps["erreur"], "mot de passe actuel incorrect");
    }

    #[tokio::test]
    async fn segment_id_non_uuid_est_rejete_proprement() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;

        // Un `id` qui n'est pas un UUID valide doit renvoyer l'enveloppe
        // JSON standard, pas le corps en texte brut par défaut d'axum
        // (voir extracteurs::Path).
        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            "/api/users/pas-un-uuid",
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(corps["erreur"], "paramètre d'URL invalide");
    }

    #[tokio::test]
    async fn actions_admin_avec_erreur_de_stockage_retournent_500() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (id_cible, _) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(&ctx.app, "GET", "/api/users", Some(&jeton_admin), None).await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);

        let (statut, _) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/{id_cible}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_cible}"),
            Some(&jeton_admin),
            Some(json!({ "pseudo": "peu-importe" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id_cible}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{id_cible}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // --- GET /api/users/lookup ------------------------------------------
    //
    // Contrairement au reste de `/api/users`, cette route n'exige qu'une
    // authentification valide (`UtilisateurAuthentifie`), pas le rôle
    // admin — voir `utilisateurs_controller::rechercher` et docs/API.md.

    #[tokio::test]
    async fn lookup_resout_un_seul_id_pour_un_utilisateur_non_admin() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton_alice) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={id_alice}"),
            Some(&jeton_alice),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        let tableau = corps.as_array().unwrap();
        assert_eq!(tableau.len(), 1);
        assert_eq!(tableau[0]["id"], id_alice.to_string());
        assert_eq!(tableau[0]["pseudo"], "alice");
        // Rien d'autre que id/pseudo : ni courriel, ni role, ni horodatages.
        assert_eq!(
            tableau[0]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn lookup_resout_plusieurs_ids_et_omet_les_ids_inconnus() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton_alice) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (id_bob, _) = inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;
        let id_inconnu = Uuid::new_v4();

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={id_alice},{id_inconnu},{id_bob}"),
            Some(&jeton_alice),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        let tableau = corps.as_array().unwrap();
        // L'id inconnu est omis (pas une erreur, pas de 404 partiel).
        assert_eq!(tableau.len(), 2);
        let pseudos: Vec<&str> = tableau
            .iter()
            .map(|u| u["pseudo"].as_str().unwrap())
            .collect();
        assert!(pseudos.contains(&"alice"));
        assert!(pseudos.contains(&"bob"));
    }

    #[tokio::test]
    async fn lookup_aucun_id_connu_retourne_un_tableau_vide() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={}", Uuid::new_v4()),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn lookup_sans_jeton_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={}", Uuid::new_v4()),
            None,
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn lookup_sans_parametre_ids_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) =
            requete(&ctx.app, "GET", "/api/users/lookup", Some(&jeton), None).await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "le paramètre 'ids' est requis et ne peut pas être vide"
        );
    }

    #[tokio::test]
    async fn lookup_avec_parametre_ids_vide_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            "/api/users/lookup?ids=",
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "le paramètre 'ids' est requis et ne peut pas être vide"
        );
    }

    #[tokio::test]
    async fn lookup_avec_une_virgule_superflue_est_refuse() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={id_alice},"),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert!(corps["erreur"].as_str().unwrap().contains("valeur vide"));
    }

    #[tokio::test]
    async fn lookup_avec_un_id_malforme_rejette_toute_la_requete() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        // Un seul id invalide dans la liste doit rejeter toute la requête
        // (choix documenté dans docs/API.md et utilisateurs_controller::parser_ids),
        // pas seulement ignorer cette entrée.
        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={id_alice},pas-un-uuid"),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "'pas-un-uuid' dans le paramètre 'ids' n'est pas un UUID valide"
        );
    }

    #[tokio::test]
    async fn lookup_avec_trop_dids_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let trop_dids = (0..101)
            .map(|_| Uuid::new_v4().to_string())
            .collect::<Vec<_>>()
            .join(",");
        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={trop_dids}"),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(
            corps["erreur"],
            "le paramètre 'ids' accepte au plus 100 identifiants"
        );
    }

    #[tokio::test]
    async fn lookup_accepte_des_espaces_autour_des_virgules() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton_alice) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (id_bob, _) = inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;

        // Espaces encodés en `%20` : un `http::Uri` brut n'accepte pas
        // d'espace non encodé dans la chaîne de requête.
        let (statut, corps) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids=%20{id_alice}%20,%20{id_bob}%20"),
            Some(&jeton_alice),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn lookup_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);

        let (statut, _) = requete(
            &ctx.app,
            "GET",
            &format!("/api/users/lookup?ids={}", Uuid::new_v4()),
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // --- GET /api/users/search ------------------------------------------
    //
    // Recherche par début de pseudo, ouverte à tout utilisateur authentifié
    // (voir `utilisateurs_controller::rechercher_par_pseudo`).

    async fn chercher(ctx: &Contexte, jeton: &str, query: &str) -> (StatusCode, Value) {
        requete(
            &ctx.app,
            "GET",
            &format!("/api/users/search{query}"),
            Some(jeton),
            None,
        )
        .await
    }

    #[tokio::test]
    async fn search_trouve_par_prefixe_insensible_a_la_casse_et_trie() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "zoe", "zoe@example.com", "motdepasse123").await;
        inscrire(&ctx.app, "Alicia", "alicia@example.com", "motdepasse123").await;
        inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;

        let (statut, corps) = chercher(&ctx, &jeton, "?q=ALI").await;
        assert_eq!(statut, StatusCode::OK);
        let tableau = corps.as_array().unwrap();
        let pseudos: Vec<&str> = tableau
            .iter()
            .map(|u| u["pseudo"].as_str().unwrap())
            .collect();
        assert_eq!(pseudos, ["alice", "Alicia"]);
        // Rien d'autre que id/pseudo.
        assert_eq!(tableau[0].as_object().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn search_n_exclut_pas_l_appelant_et_respecte_limit() {
        let ctx = nouveau_contexte();
        let (id_alice, jeton) =
            inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        inscrire(&ctx.app, "alicia", "alicia@example.com", "motdepasse123").await;

        let (statut, corps) = chercher(&ctx, &jeton, "?q=ali&limit=1").await;
        assert_eq!(statut, StatusCode::OK);
        let tableau = corps.as_array().unwrap();
        assert_eq!(tableau.len(), 1);
        assert_eq!(tableau[0]["id"], id_alice.to_string());
    }

    #[tokio::test]
    async fn search_aucun_resultat_retourne_un_tableau_vide() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (statut, corps) = chercher(&ctx, &jeton, "?q=zzz").await;
        assert_eq!(statut, StatusCode::OK);
        assert!(corps.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn search_ignore_les_espaces_autour_du_texte() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let (statut, corps) = chercher(&ctx, &jeton, "?q=%20ali%20").await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_sans_jeton_est_refuse() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "GET", "/api/users/search?q=a", None, None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn search_rejette_les_parametres_invalides() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        let trop_long = "a".repeat(21);
        for query in [
            String::new(),
            "?q=".into(),
            "?q=%20%20".into(),
            format!("?q={trop_long}"),
            "?q=a&limit=0".into(),
            "?q=a&limit=51".into(),
            "?q=a&limit=abc".into(),
        ] {
            let (statut, _) = chercher(&ctx, &jeton, &query).await;
            assert_eq!(statut, StatusCode::BAD_REQUEST, "requête {query:?}");
        }
    }

    #[tokio::test]
    async fn search_avec_erreur_de_stockage_retourne_500() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        ctx.depot_concret.provoquer_erreur_interne(true);
        let (statut, _) = chercher(&ctx, &jeton, "?q=a").await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn deconnexion_des_autres_garde_seulement_l_appareil_courant() {
        let ctx = nouveau_contexte();
        let (id, jeton_a) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        // Un second appareil connecté.
        let (_, autre) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice", "mot_de_passe": "motdepasse123" })),
        )
        .await;
        let jeton_b = autre["jeton"].as_str().unwrap().to_string();
        let refresh_b = autre["refresh_token"].as_str().unwrap().to_string();

        let (statut, nouvelle) = requete(
            &ctx.app,
            "POST",
            "/api/auth/logout-others",
            Some(&jeton_a),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(nouvelle["utilisateur"]["id"], id.to_string());
        let jeton_c = nouvelle["jeton"].as_str().unwrap();
        let refresh_c = nouvelle["refresh_token"].as_str().unwrap();

        // Anciens jetons (les deux appareils) refusés, refresh de l'autre appareil aussi.
        for jeton in [&jeton_a, &jeton_b] {
            let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(jeton), None).await;
            assert_eq!(statut, StatusCode::UNAUTHORIZED);
        }
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/refresh",
            None,
            Some(json!({ "refresh_token": refresh_b })),
        )
        .await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);
        // La nouvelle session de l'appareil courant est valide et renouvelable.
        let (statut, _) = requete(&ctx.app, "GET", "/api/auth/me", Some(jeton_c), None).await;
        assert_eq!(statut, StatusCode::OK);
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/refresh",
            None,
            Some(json!({ "refresh_token": refresh_c })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(*ctx.notificateur.sessions.lock().unwrap(), vec![id]);
    }

    #[tokio::test]
    async fn deconnexion_des_autres_exige_un_jeton_et_gere_les_erreurs() {
        let ctx = nouveau_contexte();
        let (statut, _) = requete(&ctx.app, "POST", "/api/auth/logout-others", None, None).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED);

        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        // L'authentification lit le dépôt une fois ; l'invalidation échoue.
        ctx.depot_concret.provoquer_erreur_interne_apres(1);
        let (statut, _) = requete(
            &ctx.app,
            "POST",
            "/api/auth/logout-others",
            Some(&jeton),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(ctx.notificateur.sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn deconnexion_des_autres_d_un_compte_supprime_entre_temps() {
        use super::auth_controller::deconnexion_autres;
        use crate::interface::http::middleware::UtilisateurAuthentifie;
        let ctx = nouveau_contexte();
        let user = ctx
            .depot_concret
            .creer("parti", "parti@example.com", "hash")
            .await
            .unwrap();
        let session = UtilisateurAuthentifie {
            id_utilisateur: user.id,
            utilisateur: crate::domain::utilisateur::UtilisateurPublic::from(user.clone()),
            pseudo: user.pseudo.clone(),
            role: user.role(),
        };
        ctx.depot_concret.supprimer(user.id).await.unwrap();
        let resultat = deconnexion_autres(axum::extract::State(ctx.state.clone()), session).await;
        assert!(matches!(resultat, Err(AppError::NonAutorise(_))));
    }

    #[tokio::test]
    async fn mot_de_passe_change_et_deconnexion_partout_previennent_message_service() {
        let ctx = nouveau_contexte();
        let (id, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;
        // Pseudo seul : les jetons restent valides, rien à signaler.
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "pseudo": "alice2" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert!(ctx.notificateur.sessions.lock().unwrap().is_empty());

        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(
                json!({ "mot_de_passe": "nouveaumdp123", "mot_de_passe_actuel": "motdepasse123" }),
            ),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(*ctx.notificateur.sessions.lock().unwrap(), vec![id]);

        let (_, corps) = requete(
            &ctx.app,
            "POST",
            "/api/auth/login",
            None,
            Some(json!({ "identifiant": "alice2", "mot_de_passe": "nouveaumdp123" })),
        )
        .await;
        let jeton = corps["jeton"].as_str().unwrap();
        let (statut, _) =
            requete(&ctx.app, "POST", "/api/auth/logout-all", Some(jeton), None).await;
        assert_eq!(statut, StatusCode::NO_CONTENT);
        assert_eq!(*ctx.notificateur.sessions.lock().unwrap(), vec![id, id]);
    }

    #[tokio::test]
    async fn changement_de_role_previent_message_service() {
        let ctx = nouveau_contexte();
        let (id_admin, _) =
            inscrire(&ctx.app, "admin1", "admin1@example.com", "motdepasse123").await;
        let jeton_admin =
            promouvoir_admin_et_relogin(&ctx, id_admin, "admin1", "motdepasse123").await;
        let (cible, _) = inscrire(&ctx.app, "bob", "bob@example.com", "motdepasse123").await;
        let (statut, _) = requete(
            &ctx.app,
            "PATCH",
            &format!("/api/users/{cible}/role"),
            Some(&jeton_admin),
            Some(json!({ "role": "admin" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(*ctx.notificateur.sessions.lock().unwrap(), vec![cible]);
    }
}
