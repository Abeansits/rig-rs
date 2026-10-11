//! Chat types and seat-token sender resolution.
//!
//! Slice 4's `rig ask` reads history through [`ChatRepo`]. The daemon resolves
//! the sender before it calls [`ChatRepo::post`].

use std::str::FromStr;

use crewmux_types::{SeatId, SenderRef};

pub use crewmux_core::chat::{
    ChatError, ChatMessage, ChatRepo, HistoryPage, HistoryQuery, NewChat, NewTopic,
};

/// A seat id plus the token the caller presented and the token the store has.
pub struct SeatProof<'a> {
    /// `X-Rig-Seat-Id`.
    pub seat_id: &'a str,
    /// `X-Rig-Seat-Token`.
    pub token: &'a str,
    /// Token stored at launch, when this seat has one.
    pub stored_token: Option<&'a str>,
}

/// Decide who a chat write is from.
///
/// No proof and no claim, or a claim of `operator`, is [`SenderRef::operator`].
/// A claim of anyone else without a matching seat token is [`ChatError::UnknownSender`].
/// A matching token attributes the row to that seat. A body claim that names
/// someone else is rejected.
///
/// # Errors
///
/// Returns [`ChatError::UnknownSender`] when the proof or the claim does not authenticate.
pub fn resolve_sender(
    proof: Option<SeatProof<'_>>,
    claimed: Option<&str>,
) -> Result<SenderRef, ChatError> {
    let claimed = claimed.filter(|text| !text.is_empty());
    match proof {
        Some(proof) => authenticated_seat(&proof, claimed),
        None => unauthenticated(claimed),
    }
}

fn authenticated_seat(
    proof: &SeatProof<'_>,
    claimed: Option<&str>,
) -> Result<SenderRef, ChatError> {
    let stored = proof.stored_token.ok_or(ChatError::UnknownSender)?;
    if stored != proof.token || proof.token.is_empty() {
        return Err(ChatError::UnknownSender);
    }
    let seat = SeatId::from_str(proof.seat_id).map_err(|_| ChatError::UnknownSender)?;
    let sender = SenderRef::seat(seat);
    if let Some(claimed) = claimed
        && claimed != sender.as_str()
    {
        return Err(ChatError::UnknownSender);
    }
    Ok(sender)
}

fn unauthenticated(claimed: Option<&str>) -> Result<SenderRef, ChatError> {
    match claimed {
        None => Ok(SenderRef::operator()),
        Some(text) if text == SenderRef::OPERATOR => Ok(SenderRef::operator()),
        Some(_) => Err(ChatError::UnknownSender),
    }
}

#[cfg(test)]
mod tests {
    use super::{SeatProof, resolve_sender};
    use crewmux_core::chat::ChatError;
    use crewmux_types::{SeatId, SenderRef};

    #[test]
    fn outside_a_seat_is_operator() {
        assert_eq!(
            resolve_sender(None, None).expect("operator"),
            SenderRef::operator()
        );
        assert_eq!(
            resolve_sender(None, Some(SenderRef::OPERATOR)).expect("operator"),
            SenderRef::operator()
        );
    }

    #[test]
    fn spoofed_claim_without_a_token_is_rejected() {
        let err = resolve_sender(None, Some("alice")).expect_err("spoof");
        assert_eq!(err, ChatError::UnknownSender);
    }

    #[test]
    fn matching_token_is_the_seat_and_a_mismatch_is_rejected() {
        let seat = SeatId::new();
        let seat_text = seat.to_string();
        let proof = SeatProof {
            seat_id: &seat_text,
            token: "token-a",
            stored_token: Some("token-a"),
        };
        assert_eq!(
            resolve_sender(Some(proof), None).expect("seat"),
            SenderRef::seat(seat)
        );
        let wrong = SeatProof {
            seat_id: &seat_text,
            token: "nope",
            stored_token: Some("token-a"),
        };
        assert_eq!(
            resolve_sender(Some(wrong), None).expect_err("token"),
            ChatError::UnknownSender
        );
        let mismatched_claim = SeatProof {
            seat_id: &seat_text,
            token: "token-a",
            stored_token: Some("token-a"),
        };
        assert_eq!(
            resolve_sender(Some(mismatched_claim), Some("mallory")).expect_err("claim"),
            ChatError::UnknownSender
        );
    }
}
