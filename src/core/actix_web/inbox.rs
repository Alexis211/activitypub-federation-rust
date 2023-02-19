use crate::{
    core::{
        object_id::ObjectId,
        signatures::{verify_inbox_hash, verify_signature},
    },
    request_data::RequestData,
    traits::{ActivityHandler, Actor, ApubObject},
    Error,
};
use actix_web::{web::Bytes, HttpRequest, HttpResponse};
use serde::de::DeserializeOwned;
use tracing::debug;

/// Receive an activity and perform some basic checks, including HTTP signature verification.
pub async fn receive_activity<Activity, ActorT, Datatype>(
    request: HttpRequest,
    body: Bytes,
    data: &RequestData<Datatype>,
) -> Result<HttpResponse, <Activity as ActivityHandler>::Error>
where
    Activity: ActivityHandler<DataType = Datatype> + DeserializeOwned + Send + 'static,
    ActorT: ApubObject<DataType = Datatype> + Actor + Send + 'static,
    for<'de2> <ActorT as ApubObject>::ApubType: serde::Deserialize<'de2>,
    <Activity as ActivityHandler>::Error: From<anyhow::Error>
        + From<Error>
        + From<<ActorT as ApubObject>::Error>
        + From<serde_json::Error>,
    <ActorT as ApubObject>::Error: From<Error> + From<anyhow::Error>,
    Datatype: Clone,
{
    verify_inbox_hash(request.headers().get("Digest"), &body)?;

    let activity: Activity = serde_json::from_slice(&body)?;
    data.config.verify_url_and_domain(&activity).await?;
    let actor = ObjectId::<ActorT>::new(activity.actor().clone())
        .dereference(data)
        .await?;

    verify_signature(
        request.headers(),
        request.method(),
        request.uri(),
        actor.public_key(),
    )?;

    debug!("Receiving activity {}", activity.id().to_string());
    activity.receive(data).await?;
    Ok(HttpResponse::Ok().finish())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        config::FederationConfig,
        core::signatures::{sign_request, PublicKey},
        traits::tests::{DbConnection, DbUser, Follow, DB_USER_KEYPAIR},
    };
    use actix_web::test::TestRequest;
    use reqwest::Client;
    use reqwest_middleware::ClientWithMiddleware;
    use url::Url;

    #[actix_rt::test]
    async fn test_receive_activity() {
        let (body, incoming_request, config) = setup_receive_test().await;
        receive_activity::<Follow, DbUser, DbConnection>(
            incoming_request.to_http_request(),
            body.into(),
            &config.to_request_data(),
        )
        .await
        .unwrap();
    }

    #[actix_rt::test]
    async fn test_receive_activity_invalid_body_signature() {
        let (_, incoming_request, config) = setup_receive_test().await;
        let err = receive_activity::<Follow, DbUser, DbConnection>(
            incoming_request.to_http_request(),
            "invalid".into(),
            &config.to_request_data(),
        )
        .await
        .err()
        .unwrap();

        let e = err.root_cause().downcast_ref::<Error>().unwrap();
        assert_eq!(e, &Error::ActivityBodyDigestInvalid)
    }

    #[actix_rt::test]
    async fn test_receive_activity_invalid_path() {
        let (body, incoming_request, config) = setup_receive_test().await;
        let incoming_request = incoming_request.uri("/wrong");
        let err = receive_activity::<Follow, DbUser, DbConnection>(
            incoming_request.to_http_request(),
            body.into(),
            &config.to_request_data(),
        )
        .await
        .err()
        .unwrap();

        let e = err.root_cause().downcast_ref::<Error>().unwrap();
        assert_eq!(e, &Error::ActivitySignatureInvalid)
    }

    async fn setup_receive_test() -> (String, TestRequest, FederationConfig<DbConnection>) {
        let request_builder =
            ClientWithMiddleware::from(Client::default()).post("https://example.com/inbox");
        let public_key = PublicKey::new_main_key(
            Url::parse("https://example.com").unwrap(),
            DB_USER_KEYPAIR.public_key.clone(),
        );
        let activity = Follow {
            actor: "http://localhost:123".try_into().unwrap(),
            object: "http://localhost:124".try_into().unwrap(),
            kind: Default::default(),
            id: "http://localhost:123/1".try_into().unwrap(),
        };
        let body = serde_json::to_string(&activity).unwrap();
        let outgoing_request = sign_request(
            request_builder,
            body.to_string(),
            public_key,
            DB_USER_KEYPAIR.private_key.clone(),
            false,
        )
        .await
        .unwrap();
        let mut incoming_request = TestRequest::post().uri(outgoing_request.url().path());
        for h in outgoing_request.headers() {
            incoming_request = incoming_request.append_header(h);
        }

        let config = FederationConfig::builder()
            .hostname("localhost:8002")
            .app_data(DbConnection)
            .debug(true)
            .build()
            .unwrap();
        (body, incoming_request, config)
    }
}
