use crate::{request_data::RequestData, traits::ApubObject, utils::fetch_object_http, Error};
use anyhow::anyhow;
use chrono::{Duration as ChronoDuration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display, Formatter},
    marker::PhantomData,
};
use url::Url;

/// Typed wrapper for Activitypub Object ID.
///
/// It provides convenient methods for fetching the object from remote server or local database.
/// Objects are automatically cached locally, so they don't have to be fetched every time. Much of
/// the crate functionality relies on this wrapper.
///
/// Every time an object is fetched via HTTP, [RequestData.request_counter] is incremented by one.
/// If the value exceeds [FederationSettings.http_fetch_limit], the request is aborted with
/// [Error::RequestLimit]. This prevents denial of service attacks where an attack triggers
/// infinite, recursive fetching of data.
///
/// ```
/// # use activitypub_federation::core::object_id::ObjectId;
/// # use activitypub_federation::config::FederationConfig;
/// # use activitypub_federation::Error::NotFound;
/// # use activitypub_federation::traits::tests::{DbConnection, DbUser};
/// # let _ = actix_rt::System::new();
/// # actix_rt::Runtime::new().unwrap().block_on(async {
/// # let db_connection = DbConnection;
/// let config = FederationConfig::builder()
///     .hostname("example.com")
///     .app_data(db_connection)
///     .build()?;
/// let request_data = config.to_request_data();
/// let object_id: ObjectId::<DbUser> = "https://lemmy.ml/u/nutomic".try_into()?;
/// // Attempt to fetch object from local database or fall back to remote server
/// let user = object_id.dereference(&request_data).await;
/// assert!(user.is_ok());
/// // Now you can also read the object from local database without network requests
/// let user = object_id.dereference_local(&request_data).await;
/// assert!(user.is_ok());
/// # Ok::<(), anyhow::Error>(())
/// # }).unwrap();
/// ```
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId<Kind>(Box<Url>, PhantomData<Kind>)
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>;

impl<Kind> ObjectId<Kind>
where
    Kind: ApubObject + Send + 'static,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    /// Construct a new objectid instance
    pub fn new<T>(url: T) -> Self
    where
        T: Into<Url>,
    {
        ObjectId(Box::new(url.into()), PhantomData::<Kind>)
    }

    pub fn inner(&self) -> &Url {
        &self.0
    }

    pub fn into_inner(self) -> Url {
        *self.0
    }

    /// Fetches an activitypub object, either from local database (if possible), or over http.
    pub async fn dereference(
        &self,
        data: &RequestData<<Kind as ApubObject>::DataType>,
    ) -> Result<Kind, <Kind as ApubObject>::Error>
    where
        <Kind as ApubObject>::Error: From<Error> + From<anyhow::Error>,
    {
        let db_object = self.dereference_from_db(data).await?;

        // if its a local object, only fetch it from the database and not over http
        if data.config.is_local_url(&self.0) {
            return match db_object {
                None => Err(Error::NotFound.into()),
                Some(o) => Ok(o),
            };
        }

        // object found in database
        if let Some(object) = db_object {
            // object is old and should be refetched
            if let Some(last_refreshed_at) = object.last_refreshed_at() {
                if should_refetch_object(last_refreshed_at) {
                    return self.dereference_from_http(data, Some(object)).await;
                }
            }
            Ok(object)
        }
        // object not found, need to fetch over http
        else {
            self.dereference_from_http(data, None).await
        }
    }

    /// Fetch an object from the local db. Instead of falling back to http, this throws an error if
    /// the object is not found in the database.
    pub async fn dereference_local(
        &self,
        data: &RequestData<<Kind as ApubObject>::DataType>,
    ) -> Result<Kind, <Kind as ApubObject>::Error>
    where
        <Kind as ApubObject>::Error: From<Error>,
    {
        let object = self.dereference_from_db(data).await?;
        object.ok_or_else(|| Error::NotFound.into())
    }

    /// returning none means the object was not found in local db
    async fn dereference_from_db(
        &self,
        data: &RequestData<<Kind as ApubObject>::DataType>,
    ) -> Result<Option<Kind>, <Kind as ApubObject>::Error> {
        let id = self.0.clone();
        ApubObject::read_from_apub_id(*id, data).await
    }

    async fn dereference_from_http(
        &self,
        data: &RequestData<<Kind as ApubObject>::DataType>,
        db_object: Option<Kind>,
    ) -> Result<Kind, <Kind as ApubObject>::Error>
    where
        <Kind as ApubObject>::Error: From<Error> + From<anyhow::Error>,
    {
        let res = fetch_object_http(&self.0, data).await;

        if let Err(Error::ObjectDeleted) = &res {
            if let Some(db_object) = db_object {
                db_object.delete(data).await?;
            }
            return Err(anyhow!("Fetched remote object {} which was deleted", self).into());
        }

        let res2 = res?;

        Kind::from_apub(res2, data).await
    }
}

/// Need to implement clone manually, to avoid requiring Kind to be Clone
impl<Kind> Clone for ObjectId<Kind>
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn clone(&self) -> Self {
        ObjectId(self.0.clone(), self.1)
    }
}

static ACTOR_REFETCH_INTERVAL_SECONDS: i64 = 24 * 60 * 60;
static ACTOR_REFETCH_INTERVAL_SECONDS_DEBUG: i64 = 20;

/// Determines when a remote actor should be refetched from its instance. In release builds, this is
/// `ACTOR_REFETCH_INTERVAL_SECONDS` after the last refetch, in debug builds
/// `ACTOR_REFETCH_INTERVAL_SECONDS_DEBUG`.
fn should_refetch_object(last_refreshed: NaiveDateTime) -> bool {
    let update_interval = if cfg!(debug_assertions) {
        // avoid infinite loop when fetching community outbox
        ChronoDuration::seconds(ACTOR_REFETCH_INTERVAL_SECONDS_DEBUG)
    } else {
        ChronoDuration::seconds(ACTOR_REFETCH_INTERVAL_SECONDS)
    };
    let refresh_limit = Utc::now().naive_utc() - update_interval;
    last_refreshed.lt(&refresh_limit)
}

impl<Kind> Display for ObjectId<Kind>
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl<Kind> Debug for ObjectId<Kind>
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl<Kind> From<ObjectId<Kind>> for Url
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn from(id: ObjectId<Kind>) -> Self {
        *id.0
    }
}

impl<Kind> From<Url> for ObjectId<Kind>
where
    Kind: ApubObject + Send + 'static,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn from(url: Url) -> Self {
        ObjectId::new(url)
    }
}

impl<'a, Kind> TryFrom<&'a str> for ObjectId<Kind>
where
    Kind: ApubObject + Send + 'static,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    type Error = url::ParseError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        Ok(ObjectId::new(Url::parse(value)?))
    }
}

impl<Kind> PartialEq for ObjectId<Kind>
where
    Kind: ApubObject,
    for<'de2> <Kind as ApubObject>::ApubType: serde::Deserialize<'de2>,
{
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0) && self.1 == other.1
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{core::object_id::should_refetch_object, traits::tests::DbUser};

    #[test]
    fn test_deserialize() {
        let url = Url::parse("http://test.com/").unwrap();
        let id = ObjectId::<DbUser>::new(url);

        let string = serde_json::to_string(&id).unwrap();
        assert_eq!("\"http://test.com/\"", string);

        let parsed: ObjectId<DbUser> = serde_json::from_str(&string).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_should_refetch_object() {
        let one_second_ago = Utc::now().naive_utc() - ChronoDuration::seconds(1);
        assert_eq!(false, should_refetch_object(one_second_ago));

        let two_days_ago = Utc::now().naive_utc() - ChronoDuration::days(2);
        assert_eq!(true, should_refetch_object(two_days_ago));
    }
}
