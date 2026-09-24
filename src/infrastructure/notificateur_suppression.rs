//! Prévient les autres microservices qu'un compte a été supprimé
//! (`DELETE {url}/api/internal/users/{id}`, en-tête `x-internal-token`).
//! Sans ça, ses serveurs restaient sans propriétaire, ses messages et sa
//! configuration (alias, avatar) restaient en base pour toujours.
//!
//! « Tire et oublie » : chaque service est appelé dans une tâche de fond,
//! avec quelques nouvelles tentatives ; un échec définitif est journalisé
//! (les routes des services sont idempotentes : l'appel peut être rejoué à
//! la main).

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::domain::ports::NotificateurSuppressionPort;

const DELAI_APPEL: Duration = Duration::from_secs(5);
/// Attentes avant chaque nouvelle tentative (donc 3 essais au total).
const ATTENTES_ENTRE_TENTATIVES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

pub struct HttpNotificateurSuppression {
    client: reqwest::Client,
    urls: Arc<Vec<String>>,
    jeton_interne: Option<String>,
    attentes: &'static [Duration],
}

impl HttpNotificateurSuppression {
    /// `urls` : URL de base de chaque service à prévenir (sans `/` final
    /// obligatoire). Sans URL ou sans jeton interne, rien n'est envoyé (un
    /// avertissement est journalisé à chaque suppression).
    pub fn new(urls: Vec<String>, jeton_interne: Option<String>) -> Self {
        Self::avec_attentes(urls, jeton_interne, &ATTENTES_ENTRE_TENTATIVES)
    }

    fn avec_attentes(
        urls: Vec<String>,
        jeton_interne: Option<String>,
        attentes: &'static [Duration],
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(DELAI_APPEL)
            .build()
            .expect("impossible de créer le client HTTP");
        Self {
            client,
            urls: Arc::new(
                urls.into_iter()
                    .map(|url| url.trim_end_matches('/').to_string())
                    .collect(),
            ),
            jeton_interne,
            attentes,
        }
    }

    /// Appelle chaque service ; renvoie ceux qui ont définitivement échoué.
    async fn prevenir(
        client: reqwest::Client,
        urls: Arc<Vec<String>>,
        jeton: String,
        id: Uuid,
        attentes: &'static [Duration],
    ) -> Vec<String> {
        let mut echecs = Vec::new();
        for base in urls.iter() {
            let url = format!("{base}/api/internal/users/{id}");
            let mut reussi = false;
            for tentative in 0..=attentes.len() {
                if tentative > 0 {
                    tokio::time::sleep(attentes[tentative - 1]).await;
                }
                match client
                    .delete(&url)
                    .header("x-internal-token", &jeton)
                    .send()
                    .await
                {
                    Ok(reponse) if reponse.status().is_success() => {
                        reussi = true;
                        break;
                    }
                    Ok(reponse) => tracing::warn!(
                        %url, statut = %reponse.status(), tentative,
                        "compte supprimé : service refusant la notification"
                    ),
                    Err(erreur) => tracing::warn!(
                        %url, %erreur, tentative,
                        "compte supprimé : service injoignable"
                    ),
                }
            }
            if !reussi {
                tracing::error!(
                    %url,
                    "compte supprimé : service non prévenu, ses données restent (rejouer l'appel à la main)"
                );
                echecs.push(base.clone());
            }
        }
        echecs
    }
}

impl NotificateurSuppressionPort for HttpNotificateurSuppression {
    fn compte_supprime(&self, id: Uuid) {
        if self.urls.is_empty() {
            tracing::warn!(%id, "compte supprimé : SERVICES_COMPTE_SUPPRIME_URLS vide, aucun service prévenu");
            return;
        }
        let Some(jeton) = self.jeton_interne.clone() else {
            tracing::warn!(%id, "compte supprimé : INTERNAL_SERVICE_TOKEN absent, aucun service prévenu");
            return;
        };
        tokio::spawn(Self::prevenir(
            self.client.clone(),
            self.urls.clone(),
            jeton,
            id,
            self.attentes,
        ));
    }
}

/// Pour les tests des routes : retient les comptes signalés.
#[cfg(test)]
#[derive(Default)]
pub struct NotificateurEnregistreur {
    pub ids: std::sync::Mutex<Vec<Uuid>>,
}

#[cfg(test)]
impl NotificateurSuppressionPort for NotificateurEnregistreur {
    fn compte_supprime(&self, id: Uuid) {
        self.ids.lock().unwrap().push(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        routing::delete,
        Router,
    };
    use std::sync::Mutex;

    /// `(id de l'URL, en-tête x-internal-token)` de chaque appel reçu.
    type Appels = Arc<Mutex<Vec<(String, Option<String>)>>>;

    #[derive(Clone, Default)]
    struct Recu {
        appels: Appels,
        echecs_restants: Arc<Mutex<u32>>,
    }

    async fn route(
        State(recu): State<Recu>,
        Path(id): Path<String>,
        entetes: HeaderMap,
    ) -> StatusCode {
        let jeton = entetes
            .get("x-internal-token")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        recu.appels.lock().unwrap().push((id, jeton));
        let mut echecs = recu.echecs_restants.lock().unwrap();
        if *echecs > 0 {
            *echecs -= 1;
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        }
    }

    async fn service_factice(echecs: u32) -> (String, Recu) {
        let recu = Recu::default();
        *recu.echecs_restants.lock().unwrap() = echecs;
        let app = Router::new()
            .route("/api/internal/users/:id", delete(route))
            .with_state(recu.clone());
        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecoute.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(ecoute, app).await.unwrap() });
        (format!("http://{adresse}/"), recu)
    }

    const SANS_ATTENTE: [Duration; 2] = [Duration::ZERO, Duration::ZERO];

    #[tokio::test]
    async fn previent_chaque_service_avec_le_jeton_interne() {
        let (url_a, recu_a) = service_factice(0).await;
        let (url_b, recu_b) = service_factice(0).await;
        let id = Uuid::new_v4();

        let echecs = HttpNotificateurSuppression::prevenir(
            reqwest::Client::new(),
            Arc::new(vec![
                url_a.trim_end_matches('/').to_string(),
                url_b.trim_end_matches('/').to_string(),
            ]),
            "secret".into(),
            id,
            &SANS_ATTENTE,
        )
        .await;

        assert!(echecs.is_empty());
        for recu in [recu_a, recu_b] {
            assert_eq!(
                *recu.appels.lock().unwrap(),
                vec![(id.to_string(), Some("secret".to_string()))]
            );
        }
    }

    #[tokio::test]
    async fn reessaie_puis_abandonne_un_service_en_panne() {
        let (url_ok, recu_ok) = service_factice(1).await;
        let (url_ko, recu_ko) = service_factice(10).await;
        let url_ko = url_ko.trim_end_matches('/').to_string();

        let echecs = HttpNotificateurSuppression::prevenir(
            reqwest::Client::new(),
            Arc::new(vec![
                url_ok.trim_end_matches('/').to_string(),
                url_ko.clone(),
            ]),
            "secret".into(),
            Uuid::new_v4(),
            &SANS_ATTENTE,
        )
        .await;

        // Un échec puis une réussite ; trois échecs.
        assert_eq!(recu_ok.appels.lock().unwrap().len(), 2);
        assert_eq!(recu_ko.appels.lock().unwrap().len(), 3);
        assert_eq!(echecs, vec![url_ko]);
    }

    #[tokio::test]
    async fn un_service_injoignable_ne_bloque_pas_les_autres() {
        let (url_ok, recu_ok) = service_factice(0).await;
        let echecs = HttpNotificateurSuppression::prevenir(
            reqwest::Client::new(),
            Arc::new(vec![
                "http://127.0.0.1:1".to_string(),
                url_ok.trim_end_matches('/').to_string(),
            ]),
            "secret".into(),
            Uuid::new_v4(),
            &SANS_ATTENTE,
        )
        .await;
        assert_eq!(echecs, vec!["http://127.0.0.1:1".to_string()]);
        assert_eq!(recu_ok.appels.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn compte_supprime_envoie_en_tache_de_fond() {
        let (url, recu) = service_factice(0).await;
        let notificateur = HttpNotificateurSuppression::avec_attentes(
            vec![url],
            Some("secret".into()),
            &SANS_ATTENTE,
        );
        let id = Uuid::new_v4();
        notificateur.compte_supprime(id);
        for _ in 0..100 {
            if !recu.appels.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(recu.appels.lock().unwrap()[0].0, id.to_string());
    }

    #[tokio::test]
    async fn sans_url_ou_sans_jeton_rien_n_est_envoye() {
        let (url, recu) = service_factice(0).await;
        HttpNotificateurSuppression::new(vec![url], None).compte_supprime(Uuid::new_v4());
        HttpNotificateurSuppression::new(vec![], Some("secret".into()))
            .compte_supprime(Uuid::new_v4());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(recu.appels.lock().unwrap().is_empty());
    }
}
