// Copyright 2025 TikTok Inc. and/or its affiliates
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use base64::prelude::*;
use blind_rsa::BlindPublicKey;
use blind_rsa::blinder::RsaBlinder;
use bytes::Bytes;
use futures::TryStreamExt;
use futures::stream::Stream;
use futures::stream::StreamExt;
use identity::IdentityClient;
use noise::{NoiseNnInitiator, NoiseNnTransport};
use ohttp_wrap::{ClientRequest, KeyConfig, Message, Mode, OhttpClient};
use p256::ecdsa::{Signature, signature::Verifier};
use rand_core::{OsRng, RngCore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::codec::LengthDelimitedCodec;
use tokio_util::io::StreamReader;
use tracing::error;
#[cfg(feature = "attestation")]
use tracing::info;
use types::{
    ApiError, ApiResponse, AttestationResponse, HandShakeResp, ReportData, UploadDocumentReq,
    async_rw::{self},
    http::{
        HttpClient,
        reqwest::{
            IntoUrl, Url,
            header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
        },
    },
    keys::{
        BlindMessageRequest, BlindMessageResponse, ContextKey, PublicKeyFields,
        decode_verifying_key,
    },
};

#[cfg(feature = "attestation")]
use verifier::{InitDataHash, ReportData as TeeReportData, to_verifier};

use futures::{AsyncReadExt, AsyncWriteExt};

const ESTABLISH_PATH: &str = "/v1/establish";
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const HANDSHAKE_WITH_ATTESTATION_PATH: &str = "/v1/handshake";
const ATTESTATION_PATH: &str = "/v1/attestation";
const UPLOAD_KEY_PATH: &str = "/v1/keys";
const UPLOAD_DOCUMENT_PATH: &str = "/v1/documents";
#[cfg(feature = "attestation")]
const NVIDIA_NONCE_SIZE: usize = 32;

const SESSION_ID_HEADER: &str = "X-Session-ID";
const IDENTITY_TOKEN_HEADER: &str = "X-Identity-Token";
const IDENTITY_MESSAGE_HEADER: &str = "X-Identity-Message";
const PVC_ROOT_DIR: &str = ".pvc";
const KEY_FILE: &str = "secret";

pub type Claim = Vec<(Value, String)>;
pub type ChatCompletionStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PvcClientConfig {
    pub identity_server_url: String,
    pub ohttp_gateway_url: String,
    pub relay_url: String,
    pub target_url: String,
}

pub struct PvcClient {
    identity_server_url: Url,
    relay_url: Url,
    target_url: String,
    http_client: HttpClient,
    ohttp_key_config: KeyConfig,
    session_id: Option<String>,
    noise_transport: Option<Arc<Mutex<NoiseNnTransport>>>,
}

impl PvcClient {
    pub async fn new<U: IntoUrl + Send>(
        identity_server_url: U,
        ohttp_gateway_url: U,
        relay_url: U,
        target_url: String,
    ) -> Result<Self> {
        let key_config = PvcClient::ohttp_initialize(ohttp_gateway_url).await?;
        Ok(Self {
            identity_server_url: identity_server_url.into_url()?,
            relay_url: relay_url.into_url()?,
            target_url,
            http_client: HttpClient::new(),
            ohttp_key_config: key_config,
            session_id: None,
            noise_transport: None,
        })
    }

    pub async fn from_config(config: &PvcClientConfig) -> Result<Self> {
        Self::new(
            config.identity_server_url.clone(),
            config.ohttp_gateway_url.clone(),
            config.relay_url.clone(),
            config.target_url.clone(),
        )
        .await
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub async fn handshake_with_attestation(&mut self, id_token: Option<String>) -> Result<()> {
        let identity = self.get_identity_token(id_token).await?;
        let claims = self.attest_with_identity(None, &identity).await?;
        let verifying_key = extract_report_data(&claims);
        self.establish_with_identity(verifying_key, &identity)
            .await?;
        Ok(())
    }

    pub async fn attest(
        &mut self,
        nonce: Option<String>,
        id_token: Option<String>,
    ) -> Result<Claim> {
        let identity = self.get_identity_token(id_token).await?;
        self.attest_with_identity(nonce, &identity).await
    }

    async fn attest_with_identity(
        &mut self,
        nonce: Option<String>,
        identity: &(String, String),
    ) -> Result<Claim> {
        let handshake_header = self.generate_identity_header(identity)?;

        let resp: AttestationResponse = match &nonce {
            Some(nonce) => {
                let nonce_data = BASE64_STANDARD.decode(nonce)?;
                self.ohttp_post(
                    &self.target_url,
                    ATTESTATION_PATH,
                    Some(handshake_header),
                    Some(nonce_data),
                )
                .await?
                .ok_or(ApiError::MissingData)?
            }
            None => self
                .ohttp_post(
                    &self.target_url,
                    HANDSHAKE_WITH_ATTESTATION_PATH,
                    Some(handshake_header),
                    None,
                )
                .await?
                .ok_or(ApiError::MissingData)?,
        };

        let claim = {
            #[cfg(feature = "attestation")]
            {
                let (cpu_report_data, gpu_report_data) = match &nonce {
                    Some(n) => {
                        let decoded_nonce = BASE64_STANDARD.decode(n)?;
                        assert!(decoded_nonce.len() >= NVIDIA_NONCE_SIZE);
                        (
                            TeeReportData::Value(&decoded_nonce.clone()),
                            TeeReportData::Value(&decoded_nonce.clone()[0..NVIDIA_NONCE_SIZE]),
                        )
                    }
                    None => (TeeReportData::NotProvided, TeeReportData::NotProvided),
                };

                let verifier = to_verifier(&resp.tee_type, None).await.unwrap();
                let mut claim = verifier
                    .evaluate(resp.evidence, &cpu_report_data, &InitDataHash::NotProvided)
                    .await?;
                info!("device tee num: {:?}", resp.device_evidences);
                if let Some((tee, evidence)) = resp.device_evidences {
                    let json_data = r#"{
                        "nvidia_verifier": {
                            "type": "Remote",
                            "verifier_url": "https://nras.attestation.nvidia.com/v4/attest"
                        }
                    }"#;
                    let config: verifier::VerifierConfig = serde_json::from_str(json_data).unwrap();
                    let device_verifier = to_verifier(&tee, Some(config)).await.unwrap();
                    match device_verifier
                        .evaluate(
                            evidence.clone(),
                            &gpu_report_data,
                            &InitDataHash::NotProvided,
                        )
                        .await
                    {
                        Ok(mut device_claim) => {
                            claim.append(&mut device_claim);
                        }
                        Err(e) => {
                            error!("failed to verify device evidence {:?}", e)
                        }
                    };
                }
                claim
            }

            #[cfg(not(feature = "attestation"))]
            {
                let mut claim = Vec::new();
                claim.push((resp.evidence, "cpu".to_string()));
                claim
            }
        };

        if self.session_id.is_none() && resp.sid.is_some() {
            self.session_id = resp.sid;
        }
        Ok(claim)
    }

    pub async fn establish(
        &mut self,
        verifying_key: [u8; 64],
        id_token: Option<String>,
    ) -> Result<()> {
        let identity = self.get_identity_token(id_token).await?;
        self.establish_with_identity(verifying_key, &identity).await
    }

    async fn establish_with_identity(
        &mut self,
        verifying_key: [u8; 64],
        identity: &(String, String),
    ) -> Result<()> {
        let mut noise_initiator =
            NoiseNnInitiator::new("Noise_NN_25519_ChaChaPoly_BLAKE2s".parse().unwrap(), None)?;

        let handshake_header = self.generate_identity_header(identity)?;

        let ephemeral = noise_initiator.generate_ephemeral()?;
        let resp: HandShakeResp = self
            .ohttp_post(
                &self.target_url,
                ESTABLISH_PATH,
                Some(handshake_header),
                Some(ephemeral.clone()),
            )
            .await?
            .ok_or(ApiError::MissingData)?;

        verify_noise_script_signature(verifying_key, &ephemeral, &resp.data, &resp.signature)?;

        let tp = noise_initiator.recv_response(&resp.data)?;
        self.noise_transport = Some(Arc::new(Mutex::new(tp)));
        Ok(())
    }

    #[cfg(feature = "noise")]
    fn encrypt_message(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        if let Some(t) = &self.noise_transport {
            let mut transport = t
                .try_lock()
                .map_err(|_| anyhow!("Failed to acquire lock on noise transport"))?;
            transport.encrypt(message)
        } else {
            Err(anyhow!("noise transport is none, internal error happens"))
        }
    }

    #[cfg(not(feature = "noise"))]
    fn encrypt_message(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        Ok(message.to_vec())
    }

    pub async fn upload_encryption_key(&mut self, session_key: &ContextKey) -> Result<()> {
        let encrypted_key = self.encrypt_message(&session_key.0)?;
        let headers = self.generate_header();

        let _: Option<()> = self
            .ohttp_post(
                &self.target_url,
                UPLOAD_KEY_PATH,
                Some(headers),
                Some(encrypted_key),
            )
            .await?;
        Ok(())
    }

    pub async fn upload_knowledge_document(&mut self, filename: &str, content: &str) -> Result<()> {
        let req = UploadDocumentReq {
            filename: filename.to_string(),
            content: content.to_string(),
        };
        let req_str = serde_json::to_string(&req).unwrap();
        let encrypted = self.encrypt_message(req_str.as_bytes())?;
        let headers = self.generate_header();
        let _: Option<()> = self
            .ohttp_post(
                &self.target_url,
                UPLOAD_DOCUMENT_PATH,
                Some(headers),
                Some(encrypted),
            )
            .await?;
        Ok(())
    }

    pub async fn chat_completions(
        &mut self,
        h: Option<&HeaderMap>,
        body: &[u8],
    ) -> Result<ChatCompletionStream> {
        let mut headers = match h {
            Some(h) => h.clone(),
            None => HeaderMap::new(),
        };
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        if let Some(sid) = &self.session_id {
            headers.insert(SESSION_ID_HEADER, HeaderValue::from_str(sid)?);
        }

        let encrypted_input = self
            .encrypt_message(body)
            .map_err(|e| anyhow!("Failed to encrypt message: {}", e))?;
        let stream = self
            .ohttp_post_stream(
                &self.target_url,
                CHAT_COMPLETIONS_PATH,
                Some(headers),
                Some(encrypted_input),
            )
            .await?;

        self.decrypt_cipher_stream(stream).await
    }

    async fn get_identity_token(&self, id_token: Option<String>) -> Result<(String, String)> {
        let pk: BlindPublicKey = self.fetch_public_key().await?;

        let msg = {
            let mut msg = vec![0; 20];
            OsRng.fill_bytes(&mut msg);
            msg
        };
        let blinder = RsaBlinder {};
        let state = blinder
            .blind(&msg, pk.clone())
            .context("Failed to blind message")?;
        let blinded_msg = state.blinded_message()?;

        let blind_sig_bytes = self
            .request_blind_signature(&blinded_msg, id_token)
            .await
            .context("Failed to request blind signature")?;

        let sig = blinder
            .verify(&blind_sig_bytes, &state, pk)
            .context("Failed to verify unblind signature")?;

        Ok((hex::encode(&msg), hex::encode(&sig)))
    }

    fn generate_header(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        if let Some(sid) = &self.session_id {
            headers.insert(SESSION_ID_HEADER, HeaderValue::from_str(sid).unwrap());
        }
        headers
    }

    fn generate_identity_header(&self, identity: &(String, String)) -> Result<HeaderMap> {
        let (msg, token) = identity;
        let mut headers = self.generate_header();
        headers.insert(IDENTITY_TOKEN_HEADER, HeaderValue::from_str(token)?);
        headers.insert(IDENTITY_MESSAGE_HEADER, HeaderValue::from_str(msg)?);
        Ok(headers)
    }

    async fn decrypt_cipher_stream(
        &mut self,
        stream: Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>,
    ) -> Result<ChatCompletionStream> {
        let noise_transport = self
            .noise_transport
            .clone()
            .ok_or_else(|| anyhow!("noise transport missing"))?;
        let reader = StreamReader::new(
            stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
        );
        let codec = LengthDelimitedCodec::builder()
            .length_field_length(4)
            .max_frame_length(1024 * 1024)
            .new_read(reader);
        let decrypted_stream = codec
            .map_err(|e| anyhow!(format!("failed to read cipher stream {}", e)))
            .then(move |frame_res| {
                let noise_transport = noise_transport.clone();
                async move {
                    let frame = match frame_res {
                        Ok(f) => f,
                        Err(e) => return Err(e),
                    };
                    #[cfg(feature = "noise")]
                    {
                        let mut transport = noise_transport.lock().await;
                        transport
                            .decrypt(&frame)
                            .map_err(|e| anyhow!("Decryption failed: {}", e))
                            .and_then(|d| {
                                String::from_utf8(d).map_err(|e| anyhow!("UTF8 error: {}", e))
                            })
                            .map(Some)
                    }
                    #[cfg(not(feature = "noise"))]
                    {
                        let _ = &noise_transport;
                        Ok(Some(String::from_utf8_lossy(&frame).to_string()))
                    }
                }
            })
            .try_filter_map(|res| async move { Ok(res) });
        Ok(Box::pin(decrypted_stream))
    }
}

pub fn create_or_get_encryption_key() -> Result<ContextKey> {
    let path = key_path_in_home()?;
    match fs::read(&path) {
        Ok(buf) => Ok(ContextKey(buf)),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }

            let mut key = vec![0u8; 32];
            OsRng.fill_bytes(&mut key);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(&key)?;
            file.sync_all()?;

            Ok(ContextKey(key))
        }
        Err(err) => Err(err.into()),
    }
}

pub fn pvc_home_dir() -> Result<PathBuf> {
    let home = std::env::home_dir().ok_or_else(|| anyhow!("home directory is unavailable"))?;
    Ok(home.join(PVC_ROOT_DIR))
}

pub fn key_path_in_home() -> Result<PathBuf> {
    Ok(pvc_home_dir()?.join(KEY_FILE))
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub fn write_private_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_parent_dir(path)?;
    let data = serde_json::to_vec_pretty(value)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&data)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub fn extract_report_data(claims: &Claim) -> ReportData {
    let cpu = claims
        .iter()
        .find(|(_val, key)| key == "cpu")
        .map(|(val, _key)| val)
        .unwrap();
    let report_data_str = cpu["report_data"].as_str().unwrap();
    match hex::decode(report_data_str) {
        Ok(r) => r.try_into().unwrap(),
        Err(_) => BASE64_STANDARD
            .decode(report_data_str)
            .unwrap()
            .try_into()
            .unwrap(),
    }
}

fn verify_noise_script_signature(
    verifying_key: [u8; 64],
    e: &[u8],
    ee: &[u8],
    signature: &[u8],
) -> Result<()> {
    let verifying_key = decode_verifying_key(verifying_key)?;
    let mut message = Vec::new();
    message.extend_from_slice(e);
    message.extend_from_slice(ee);
    verifying_key
        .verify(&message, &Signature::from_slice(signature)?)
        .map_err(|_| anyhow!("failed to verify noise script signature"))
}

#[async_trait]
impl IdentityClient for PvcClient {
    async fn fetch_public_key(&self) -> Result<BlindPublicKey> {
        let resp: PublicKeyFields = self
            .http_client
            .get(self.identity_server_url.join("pubkey")?, None)
            .await?
            .ok_or(ApiError::MissingData)?;
        let pk = BlindPublicKey {
            n: BASE64_STANDARD.decode(&resp.n)?,
            e: BASE64_STANDARD.decode(&resp.e)?,
        };
        Ok(pk)
    }

    async fn request_blind_signature(
        &self,
        blinded_message: &[u8],
        id_token: Option<String>,
    ) -> Result<Vec<u8>> {
        let body = BlindMessageRequest {
            blinded_message: blinded_message.to_vec(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(id_token) = id_token {
            headers.insert(
                AUTHORIZATION,
                format!("Bearer {}", id_token).parse().unwrap(),
            );
        }
        let body_bytes = serde_json::to_string(&body)?;
        let resp: BlindMessageResponse = self
            .http_client
            .post(
                self.identity_server_url.join("sign")?,
                body_bytes.as_bytes(),
                Some(headers),
            )
            .await?
            .ok_or(ApiError::MissingData)?;
        let sig: Vec<u8> = BASE64_STANDARD.decode(resp.signature)?;
        Ok(sig)
    }
}

#[async_trait]
impl OhttpClient for PvcClient {
    async fn ohttp_initialize<U>(ohttp_gateway_url: U) -> Result<KeyConfig>
    where
        U: IntoUrl + Send,
    {
        let ohttp_gateway_url: Url = ohttp_gateway_url.into_url()?;
        let config_url = ohttp_gateway_url.join("ohttp-configs")?;
        let http_client = HttpClient::new();
        let resp = http_client.get_with_raw_response(config_url, None).await?;
        let cfg_bytes = resp.bytes().await?;
        if cfg_bytes.len() < 2 {
            return Err(anyhow!("Invalid ohttp-configs response"));
        }
        let length_prefix = u16::from_be_bytes([cfg_bytes[0], cfg_bytes[1]]);
        if length_prefix != (cfg_bytes.len() - 2) as u16 {
            return Err(anyhow!("Invalid length prefix for ohttp-configs"));
        }
        let key_config: KeyConfig = KeyConfig::decode(&cfg_bytes[2..])?;
        Ok(key_config)
    }

    async fn ohttp_post_stream(
        &self,
        target_server: &str,
        path: &str,
        headers: Option<HeaderMap>,
        body: Option<Vec<u8>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>> {
        let mut request = Message::request(
            b"POST".to_vec(),
            b"http".to_vec(),
            target_server.as_bytes().to_vec(),
            path.as_bytes().to_vec(),
        );
        if let Some(b) = body {
            request.write_content(&b);
        }
        if let Some(headers) = headers {
            for (name, value) in headers.iter() {
                request.put_header(name.as_str(), value.as_bytes());
            }
        }

        let mut request_buf = Vec::new();
        request.write_bhttp(Mode::KnownLength, &mut request_buf)?;
        let req: ClientRequest = ClientRequest::from_config(&mut self.ohttp_key_config.clone())?;

        let (encrypted_request, client_request) = {
            let (request_write, mut request_read) = async_rw::create_channel_pair();
            let mut enc_request_writer = req
                .encapsulate_stream(request_write)
                .map_err(|e| anyhow!("Failed to encapsulate request: {}", e))?;

            let reader_task: tokio::task::JoinHandle<Result<Vec<u8>, anyhow::Error>> =
                tokio::spawn(async move {
                    let mut encrypted_request = Vec::new();
                    request_read
                        .read_to_end(&mut encrypted_request)
                        .await
                        .map_err(|e| anyhow!("Failed to read from encrypt request: {}", e))?;
                    Ok(encrypted_request)
                });

            enc_request_writer
                .write_all(&request_buf)
                .await
                .map_err(|e| anyhow!("Failed to write request: {}", e))?;

            enc_request_writer
                .close()
                .await
                .map_err(|e| anyhow!("Failed to close writer: {}", e))?;

            let encrypted_request = reader_task
                .await
                .map_err(|e| anyhow!("Failed reader task: {}", e))??;

            (encrypted_request, enc_request_writer)
        };

        let relay_request_body = {
            #[cfg(feature = "base64")]
            {
                BASE64_STANDARD.encode(&encrypted_request).into_bytes()
            }
            #[cfg(not(feature = "base64"))]
            {
                encrypted_request
            }
        };

        let mut outer_headers: HeaderMap = HeaderMap::new();
        outer_headers.insert(CONTENT_TYPE, "message/ohttp-req".parse().unwrap());
        let response = self
            .http_client
            .post_with_raw_response(
                self.relay_url.clone(),
                &relay_request_body,
                Some(outer_headers),
            )
            .await?
            .error_for_status()?;

        let (mut channel_writer, channel_reader) = async_rw::create_channel_pair_with_size(1024);
        let response_read = client_request
            .response(channel_reader)
            .map_err(|_| anyhow!("Failed to set response".to_string()))?;

        let resp_stream = response.bytes_stream();
        tokio::spawn(async move {
            let mut resp_stream = Box::pin(resp_stream);
            while let Some(bytes_chunk) = resp_stream.next().await {
                match bytes_chunk {
                    Ok(chunk) => {
                        if let Err(e) = channel_writer.write_all(&chunk).await {
                            error!("Failed to write chunk: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Error from ohttp response stream: {}", e);
                        break;
                    }
                }
            }

            let _ = channel_writer.close().await;
        });

        Ok(Box::pin(futures::stream::unfold(
            response_read,
            |mut response_read| {
                Box::pin(async move {
                    let mut buffer = vec![0; 1024];
                    match response_read.read(&mut buffer).await {
                        Ok(0) => None,
                        Ok(n) => Some((Ok(Bytes::copy_from_slice(&buffer[..n])), response_read)),
                        Err(e) => {
                            error!("OHTTP decryption failed: {}", e);
                            Some((Err(anyhow::Error::from(e)), response_read))
                        }
                    }
                })
            },
        )))
    }

    async fn ohttp_post<V: DeserializeOwned>(
        &self,
        target_server: &str,
        path: &str,
        headers: Option<HeaderMap>,
        body: Option<Vec<u8>>,
    ) -> Result<Option<V>> {
        let mut bytes_stream = self
            .ohttp_post_stream(target_server, path, headers, body)
            .await?;
        let mut res = Vec::new();
        while let Some(result) = bytes_stream.next().await {
            match result {
                Ok(chunk) => res.extend_from_slice(&chunk),
                Err(e) => return Err(e),
            }
        }

        let api_resp: ApiResponse<V> = serde_json::from_slice(&res)?;
        api_resp.data().map_err(|e| e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_signature_with_raw_p1363_bytes() {
        use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer};
        use p256::elliptic_curve::rand_core::OsRng;

        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        let e = b"client_ephemeral_e";
        let ee = b"server_ephemeral_ee";
        let mut msg = Vec::new();
        msg.extend_from_slice(e);
        msg.extend_from_slice(ee);

        let sig: Signature = signing_key.sign(&msg);
        let raw64: [u8; 64] = sig.to_bytes().into();
        let vk_bytes = verifying_key.to_encoded_point(false);
        let vk_raw = &vk_bytes.as_bytes()[1..65];
        let verifying_key: [u8; 64] = vk_raw.try_into().unwrap();

        verify_noise_script_signature(verifying_key, e, ee, &raw64).unwrap();
    }
}
