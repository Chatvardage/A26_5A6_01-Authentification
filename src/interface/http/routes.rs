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
/// d'administration via `CORS_ALLOWED_ORIGINS` (voir `config.rs`).
pub fn construire(state: AppState, origines_autorisees: Option<&[String]>) -> Router {
    let api_auth = Router::new()
        .route("/register", post(auth_controller::register))
        .route("/login", post(auth_controller::login))
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
            tracing::warn!("CORS_ALLOWED_ORIGINS est absente — aucune origine n'est autorisée");
            cors_restreint()
        }
        Some([]) => {
            tracing::warn!(
                "CORS_ALLOWED_ORIGINS est définie mais ne contient aucune origine (ex. \
                 \"CORS_ALLOWED_ORIGINS=,,,\") — aucune origine n'est autorisée"
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
                        tracing::warn!(
                            origine = o,
                            "CORS_ALLOWED_ORIGINS: origine invalide, ignorée"
                        );
                        None
                    }
                })
                .collect();
            if valeurs.is_empty() {
                tracing::warn!(
                    "CORS_ALLOWED_ORIGINS: aucune origine valide après filtrage — aucune \
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

    /// Environnement de test : garde une référence concrète au dépôt
    /// (`depot_concret`) en plus de celle (dynamique) injectée dans
    /// `AppState`, pour pouvoir promouvoir un compte admin directement
    /// (voir README : la promotion admin est une opération manuelle,
    /// hors API) et déclencher `provoquer_erreur_interne` dans les tests
    /// de la branche 500.
    struct Contexte {
        app: Router,
        depot_concret: Arc<FakeUtilisateurRepository>,
        secret: String,
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
        let state = AppState {
            auth_service,
            depot_utilisateurs: depot,
            jwt_service,
        };
        Contexte {
            app: construire(state, None),
            depot_concret,
            secret,
        }
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
            StatusCode::OK,
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
            .changer_role(id, Role::Admin)
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
        };
        // Un caractère de contrôle (retour à la ligne) ne parse jamais en
        // `HeaderValue` valide — simule une faute de frappe dans
        // `CORS_ALLOWED_ORIGINS` (voir le `tracing::warn!` dans `construire`).
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
        // Simule `CORS_ALLOWED_ORIGINS=,,,` : `config.rs` filtre déjà les
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
        assert_eq!(statut, StatusCode::OK);
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
        let jeton = service_expire.generer_token(Uuid::new_v4(), 0).unwrap();

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

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(
                json!({ "courriel": "alice2@example.com", "mot_de_passe_actuel": "motdepasse123" }),
            ),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["courriel"], "alice2@example.com");

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
    async fn modifier_moi_courriel_sans_mot_de_passe_actuel_est_refuse() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "courriel": "alice2@example.com" })),
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
                "courriel": "alice2@example.com",
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
                "courriel": "alice2@example.com",
                "mot_de_passe_actuel": "motdepasse123",
            })),
        )
        .await;
        assert_eq!(statut, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn modifier_moi_courriel_apres_suppression_est_non_autorise() {
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
                "courriel": "alice2@example.com",
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
    async fn modifier_moi_rejette_un_courriel_invalide() {
        let ctx = nouveau_contexte();
        let (_, jeton) = inscrire(&ctx.app, "alice", "alice@example.com", "motdepasse123").await;

        let (statut, corps) = requete(
            &ctx.app,
            "PATCH",
            "/api/auth/me",
            Some(&jeton),
            Some(json!({ "courriel": "pas-un-courriel" })),
        )
        .await;
        assert_eq!(statut, StatusCode::BAD_REQUEST);
        assert_eq!(corps["erreur"], "courriel: adresse courriel invalide");
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
            .changer_role(id_admin, Role::Utilisateur)
            .await
            .unwrap();

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
        assert_eq!(statut, StatusCode::OK);
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
            Some(json!({ "role": "moderateur" })),
        )
        .await;
        assert_eq!(statut, StatusCode::OK);
        assert_eq!(corps["role"], "moderateur");

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

        let (statut, _) = requete(
            &ctx.app,
            "DELETE",
            &format!("/api/users/{id_alice}"),
            Some(&jeton_admin),
            None,
        )
        .await;
        assert_eq!(statut, StatusCode::NOT_FOUND);
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
}
