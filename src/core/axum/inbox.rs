use crate::{
    core::{
        axum::ActivityData,
        object_id::ObjectId,
        signatures::{verify_inbox_hash, verify_signature},
    },
    request_data::RequestData,
    traits::{ActivityHandler, Actor, ApubObject},
    Error,
};
use serde::de::DeserializeOwned;
use tracing::debug;

/// Receive an activity and perform some basic checks, including HTTP signature verification.
pub async fn receive_activity<Activity, ActorT, Datatype>(
    activity_data: ActivityData,
    data: &RequestData<Datatype>,
) -> Result<(), <Activity as ActivityHandler>::Error>
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
    verify_inbox_hash(activity_data.headers.get("Digest"), &activity_data.body)?;

    let activity: Activity = serde_json::from_slice(&activity_data.body)?;
    data.config.verify_url_and_domain(&activity).await?;
    let actor = ObjectId::<ActorT>::new(activity.actor().clone())
        .dereference(data)
        .await?;

    verify_signature(
        &activity_data.headers,
        &activity_data.method,
        &activity_data.uri,
        actor.public_key(),
    )?;

    debug!("Receiving activity {}", activity.id().to_string());
    activity.receive(data).await?;
    Ok(())
}

// TODO: copy tests from actix-web inbox and implement for axum as well
