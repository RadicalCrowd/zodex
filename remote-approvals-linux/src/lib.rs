//! Fail-closed native protocol primitives for Remote Approvals v1.
//!
//! This crate is the core cryptographic contract engine and typed validator.
//! It intentionally contains no networking, keyring, WebAuthn assertion verification,
//! or process execution logic. Higher-level broker implementations must fulfill
//! the following seven core responsibilities:
//!
//! 1. **WebAuthn Verification**: Validate WebAuthn `clientDataJSON.challenge` (matching `base64url(SHA-256(response_digest))`),
//!    RP ID/origin, User Verification (UV) flag, and credential-to-device binding before accepting an upstream response.
//! 2. **Keyring Storage**: Secure private signing and decryption keys in the OS keyring on desktop or non-extractable WebCrypto storage in PWAs.
//! 3. **TLS/WSS Transport**: Manage outbound TLS/WSS network connections framing without listening on public inbound ports.
//! 4. **Relay Frame & Rate Limits**: Enforce 64 KiB maximum ciphertext size, 2 KiB metadata size, max 32 pending requests per active device
//!    (1 per action/thread), 60 frames/min rate limit per connection, and 5 minute max ciphertext retention.
//! 5. **Atomic Persistence**: Track single-use lifecycle state transitions ([`RequestLifecycle`]) and persist state atomically to prevent replay or race conditions.
//! 6. **Fail-Closed Upstream Resolution & Ciphertext Deletion**: Immediately resolve upstream requests as failed and delete queued relay ciphertext on error, expiry, cancellation, or revocation.
//! 7. **Metadata-Only Audit Logging**: Log only safe metadata (event names, request/device IDs, epochs, timestamps, outcome/error codes, request kinds, profile IDs, truncated key IDs) and never raw action text, file paths, command arguments, ciphertexts, URLs, WebAuthn blobs, passwords, or full hashes.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hpke::{
    aead::AesGcm256, kdf::HkdfSha256, kem::DhP256HkdfSha256, setup_receiver, setup_sender,
    Deserializable, OpModeR, OpModeS, Serializable,
};
use p256::{
    ecdsa::{
        signature::hazmat::{PrehashSigner, PrehashVerifier},
        Signature, SigningKey, VerifyingKey,
    },
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey},
};
use rand_core::{OsRng, UnwrapErr};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fmt;
use thiserror::Error;
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};
use uuid::Uuid;

pub use zeroize::Zeroizing;

/// The only accepted protocol version.
pub const PROTOCOL_VERSION: &str = "zodex.remote-approval.v1";
/// The host's absolute maximum pending duration.
pub const MAX_TTL: Duration = Duration::minutes(5);
/// A receiver may display a request with at most this much local clock skew.
pub const MAX_RECEIVER_CLOCK_SKEW: Duration = Duration::seconds(30);
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const HPKE_INFO: &[u8] = b"zodex.remote-approval.v1";

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolError {
    #[error("unsupported protocol")]
    UnsupportedProtocol,
    #[error("unsupported request kind")]
    UnsupportedRequestKind,
    #[error("invalid schema")]
    InvalidSchema,
    #[error("invalid canonicalization")]
    InvalidCanonicalization,
    #[error("invalid action digest")]
    InvalidActionDigest,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid WebAuthn assertion")]
    InvalidWebauthn,
    #[error("request expired")]
    RequestExpired,
    #[error("request cancelled")]
    RequestCancelled,
    #[error("response replayed")]
    ResponseReplayed,
    #[error("inactive device")]
    InactiveDevice,
    #[error("stale device epoch")]
    StaleDeviceEpoch,
    #[error("device revoked")]
    DeviceRevoked,
    #[error("relay limit exceeded")]
    RelayLimitExceeded,
    #[error("relay unavailable")]
    RelayUnavailable,
    #[error("transport ambiguous")]
    TransportAmbiguous,
    #[error("resolved elsewhere")]
    ResolvedElsewhere,
    #[error("key rotation required")]
    KeyRotationRequired,
    #[error("local presence required")]
    LocalPresenceRequired,
}

impl ProtocolError {
    pub const fn wire_code(self) -> &'static str {
        match self {
            Self::UnsupportedProtocol => "unsupported_protocol",
            Self::UnsupportedRequestKind => "unsupported_request_kind",
            Self::InvalidSchema => "invalid_schema",
            Self::InvalidCanonicalization => "invalid_canonicalization",
            Self::InvalidActionDigest => "invalid_action_digest",
            Self::InvalidSignature => "invalid_signature",
            Self::InvalidWebauthn => "invalid_webauthn",
            Self::RequestExpired => "request_expired",
            Self::RequestCancelled => "request_cancelled",
            Self::ResponseReplayed => "response_replayed",
            Self::InactiveDevice => "inactive_device",
            Self::StaleDeviceEpoch => "stale_device_epoch",
            Self::DeviceRevoked => "device_revoked",
            Self::RelayLimitExceeded => "relay_limit_exceeded",
            Self::RelayUnavailable => "relay_unavailable",
            Self::TransportAmbiguous => "transport_ambiguous",
            Self::ResolvedElsewhere => "resolved_elsewhere",
            Self::KeyRotationRequired => "key_rotation_required",
            Self::LocalPresenceRequired => "local_presence_required",
        }
    }
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostIdentity {
    pub host_id: String,
    pub signing_key_id: String,
    pub encryption_key_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub signing_key_id: String,
    pub credential_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviceState {
    Pairing,
    PairedInactive,
    Active,
    Revoked,
    Rotated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRecord {
    pub protocol_version: String,
    pub device_id: String,
    pub state: DeviceState,
    pub active_device_epoch: u64,
    pub signing_key_id: String,
    pub encryption_key_id: String,
    pub webauthn_credential_id: String,
    pub rp_id: String,
    pub origin: String,
    pub paired_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Command {
        title: String,
        summary: String,
        executable: String,
        argv: Vec<String>,
        cwd: String,
    },
    FileChange {
        title: String,
        summary: String,
        operation: FileOperation,
        paths: Vec<String>,
    },
    Permission {
        title: String,
        summary: String,
        permission: String,
        scope: String,
    },
    UserInput {
        title: String,
        summary: String,
        input: ClosedInput,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Create,
    Modify,
    Delete,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClosedInput {
    Boolean,
    Enum { choices: Vec<Choice> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequest {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub nonce: String,
    pub issued_at: String,
    pub expires_at: String,
    pub active_device_epoch: u64,
    pub host: HostIdentity,
    pub action: Action,
    pub action_digest: String,
    pub host_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Decision {
    Approve,
    Reject,
    Boolean { value: bool },
    Enum { choice_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebauthnAssertion {
    pub client_data_json: String,
    pub authenticator_data: String,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalResponse {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub action_digest: String,
    pub active_device_epoch: u64,
    pub device: DeviceIdentity,
    pub decision: Decision,
    pub webauthn: WebauthnAssertion,
    pub device_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayFrame {
    pub protocol_version: String,
    pub operation: RelayOperation,
    pub request_id: Uuid,
    pub sender: String,
    pub recipient: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ciphertext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayOperation {
    Connect,
    Publish,
    Subscribe,
    Ack,
    PushWakeup,
    Health,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HpkeDirection {
    HostToDevice,
    DeviceToHost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HpkeContext {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub recipient_key_id: String,
    pub active_device_epoch: u64,
    pub direction: HpkeDirection,
}

impl HpkeContext {
    pub fn new(
        request_id: Uuid,
        recipient_key_id: String,
        active_device_epoch: u64,
        direction: HpkeDirection,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            request_id,
            recipient_key_id,
            active_device_epoch,
            direction,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_identifier(&self.recipient_key_id, 1, 128, false)?;
        validate_epoch(self.active_device_epoch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseEnvelope {
    pub protocol_version: String,
    pub request_id: Uuid,
    pub recipient_key_id: String,
    pub active_device_epoch: u64,
    pub direction: HpkeDirection,
    pub enc: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    PendingApproval,
    Delivered,
    AwaitingResponse,
    Accepted,
    Rejected,
    Cancelled,
    Expired,
    ResolvedElsewhere,
    Failed,
}

impl RequestState {
    pub const fn pending(self) -> bool {
        matches!(
            self,
            Self::PendingApproval | Self::Delivered | Self::AwaitingResponse
        )
    }
}

#[derive(Debug, Clone)]
pub struct RequestLifecycle {
    request_id: Uuid,
    expires_at: OffsetDateTime,
    active_device_id: String,
    active_device_epoch: u64,
    state: RequestState,
}

impl RequestLifecycle {
    pub fn new(request: &ApprovalRequest, active_device_id: impl Into<String>) -> Result<Self> {
        request.validate()?;
        Ok(Self {
            request_id: request.request_id,
            expires_at: parse_time(&request.expires_at)?,
            active_device_id: active_device_id.into(),
            active_device_epoch: request.active_device_epoch,
            state: RequestState::PendingApproval,
        })
    }

    pub const fn state(&self) -> RequestState {
        self.state
    }

    pub fn mark_delivered(&mut self, now: OffsetDateTime) -> Result<()> {
        self.advance(RequestState::Delivered, now)
    }
    pub fn mark_awaiting_response(&mut self, now: OffsetDateTime) -> Result<()> {
        self.advance(RequestState::AwaitingResponse, now)
    }
    pub fn cancel(&mut self) {
        if self.state.pending() {
            self.state = RequestState::Cancelled;
        }
    }
    pub fn resolve_elsewhere(&mut self) {
        if self.state.pending() {
            self.state = RequestState::ResolvedElsewhere;
        }
    }
    pub fn expire(&mut self, now: OffsetDateTime) -> Result<()> {
        if now >= self.expires_at {
            self.state = RequestState::Expired;
            return Err(ProtocolError::RequestExpired);
        }
        Ok(())
    }

    fn advance(&mut self, target: RequestState, now: OffsetDateTime) -> Result<()> {
        self.expire(now)?;
        if !self.state.pending() {
            return Err(terminal_error(self.state));
        }
        self.state = target;
        Ok(())
    }

    /// Atomically consumes the request before returning success. Callers must
    /// have separately verified the WebAuthn assertion before this method.
    pub fn accept_response(
        &mut self,
        request: &ApprovalRequest,
        response: &ApprovalResponse,
        active_device: &PairingRecord,
        device_key: &VerifyingKey,
        now: OffsetDateTime,
    ) -> Result<()> {
        self.expire(now)?;
        if !self.state.pending() {
            return Err(terminal_error(self.state));
        }
        validate_response(request, response, active_device, device_key)?;
        if response.request_id != self.request_id {
            return Err(ProtocolError::InvalidSchema);
        }
        if response.device.device_id != self.active_device_id {
            return Err(ProtocolError::InactiveDevice);
        }
        if response.active_device_epoch != self.active_device_epoch {
            return Err(ProtocolError::StaleDeviceEpoch);
        }
        self.state = match response.decision {
            Decision::Reject => RequestState::Rejected,
            _ => RequestState::Accepted,
        };
        Ok(())
    }
}

fn terminal_error(state: RequestState) -> ProtocolError {
    match state {
        RequestState::Cancelled => ProtocolError::RequestCancelled,
        RequestState::Expired => ProtocolError::RequestExpired,
        RequestState::ResolvedElsewhere => ProtocolError::ResolvedElsewhere,
        RequestState::Accepted | RequestState::Rejected => ProtocolError::ResponseReplayed,
        RequestState::Failed => ProtocolError::InvalidSchema,
        _ => ProtocolError::InvalidSchema,
    }
}

impl ApprovalRequest {
    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_identifier(&self.nonce, 22, 86, true)?;
        validate_epoch(self.active_device_epoch)?;
        validate_host(&self.host)?;
        let issued = parse_time(&self.issued_at)?;
        let expires = parse_time(&self.expires_at)?;
        if expires <= issued || expires - issued > MAX_TTL {
            return Err(ProtocolError::InvalidSchema);
        }
        self.action.validate()?;
        if self.action_digest != action_digest(&self.action)? {
            return Err(ProtocolError::InvalidActionDigest);
        }
        validate_signature_text(&self.host_signature)?;
        Ok(())
    }

    pub fn request_digest(&self) -> Result<[u8; 32]> {
        digest_omitting(self, "host_signature")
    }

    pub fn sign(&mut self, key: &SigningKey) -> Result<()> {
        self.host_signature.clear();
        self.host_signature = sign_digest(key, &self.request_digest()?)?;
        Ok(())
    }

    pub fn verify_signature(&self, key: &VerifyingKey) -> Result<()> {
        self.validate()?;
        verify_digest(key, &self.request_digest()?, &self.host_signature)
    }

    /// Rejects non-JCS wire input before it is used for a digest/signature.
    pub fn parse_transmitted(bytes: &[u8]) -> Result<Self> {
        require_jcs(bytes)?;
        let request: Self =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidSchema)?;
        request.validate()?;
        Ok(request)
    }
}

impl Action {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Command {
                title,
                summary,
                executable,
                argv,
                cwd,
            } => {
                validate_safe_text(title)?;
                validate_safe_text(summary)?;
                validate_path(executable)?;
                validate_path(cwd)?;
                if argv.len() > 128 {
                    return Err(ProtocolError::InvalidSchema);
                }
                argv.iter().try_for_each(|arg| validate_safe_text(arg))
            }
            Self::FileChange {
                title,
                summary,
                paths,
                ..
            } => {
                validate_safe_text(title)?;
                validate_safe_text(summary)?;
                if paths.is_empty() || paths.len() > 128 {
                    return Err(ProtocolError::InvalidSchema);
                }
                paths.iter().try_for_each(|path| validate_path(path))
            }
            Self::Permission {
                title,
                summary,
                permission,
                scope,
            } => {
                validate_safe_text(title)?;
                validate_safe_text(summary)?;
                validate_safe_text(permission)?;
                validate_safe_text(scope)
            }
            Self::UserInput {
                title,
                summary,
                input,
            } => {
                validate_safe_text(title)?;
                validate_safe_text(summary)?;
                match input {
                    ClosedInput::Boolean => Ok(()),
                    ClosedInput::Enum { choices } => {
                        if choices.is_empty() || choices.len() > 32 {
                            return Err(ProtocolError::InvalidSchema);
                        }
                        let mut ids = std::collections::BTreeSet::new();
                        for choice in choices {
                            validate_identifier(&choice.id, 1, 128, false)?;
                            validate_safe_text(&choice.label)?;
                            if !ids.insert(&choice.id) {
                                return Err(ProtocolError::InvalidSchema);
                            }
                        }
                        Ok(())
                    }
                }
            }
        }
    }
}

impl ApprovalResponse {
    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_epoch(self.active_device_epoch)?;
        validate_device(&self.device)?;
        validate_digest_text(&self.action_digest)?;
        validate_webauthn(&self.webauthn)?;
        validate_signature_text(&self.device_signature)
    }

    pub fn response_digest(&self) -> Result<[u8; 32]> {
        digest_omitting(self, "device_signature")
    }

    pub fn sign(&mut self, key: &SigningKey) -> Result<()> {
        self.device_signature.clear();
        self.device_signature = sign_digest(key, &self.response_digest()?)?;
        Ok(())
    }

    pub fn verify_signature(&self, key: &VerifyingKey) -> Result<()> {
        self.validate()?;
        verify_digest(key, &self.response_digest()?, &self.device_signature)
    }

    pub fn parse_transmitted(bytes: &[u8]) -> Result<Self> {
        require_jcs(bytes)?;
        let response: Self =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidSchema)?;
        response.validate()?;
        Ok(response)
    }
}

impl PairingRecord {
    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_identifier(&self.device_id, 1, 128, false)?;
        validate_epoch(self.active_device_epoch)?;
        validate_identifier(&self.signing_key_id, 1, 128, false)?;
        validate_identifier(&self.encryption_key_id, 1, 128, false)?;
        validate_identifier(&self.webauthn_credential_id, 1, 2048, true)?;
        validate_rp_id(&self.rp_id)?;
        if !self.origin.starts_with("https://") {
            return Err(ProtocolError::InvalidSchema);
        }
        let origin = self
            .origin
            .strip_prefix("https://")
            .ok_or(ProtocolError::InvalidSchema)?;
        if origin.contains('/') || origin != self.rp_id {
            return Err(ProtocolError::InvalidSchema);
        }
        parse_time(&self.paired_at)?;
        if let Some(revoked) = &self.revoked_at {
            parse_time(revoked)?;
        }
        Ok(())
    }
}

impl RelayFrame {
    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_identifier(&self.sender, 1, 128, false)?;
        validate_identifier(&self.recipient, 1, 128, false)?;
        parse_time(&self.created_at)?;
        match self.operation {
            RelayOperation::Publish => {
                if self.ack_id.is_some() {
                    return Err(ProtocolError::InvalidSchema);
                }
                let ciphertext = self
                    .ciphertext
                    .as_deref()
                    .ok_or(ProtocolError::InvalidSchema)?;
                validate_b64(ciphertext, 1, 87_382)?;
            }
            RelayOperation::Ack => {
                if self.ciphertext.is_some() {
                    return Err(ProtocolError::InvalidSchema);
                }
                validate_identifier(
                    self.ack_id.as_deref().ok_or(ProtocolError::InvalidSchema)?,
                    22,
                    86,
                    true,
                )?;
            }
            _ => {
                if self.ciphertext.is_some() || self.ack_id.is_some() {
                    return Err(ProtocolError::InvalidSchema);
                }
            }
        }
        Ok(())
    }
}

impl ResponseEnvelope {
    pub fn validate(&self) -> Result<()> {
        validate_version(&self.protocol_version)?;
        validate_identifier(&self.recipient_key_id, 1, 128, false)?;
        validate_epoch(self.active_device_epoch)?;
        // RFC 9180 P-256 enc is a 65-byte uncompressed SEC1 point = 87 base64url chars.
        let enc = decode_b64(&self.enc, ProtocolError::InvalidSchema)?;
        if enc.len() != 65 {
            return Err(ProtocolError::InvalidSchema);
        }
        validate_b64(&self.ciphertext, 1, 87_382)?;
        Ok(())
    }

    pub fn hpke_context(&self) -> HpkeContext {
        HpkeContext {
            protocol_version: self.protocol_version.clone(),
            request_id: self.request_id,
            recipient_key_id: self.recipient_key_id.clone(),
            active_device_epoch: self.active_device_epoch,
            direction: self.direction,
        }
    }
}

/// Validates response binding and its P-256 signature.  WebAuthn validation is
/// intentionally a separate component because it has browser/RP dependencies.
pub fn validate_response(
    request: &ApprovalRequest,
    response: &ApprovalResponse,
    active_device: &PairingRecord,
    device_key: &VerifyingKey,
) -> Result<()> {
    request.validate()?;
    response.validate()?;
    active_device.validate()?;
    if active_device.state == DeviceState::Revoked {
        return Err(ProtocolError::DeviceRevoked);
    }
    if active_device.state != DeviceState::Active {
        return Err(ProtocolError::InactiveDevice);
    }
    if response.request_id != request.request_id || response.action_digest != request.action_digest
    {
        return Err(ProtocolError::InvalidActionDigest);
    }
    if response.active_device_epoch != request.active_device_epoch
        || response.active_device_epoch != active_device.active_device_epoch
    {
        return Err(ProtocolError::StaleDeviceEpoch);
    }
    if response.device.device_id != active_device.device_id
        || response.device.signing_key_id != active_device.signing_key_id
        || response.device.credential_id != active_device.webauthn_credential_id
    {
        return Err(ProtocolError::InactiveDevice);
    }
    validate_decision(&request.action, &response.decision)?;
    response.verify_signature(device_key)
}

fn validate_decision(action: &Action, decision: &Decision) -> Result<()> {
    match action {
        Action::Command { .. } | Action::FileChange { .. } | Action::Permission { .. } => {
            if matches!(decision, Decision::Approve | Decision::Reject) {
                Ok(())
            } else {
                Err(ProtocolError::InvalidSchema)
            }
        }
        Action::UserInput {
            input: ClosedInput::Boolean,
            ..
        } => {
            if matches!(decision, Decision::Boolean { .. }) {
                Ok(())
            } else {
                Err(ProtocolError::InvalidSchema)
            }
        }
        Action::UserInput {
            input: ClosedInput::Enum { choices },
            ..
        } => match decision {
            Decision::Enum { choice_id }
                if choices.iter().any(|choice| choice.id == *choice_id) =>
            {
                Ok(())
            }
            _ => Err(ProtocolError::InvalidSchema),
        },
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).map_err(|_| ProtocolError::InvalidCanonicalization)
}

pub fn action_digest(action: &Action) -> Result<String> {
    Ok(hex_digest(&Sha256::digest(canonical_json(action)?)))
}

pub fn digest_omitting<T: Serialize>(value: &T, omitted_field: &str) -> Result<[u8; 32]> {
    let mut json =
        serde_json::to_value(value).map_err(|_| ProtocolError::InvalidCanonicalization)?;
    let object = json
        .as_object_mut()
        .ok_or(ProtocolError::InvalidCanonicalization)?;
    object
        .remove(omitted_field)
        .ok_or(ProtocolError::InvalidCanonicalization)?;
    Ok(Sha256::digest(canonical_json(&json)?).into())
}

pub fn sign_digest(key: &SigningKey, digest: &[u8; 32]) -> Result<String> {
    let signature: Signature = key
        .sign_prehash(digest)
        .map_err(|_| ProtocolError::InvalidSignature)?;
    Ok(URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

pub fn verify_digest(key: &VerifyingKey, digest: &[u8; 32], signature: &str) -> Result<()> {
    let bytes = decode_b64(signature, ProtocolError::InvalidSignature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| ProtocolError::InvalidSignature)?;
    key.verify_prehash(digest, &signature)
        .map_err(|_| ProtocolError::InvalidSignature)
}

pub fn signing_key_from_pkcs8_der(der: &[u8]) -> Result<SigningKey> {
    SigningKey::from_pkcs8_der(der).map_err(|_| ProtocolError::InvalidSchema)
}
pub fn verifying_key_from_spki_der(der: &[u8]) -> Result<VerifyingKey> {
    VerifyingKey::from_public_key_der(der).map_err(|_| ProtocolError::InvalidSchema)
}
pub fn signing_key_to_pkcs8_der(key: &SigningKey) -> Result<Zeroizing<Vec<u8>>> {
    key.to_pkcs8_der()
        .map(|der| Zeroizing::new(der.as_bytes().to_vec()))
        .map_err(|_| ProtocolError::InvalidSchema)
}
pub fn verifying_key_to_spki_der(key: &VerifyingKey) -> Result<Vec<u8>> {
    key.to_public_key_der()
        .map(|der| der.as_bytes().to_vec())
        .map_err(|_| ProtocolError::InvalidSchema)
}

/// Encrypts plaintext with exactly RFC 9180 base-mode P-256/HKDF-SHA256/AES-256-GCM.
pub fn hpke_seal(
    context: &HpkeContext,
    recipient_public_key: &[u8],
    plaintext: &[u8],
) -> Result<ResponseEnvelope> {
    context.validate()?;
    let public_key = <DhP256HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(recipient_public_key)
        .map_err(|_| ProtocolError::InvalidSchema)?;
    let mut rng = UnwrapErr(OsRng);
    let (enc, mut hpke_ctx) = setup_sender::<AesGcm256, HkdfSha256, DhP256HkdfSha256, _>(
        &OpModeS::Base,
        &public_key,
        HPKE_INFO,
        &mut rng,
    )
    .map_err(|_| ProtocolError::InvalidSchema)?;
    let bound_aad = canonical_json(context)?;
    let ciphertext = hpke_ctx
        .seal(plaintext, &bound_aad)
        .map_err(|_| ProtocolError::InvalidSchema)?;
    let envelope = ResponseEnvelope {
        protocol_version: context.protocol_version.clone(),
        request_id: context.request_id,
        recipient_key_id: context.recipient_key_id.clone(),
        active_device_epoch: context.active_device_epoch,
        direction: context.direction,
        enc: URL_SAFE_NO_PAD.encode(enc.to_bytes()),
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
    };
    envelope.validate()?;
    Ok(envelope)
}

pub fn hpke_open(
    envelope: &ResponseEnvelope,
    recipient_private_key: &[u8],
    expected_context: &HpkeContext,
) -> Result<Vec<u8>> {
    expected_context.validate()?;
    envelope.validate()?;
    if envelope.hpke_context() != *expected_context {
        return Err(ProtocolError::InvalidSignature);
    }
    let private_key =
        <DhP256HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(recipient_private_key)
            .map_err(|_| ProtocolError::InvalidSchema)?;
    let enc = <DhP256HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&decode_b64(
        &envelope.enc,
        ProtocolError::InvalidSchema,
    )?)
    .map_err(|_| ProtocolError::InvalidSchema)?;
    let ciphertext = decode_b64(&envelope.ciphertext, ProtocolError::InvalidSchema)?;
    let mut hpke_ctx = setup_receiver::<AesGcm256, HkdfSha256, DhP256HkdfSha256>(
        &OpModeR::Base,
        &private_key,
        &enc,
        HPKE_INFO,
    )
    .map_err(|_| ProtocolError::InvalidSignature)?;
    let bound_aad = canonical_json(expected_context)?;
    hpke_ctx
        .open(&ciphertext, &bound_aad)
        .map_err(|_| ProtocolError::InvalidSignature)
}

fn require_jcs(bytes: &[u8]) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidSchema)?;
    if canonical_json(&value)? != bytes {
        return Err(ProtocolError::InvalidCanonicalization);
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<()> {
    if value == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::UnsupportedProtocol)
    }
}
fn validate_epoch(value: u64) -> Result<()> {
    if (1..=MAX_SAFE_INTEGER).contains(&value) {
        Ok(())
    } else {
        Err(ProtocolError::InvalidSchema)
    }
}
fn validate_host(host: &HostIdentity) -> Result<()> {
    validate_identifier(&host.host_id, 1, 128, false)?;
    validate_identifier(&host.signing_key_id, 1, 128, false)?;
    validate_identifier(&host.encryption_key_id, 1, 128, false)
}
fn validate_device(device: &DeviceIdentity) -> Result<()> {
    validate_identifier(&device.device_id, 1, 128, false)?;
    validate_identifier(&device.signing_key_id, 1, 128, false)?;
    validate_identifier(&device.credential_id, 1, 2048, true)
}
fn validate_webauthn(value: &WebauthnAssertion) -> Result<()> {
    validate_b64(&value.client_data_json, 1, 16_384)?;
    validate_b64(&value.authenticator_data, 1, 16_384)?;
    validate_b64(&value.signature, 1, 16_384)?;
    if let Some(user_handle) = &value.user_handle {
        validate_b64(user_handle, 1, 16_384)?;
    }
    Ok(())
}
fn validate_signature_text(value: &str) -> Result<()> {
    if value.len() != 86 {
        return Err(ProtocolError::InvalidSignature);
    }
    let decoded = decode_b64(value, ProtocolError::InvalidSignature)?;
    if decoded.len() != 64 {
        return Err(ProtocolError::InvalidSignature);
    }
    Ok(())
}
fn validate_digest_text(value: &str) -> Result<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ProtocolError::InvalidActionDigest)
    }
}
fn validate_safe_text(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(|character| character.is_control())
    {
        Err(ProtocolError::InvalidSchema)
    } else {
        Ok(())
    }
}
fn validate_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 4096
        || !value.starts_with('/')
        || value.chars().any(|character| character.is_control())
        || value.contains("//")
        || (value != "/" && value.ends_with('/'))
        || value
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        Err(ProtocolError::InvalidSchema)
    } else {
        Ok(())
    }
}
fn validate_rp_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 253
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
    {
        Err(ProtocolError::InvalidSchema)
    } else {
        Ok(())
    }
}
fn validate_identifier(value: &str, min: usize, max: usize, url_safe: bool) -> Result<()> {
    if value.len() < min || value.len() > max {
        return Err(ProtocolError::InvalidSchema);
    }
    let valid = if url_safe {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    } else {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidSchema)
    }
}
fn validate_b64(value: &str, min: usize, max: usize) -> Result<()> {
    if value.len() < min || value.len() > max {
        return Err(ProtocolError::InvalidSchema);
    }
    let decoded = decode_b64(value, ProtocolError::InvalidSchema)?;
    if URL_SAFE_NO_PAD.encode(decoded) != value {
        return Err(ProtocolError::InvalidSchema);
    }
    Ok(())
}
fn decode_b64(value: &str, error: ProtocolError) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(value).map_err(|_| error)
}
fn parse_time(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ProtocolError::InvalidSchema)
}
fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl fmt::Display for RequestState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::PendingApproval => "pending_approval",
            Self::Delivered => "delivered",
            Self::AwaitingResponse => "awaiting_response",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::ResolvedElsewhere => "resolved_elsewhere",
            Self::Failed => "failed",
        };
        formatter.write_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpke::{kem::Kem as _, Serializable};

    fn time(value: &str) -> OffsetDateTime {
        OffsetDateTime::parse(value, &Rfc3339).unwrap()
    }
    fn host_key() -> SigningKey {
        SigningKey::from_slice(&[1_u8; 32]).unwrap()
    }
    fn device_key() -> SigningKey {
        SigningKey::from_slice(&[2_u8; 32]).unwrap()
    }
    fn action() -> Action {
        Action::Command {
            title: "Run tests".into(),
            summary: "Runs tests.".into(),
            executable: "/usr/bin/npm".into(),
            argv: vec!["test".into()],
            cwd: "/tmp/project".into(),
        }
    }
    fn request() -> ApprovalRequest {
        let mut request = ApprovalRequest {
            protocol_version: PROTOCOL_VERSION.into(),
            request_id: Uuid::parse_str("1f40fb52-3957-4f08-9016-84b2c526376b").unwrap(),
            nonce: "MDEyMzQ1Njc4OWFiY2RlZg".into(),
            issued_at: "2026-08-22T10:00:00Z".into(),
            expires_at: "2026-08-22T10:05:00Z".into(),
            active_device_epoch: 7,
            host: HostIdentity {
                host_id: "desktop-01".into(),
                signing_key_id: "host-sign-01".into(),
                encryption_key_id: "host-hpke-01".into(),
            },
            action: action(),
            action_digest: action_digest(&action()).unwrap(),
            host_signature: String::new(),
        };
        request.sign(&host_key()).unwrap();
        request
    }
    fn pairing() -> PairingRecord {
        PairingRecord {
            protocol_version: PROTOCOL_VERSION.into(),
            device_id: "pixel-01".into(),
            state: DeviceState::Active,
            active_device_epoch: 7,
            signing_key_id: "pixel-sign-01".into(),
            encryption_key_id: "pixel-hpke-01".into(),
            webauthn_credential_id: "Y3JlZGVudGlhbC0x".into(),
            rp_id: "approvals.example.test".into(),
            origin: "https://approvals.example.test".into(),
            paired_at: "2026-08-22T09:00:00Z".into(),
            revoked_at: None,
        }
    }
    fn response(request: &ApprovalRequest, decision: Decision) -> ApprovalResponse {
        let mut response = ApprovalResponse {
            protocol_version: PROTOCOL_VERSION.into(),
            request_id: request.request_id,
            action_digest: request.action_digest.clone(),
            active_device_epoch: 7,
            device: DeviceIdentity {
                device_id: "pixel-01".into(),
                signing_key_id: "pixel-sign-01".into(),
                credential_id: "Y3JlZGVudGlhbC0x".into(),
            },
            decision,
            webauthn: WebauthnAssertion {
                client_data_json: "Y2xpZW50LWRhdGE".into(),
                authenticator_data: "YXV0aC1kYXRh".into(),
                signature: "d2ViYXV0aG4tc2ln".into(),
                user_handle: None,
            },
            device_signature: String::new(),
        };
        response.sign(&device_key()).unwrap();
        response
    }

    #[test]
    fn jcs_digest_and_fixed_width_ecdsa_are_stable() {
        let request = request();
        assert_eq!(
            request.action_digest,
            "d1f82d59d436e1120f2ee2028707110e5a76cfd473c1e0bef83206f7f5b6b69d"
        );
        assert_eq!(request.host_signature.len(), 86);
        request
            .verify_signature(host_key().verifying_key())
            .unwrap();
        assert_eq!(canonical_json(&request.action).unwrap(), br#"{"argv":["test"],"cwd":"/tmp/project","executable":"/usr/bin/npm","kind":"command","summary":"Runs tests.","title":"Run tests"}"#);
    }

    #[test]
    fn signed_response_accepts_once_and_exactly_binds_action() {
        let request = request();
        let mut lifecycle = RequestLifecycle::new(&request, "pixel-01").unwrap();
        let approved_response = response(&request, Decision::Approve);
        lifecycle
            .accept_response(
                &request,
                &approved_response,
                &pairing(),
                device_key().verifying_key(),
                time("2026-08-22T10:04:59Z"),
            )
            .unwrap();
        assert_eq!(lifecycle.state(), RequestState::Accepted);
        assert_eq!(
            lifecycle.accept_response(
                &request,
                &approved_response,
                &pairing(),
                device_key().verifying_key(),
                time("2026-08-22T10:04:59Z")
            ),
            Err(ProtocolError::ResponseReplayed)
        );
        let mut mismatch = response(&request, Decision::Approve);
        mismatch.action_digest = "f".repeat(64);
        mismatch.sign(&device_key()).unwrap();
        let mut lifecycle = RequestLifecycle::new(&request, "pixel-01").unwrap();
        assert_eq!(
            lifecycle.accept_response(
                &request,
                &mismatch,
                &pairing(),
                device_key().verifying_key(),
                time("2026-08-22T10:04:59Z")
            ),
            Err(ProtocolError::InvalidActionDigest)
        );
    }

    #[test]
    fn expiry_boundary_epoch_and_choice_mismatch_fail_closed() {
        let base_request = request();
        let approved_response = response(&base_request, Decision::Approve);
        let mut lifecycle = RequestLifecycle::new(&base_request, "pixel-01").unwrap();
        assert_eq!(
            lifecycle.accept_response(
                &base_request,
                &approved_response,
                &pairing(),
                device_key().verifying_key(),
                time("2026-08-22T10:05:00Z")
            ),
            Err(ProtocolError::RequestExpired)
        );
        let enum_action = Action::UserInput {
            title: "Pick".into(),
            summary: "Pick one.".into(),
            input: ClosedInput::Enum {
                choices: vec![Choice {
                    id: "yes".into(),
                    label: "Yes".into(),
                }],
            },
        };
        let mut enum_request = super::tests::request();
        enum_request.action = enum_action;
        enum_request.action_digest = action_digest(&enum_request.action).unwrap();
        enum_request.sign(&host_key()).unwrap();
        let invalid = response(
            &enum_request,
            Decision::Enum {
                choice_id: "no".into(),
            },
        );
        assert_eq!(
            validate_response(
                &enum_request,
                &invalid,
                &pairing(),
                device_key().verifying_key()
            ),
            Err(ProtocolError::InvalidSchema)
        );
        let mut stale = response(&base_request, Decision::Approve);
        stale.active_device_epoch = 6;
        stale.sign(&device_key()).unwrap();
        assert_eq!(
            validate_response(
                &base_request,
                &stale,
                &pairing(),
                device_key().verifying_key()
            ),
            Err(ProtocolError::StaleDeviceEpoch)
        );
    }

    #[test]
    fn hpke_round_trip_and_context_tampering_fail() {
        let mut rng = UnwrapErr(OsRng);
        let (private, public) = DhP256HkdfSha256::gen_keypair(&mut rng);
        let context = HpkeContext::new(
            Uuid::new_v4(),
            "recipient-01".into(),
            7,
            HpkeDirection::DeviceToHost,
        );
        let envelope = hpke_seal(&context, &public.to_bytes(), b"secret payload").unwrap();
        assert_eq!(envelope.enc.len(), 87);
        assert_eq!(
            hpke_open(&envelope, &private.to_bytes(), &context).unwrap(),
            b"secret payload"
        );
        let (wrong_private, _) = DhP256HkdfSha256::gen_keypair(&mut rng);
        assert_eq!(
            hpke_open(&envelope, &wrong_private.to_bytes(), &context),
            Err(ProtocolError::InvalidSignature)
        );

        let mut bad_version = envelope.clone();
        bad_version.protocol_version = "zodex.remote-approval.v99".into();
        assert_eq!(
            hpke_open(&bad_version, &private.to_bytes(), &context),
            Err(ProtocolError::UnsupportedProtocol)
        );

        let mut bad_req_id = envelope.clone();
        bad_req_id.request_id = Uuid::new_v4();
        assert_eq!(
            hpke_open(&bad_req_id, &private.to_bytes(), &context),
            Err(ProtocolError::InvalidSignature)
        );

        let mut bad_key_id = envelope.clone();
        bad_key_id.recipient_key_id = "recipient-wrong".into();
        assert_eq!(
            hpke_open(&bad_key_id, &private.to_bytes(), &context),
            Err(ProtocolError::InvalidSignature)
        );

        let mut wrong_epoch = context.clone();
        wrong_epoch.active_device_epoch += 1;
        assert_eq!(
            hpke_open(&envelope, &private.to_bytes(), &wrong_epoch),
            Err(ProtocolError::InvalidSignature)
        );

        let mut wrong_direction = context.clone();
        wrong_direction.direction = HpkeDirection::HostToDevice;
        assert_eq!(
            hpke_open(&envelope, &private.to_bytes(), &wrong_direction),
            Err(ProtocolError::InvalidSignature)
        );
    }

    #[test]
    fn normalized_path_validation_enforces_strict_formatting() {
        assert!(validate_path("/usr/bin/npm").is_ok());
        assert!(validate_path("/").is_ok());
        assert!(validate_path("/home/user/file.rs").is_ok());
        assert_eq!(validate_path(""), Err(ProtocolError::InvalidSchema));
        assert_eq!(validate_path("usr/bin"), Err(ProtocolError::InvalidSchema));
        assert_eq!(
            validate_path("/usr//bin"),
            Err(ProtocolError::InvalidSchema)
        );
        assert_eq!(
            validate_path("/usr/./bin"),
            Err(ProtocolError::InvalidSchema)
        );
        assert_eq!(
            validate_path("/usr/../bin"),
            Err(ProtocolError::InvalidSchema)
        );
        assert_eq!(
            validate_path("/usr/bin/"),
            Err(ProtocolError::InvalidSchema)
        );
        assert_eq!(
            validate_path("/usr/bin\n"),
            Err(ProtocolError::InvalidSchema)
        );
        assert_eq!(
            validate_path("/usr/bin\0"),
            Err(ProtocolError::InvalidSchema)
        );
    }

    #[test]
    fn relay_frame_schema_and_operation_constraints() {
        let mut publish = RelayFrame {
            protocol_version: PROTOCOL_VERSION.into(),
            operation: RelayOperation::Publish,
            request_id: Uuid::new_v4(),
            sender: "desktop-01".into(),
            recipient: "pixel-01".into(),
            created_at: "2026-08-22T10:00:00Z".into(),
            ciphertext: Some("b3BhcXVlLWNpcGhlcnRleHQ".into()),
            ack_id: None,
        };
        assert!(publish.validate().is_ok());
        publish.ack_id = Some("MDEyMzQ1Njc4OWFiY2RlZg".into());
        assert_eq!(publish.validate(), Err(ProtocolError::InvalidSchema));

        let mut ack = RelayFrame {
            protocol_version: PROTOCOL_VERSION.into(),
            operation: RelayOperation::Ack,
            request_id: Uuid::new_v4(),
            sender: "pixel-01".into(),
            recipient: "desktop-01".into(),
            created_at: "2026-08-22T10:00:00Z".into(),
            ciphertext: None,
            ack_id: Some("MDEyMzQ1Njc4OWFiY2RlZg".into()),
        };
        assert!(ack.validate().is_ok());
        ack.ciphertext = Some("b3BhcXVlLWNpcGhlcnRleHQ".into());
        assert_eq!(ack.validate(), Err(ProtocolError::InvalidSchema));

        for op in [
            RelayOperation::Connect,
            RelayOperation::Subscribe,
            RelayOperation::PushWakeup,
            RelayOperation::Health,
        ] {
            let mut other = RelayFrame {
                protocol_version: PROTOCOL_VERSION.into(),
                operation: op,
                request_id: Uuid::new_v4(),
                sender: "desktop-01".into(),
                recipient: "pixel-01".into(),
                created_at: "2026-08-22T10:00:00Z".into(),
                ciphertext: None,
                ack_id: None,
            };
            assert!(other.validate().is_ok());
            other.ciphertext = Some("b3BhcXVlLWNpcGhlcnRleHQ".into());
            assert_eq!(other.validate(), Err(ProtocolError::InvalidSchema));
            other.ciphertext = None;
            other.ack_id = Some("MDEyMzQ1Njc4OWFiY2RlZg".into());
            assert_eq!(other.validate(), Err(ProtocolError::InvalidSchema));
        }
    }

    #[test]
    fn signing_canonicalization_digest_and_tampering_protection() {
        let mut req = request();
        let original_digest = req.request_digest().unwrap();
        let sig = sign_digest(&host_key(), &original_digest).unwrap();
        assert!(verify_digest(host_key().verifying_key(), &original_digest, &sig).is_ok());

        let mut tampered_sig = sig;
        tampered_sig.replace_range(0..1, "B");
        assert_eq!(
            verify_digest(host_key().verifying_key(), &original_digest, &tampered_sig),
            Err(ProtocolError::InvalidSignature)
        );

        req.nonce = "MDEyMzQ1Njc4OWFiY2RlZ2dn".into();
        assert_eq!(
            req.verify_signature(host_key().verifying_key()),
            Err(ProtocolError::InvalidSignature)
        );
    }

    #[test]
    fn typed_fixture_coverage_for_all_13_fixtures() {
        let pairing_json =
            include_str!("../../remote-approval-protocol/fixtures/pairing-record.json");
        let pairing: PairingRecord = serde_json::from_str(pairing_json).unwrap();
        pairing.validate().unwrap();

        let req_fixtures = [
            (
                "command",
                include_str!("../../remote-approval-protocol/fixtures/request-command.json"),
            ),
            (
                "file-change",
                include_str!("../../remote-approval-protocol/fixtures/request-file-change.json"),
            ),
            (
                "permission",
                include_str!("../../remote-approval-protocol/fixtures/request-permission.json"),
            ),
            (
                "boolean",
                include_str!(
                    "../../remote-approval-protocol/fixtures/request-user-input-boolean.json"
                ),
            ),
            (
                "enum",
                include_str!(
                    "../../remote-approval-protocol/fixtures/request-user-input-enum.json"
                ),
            ),
        ];
        for (name, json) in req_fixtures {
            let req: ApprovalRequest = serde_json::from_str(json).unwrap();
            let calculated = action_digest(&req.action).unwrap();
            println!(
                "{name}: in_file={}, calculated={calculated}",
                req.action_digest
            );
            req.validate().unwrap();
            assert_eq!(req.action_digest, action_digest(&req.action).unwrap());
            assert_eq!(
                req.verify_signature(host_key().verifying_key()),
                Err(ProtocolError::InvalidSignature)
            );
        }

        let resp_fixtures = [
            include_str!("../../remote-approval-protocol/fixtures/response-approve.json"),
            include_str!("../../remote-approval-protocol/fixtures/response-reject.json"),
            include_str!(
                "../../remote-approval-protocol/fixtures/response-user-input-boolean.json"
            ),
            include_str!("../../remote-approval-protocol/fixtures/response-user-input-enum.json"),
        ];
        for json in resp_fixtures {
            let resp: ApprovalResponse = serde_json::from_str(json).unwrap();
            resp.validate().unwrap();
            assert_eq!(
                resp.verify_signature(device_key().verifying_key()),
                Err(ProtocolError::InvalidSignature)
            );
        }

        let env_json =
            include_str!("../../remote-approval-protocol/fixtures/response-envelope.json");
        let env: ResponseEnvelope = serde_json::from_str(env_json).unwrap();
        env.validate().unwrap();

        let relay_fixtures = [
            include_str!("../../remote-approval-protocol/fixtures/relay-publish.json"),
            include_str!("../../remote-approval-protocol/fixtures/relay-ack.json"),
        ];
        for json in relay_fixtures {
            let frame: RelayFrame = serde_json::from_str(json).unwrap();
            frame.validate().unwrap();
        }
    }
    #[test]
    fn placeholders_unknown_fields_and_noncanonical_wire_never_verify() {
        let fixture = include_str!("../../remote-approval-protocol/fixtures/request-command.json");
        let placeholder: ApprovalRequest = serde_json::from_str(fixture).unwrap();
        assert_eq!(
            placeholder.verify_signature(host_key().verifying_key()),
            Err(ProtocolError::InvalidSignature)
        );
        let unknown = br#"{"action_digest":"0000000000000000000000000000000000000000000000000000000000000000","action":{"argv":[],"cwd":"/tmp","executable":"/bin/true","kind":"command","summary":"s","title":"t"},"active_device_epoch":1,"expires_at":"2026-08-22T10:01:00Z","host":{"encryption_key_id":"e","host_id":"h","signing_key_id":"s"},"host_signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","issued_at":"2026-08-22T10:00:00Z","nonce":"MDEyMzQ1Njc4OWFiY2RlZg","protocol_version":"zodex.remote-approval.v1","request_id":"1f40fb52-3957-4f08-9016-84b2c526376b","unexpected":true}"#;
        assert!(serde_json::from_slice::<ApprovalRequest>(unknown).is_err());
        assert_eq!(
            ApprovalRequest::parse_transmitted(unknown),
            Err(ProtocolError::InvalidCanonicalization)
        );
        let noncanonical = serde_json::to_vec(&request()).unwrap();
        assert_eq!(
            ApprovalRequest::parse_transmitted(&noncanonical),
            Err(ProtocolError::InvalidCanonicalization)
        );
    }

    #[test]
    fn pairing_relay_and_envelope_limits_are_enforced() {
        pairing().validate().unwrap();
        let mut pair = pairing();
        pair.origin = "http://approvals.example.test".into();
        assert_eq!(pair.validate(), Err(ProtocolError::InvalidSchema));
        let frame = RelayFrame {
            protocol_version: PROTOCOL_VERSION.into(),
            operation: RelayOperation::Publish,
            request_id: Uuid::new_v4(),
            sender: "desktop".into(),
            recipient: "pixel".into(),
            created_at: "2026-08-22T10:00:00Z".into(),
            ciphertext: None,
            ack_id: None,
        };
        assert_eq!(frame.validate(), Err(ProtocolError::InvalidSchema));
        let envelope = ResponseEnvelope {
            protocol_version: PROTOCOL_VERSION.into(),
            request_id: Uuid::new_v4(),
            recipient_key_id: "key".into(),
            active_device_epoch: 1,
            direction: HpkeDirection::DeviceToHost,
            enc: "A".repeat(86),
            ciphertext: "YQ".into(),
        };
        assert_eq!(envelope.validate(), Err(ProtocolError::InvalidSchema));
    }

    #[test]
    fn checked_in_fixtures_are_parseable_but_not_cryptographic_vectors() {
        for fixture in [
            include_str!("../../remote-approval-protocol/fixtures/request-command.json"),
            include_str!("../../remote-approval-protocol/fixtures/response-approve.json"),
            include_str!("../../remote-approval-protocol/fixtures/pairing-record.json"),
            include_str!("../../remote-approval-protocol/fixtures/relay-publish.json"),
        ] {
            serde_json::from_str::<serde_json::Value>(fixture).unwrap();
        }
        let request: ApprovalRequest = serde_json::from_str(include_str!(
            "../../remote-approval-protocol/fixtures/request-command.json"
        ))
        .unwrap();
        let response: ApprovalResponse = serde_json::from_str(include_str!(
            "../../remote-approval-protocol/fixtures/response-approve.json"
        ))
        .unwrap();
        assert!(request
            .verify_signature(host_key().verifying_key())
            .is_err());
        assert!(response
            .verify_signature(device_key().verifying_key())
            .is_err());
    }
}
