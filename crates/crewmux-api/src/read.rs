//! Typed GET helpers over the blocking client.

use serde::de::DeserializeOwned;

use crate::client::Client;
use crate::endpoint::{self, Endpoint, Hydrate, ListSeats};
use crate::error::Error;
use crate::model::{AttachInfo, SeatList, Snapshot};

impl Client {
    /// `GET /api/seats`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the daemon is unreachable or the body is not a seat list.
    pub fn list_seats(&self) -> Result<SeatList, Error> {
        self.get_json(ListSeats::PATH)
    }

    /// `GET /api/snapshot`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the daemon is unreachable or the body is not a snapshot.
    pub fn hydrate(&self) -> Result<Snapshot, Error> {
        self.get_json(Hydrate::PATH)
    }

    /// `GET /api/seats/{seat}/attach`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the seat is unknown or the body is not attach info.
    pub fn attach_info(&self, seat: &str) -> Result<AttachInfo, Error> {
        self.get_json(&endpoint::attach_path(seat))
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let response = self.request(ListSeats::METHOD.as_str(), path, None)?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Status {
                path: path.to_owned(),
                status: response.status,
            });
        }
        serde_json::from_str(&response.body).map_err(|err| Error::Json {
            path: path.to_owned(),
            message: err.to_string(),
        })
    }
}
