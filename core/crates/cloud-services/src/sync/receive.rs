use crate::{CloudServices, Error, KeyManager};

use sd_cloud_schema::{
	devices,
	sync::{
		groups,
		messages::{pull, MessagesCollection},
	},
	Client, Service,
};
use sd_core_sync::{
	cloud_crdt_op_db, CRDTOperation, CompressedCRDTOperationsPerModel, SyncManager,
};

use sd_actors::{Actor, Stopper};
use sd_crypto::{
	cloud::{OneShotDecryption, SecretKey, StreamDecryption},
	primitives::{EncryptedBlock, OneShotNonce, StreamNonce},
};
use sd_prisma::prisma::PrismaClient;

use std::{
	collections::{hash_map::Entry, HashMap},
	future::IntoFuture,
	num::NonZero,
	path::Path,
	pin::Pin,
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
	task::{Context, Poll},
};

use chrono::{DateTime, Utc};
use futures::{FutureExt, StreamExt, TryStreamExt};
use futures_concurrency::future::{Join, Race, TryJoin};
use quic_rpc::transport::quinn::QuinnConnection;
use reqwest::Response;
use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use tokio::{
	fs,
	io::{self, AsyncRead, AsyncReadExt, ReadBuf},
	spawn,
	sync::{Notify, Semaphore},
	time::sleep,
};
use tokio_util::io::StreamReader;
use tracing::{error, instrument};
use uuid::Uuid;

use super::{SyncActors, ONE_MINUTE};

const CLOUD_SYNC_DATA_KEEPER_FILE: &str = "cloud_sync_data_keeper.bin";

/// Responsible for downloading sync operations from the cloud to be processed by the ingester

pub struct Receiver {
	keeper: LastTimestampKeeper,
	sync_group_pub_id: groups::PubId,
	device_pub_id: devices::PubId,
	cloud_services: Arc<CloudServices>,
	cloud_client: Client<QuinnConnection<Service>>,
	semaphore: Arc<Semaphore>,
	key_manager: Arc<KeyManager>,
	sync: SyncManager,
	ingest_notify: Arc<Notify>,
	active: Arc<AtomicBool>,
	active_notify: Arc<Notify>,
}

impl Actor<SyncActors> for Receiver {
	const IDENTIFIER: SyncActors = SyncActors::Receiver;

	async fn run(&mut self, stop: Stopper) {
		enum Race {
			Continue,
			Stop,
		}

		loop {
			self.active.store(true, Ordering::Relaxed);
			self.active_notify.notify_waiters();

			let res = self.run_loop_iteration().await;

			self.active.store(false, Ordering::Relaxed);

			if let Err(e) = res {
				error!(?e, "Error during cloud sync sender actor iteration");
				sleep(ONE_MINUTE).await;
				continue;
			}

			self.active_notify.notify_waiters();

			if matches!(
				(
					sleep(ONE_MINUTE).map(|()| Race::Continue),
					stop.into_future().map(|()| Race::Stop),
				)
					.race()
					.await,
				Race::Stop
			) {
				break;
			}
		}
	}
}

impl Receiver {
	pub async fn new(
		data_dir: impl AsRef<Path> + Send,
		sync_group_pub_id: groups::PubId,
		cloud_services: Arc<CloudServices>,
		sync: SyncManager,
		ingest_notify: Arc<Notify>,
		active: Arc<AtomicBool>,
		active_notify: Arc<Notify>,
	) -> Result<Self, Error> {
		let (keeper, cloud_client, key_manager) = (
			LastTimestampKeeper::load(data_dir.as_ref()),
			cloud_services.client(),
			cloud_services.key_manager(),
		)
			.try_join()
			.await?;

		Ok(Self {
			keeper,
			sync_group_pub_id,
			device_pub_id: devices::PubId(Uuid::from(&sync.device_pub_id)),
			cloud_services,
			cloud_client,
			semaphore: Arc::new(Semaphore::new(
				std::thread::available_parallelism()
					.map(NonZero::get)
					.unwrap_or(1),
			)),
			key_manager,
			sync,
			ingest_notify,
			active,
			active_notify,
		})
	}

	async fn run_loop_iteration(&mut self) -> Result<(), Error> {
		let mut responses_stream = self
			.cloud_client
			.sync()
			.messages()
			.pull(pull::Request {
				access_token: self
					.cloud_services
					.token_refresher
					.get_access_token()
					.await?,
				group_pub_id: self.sync_group_pub_id,
				current_device_pub_id: self.device_pub_id,
				start_time_per_device: self
					.keeper
					.timestamps
					.iter()
					.map(|(device_pub_id, timestamp)| (*device_pub_id, *timestamp))
					.collect(),
			})
			.await?;

		while let Some(new_messages_res) = responses_stream.next().await {
			let pull::Response(new_messages) = new_messages_res??;
			if new_messages.is_empty() {
				break;
			}

			self.handle_new_messages(new_messages).await?;
			self.ingest_notify.notify_waiters();
		}

		self.keeper.save().await
	}

	async fn handle_new_messages(
		&mut self,
		new_messages: Vec<MessagesCollection>,
	) -> Result<(), Error> {
		let handles = new_messages
			.into_iter()
			.map(|message| {
				let sync_group_pub_id = self.sync_group_pub_id;
				let semaphore = Arc::clone(&self.semaphore);
				let key_manager = Arc::clone(&self.key_manager);
				let sync = self.sync.clone();
				let http_client = self.cloud_services.http_client().clone();

				async move {
					spawn(handle_single_message(
						sync_group_pub_id,
						message,
						semaphore,
						key_manager,
						sync,
						http_client,
					))
					.await
				}
			})
			.collect::<Vec<_>>();

		for res in handles.join().await {
			let Ok(res) = res else {
				return Err(Error::SyncMessagesDownloadAndDecryptTaskPanicked);
			};

			let (device_pub_id, timestamp) = res?;

			match self.keeper.timestamps.entry(device_pub_id) {
				Entry::Occupied(mut entry) => {
					if entry.get() < &timestamp {
						*entry.get_mut() = timestamp;
					}
				}
				Entry::Vacant(entry) => {
					entry.insert(timestamp);
				}
			}
		}

		Ok(())
	}
}

async fn handle_single_message(
	sync_group_pub_id: groups::PubId,
	MessagesCollection {
		original_device_pub_id,
		end_time,
		operations_count,
		key_hash,
		signed_download_link,
		..
	}: MessagesCollection,
	semaphore: Arc<Semaphore>,
	key_manager: Arc<KeyManager>,
	sync: SyncManager,
	http_client: ClientWithMiddleware,
) -> Result<(devices::PubId, DateTime<Utc>), Error> {
	// FIXME(@fogodev): If we don't have the key hash, we need to fetch it from another device in the group if possible
	let Some(secret_key) = key_manager.get_key(sync_group_pub_id, &key_hash).await else {
		return Err(Error::MissingKeyHash);
	};

	let _permit = semaphore
		.acquire()
		.await
		.expect("sync messages receiver semaphore never closes");

	let response = http_client
		.get(signed_download_link)
		.send()
		.await
		.map_err(Error::DownloadSyncMessages)?
		.error_for_status()
		.map_err(Error::ErrorResponseDownloadSyncMessages)?;

	let crdt_ops = if let Some(size) = response.content_length() {
		extract_messages_known_size(response, size, secret_key, original_device_pub_id).await
	} else {
		extract_messages_unknown_size(response, secret_key, original_device_pub_id).await
	}?;
	assert_eq!(
		crdt_ops.len(),
		operations_count as usize,
		"Sync messages count mismatch"
	);
	write_cloud_ops_to_db(crdt_ops, &sync.db).await?;
	Ok((original_device_pub_id, end_time))
}

#[instrument(skip(response, secret_key), err)]
async fn extract_messages_known_size(
	response: Response,
	size: u64,
	secret_key: SecretKey,
	devices::PubId(device_pub_id): devices::PubId,
) -> Result<Vec<CRDTOperation>, Error> {
	let plain_text = if size <= EncryptedBlock::CIPHER_TEXT_SIZE as u64 {
		OneShotDecryption::decrypt(
			&secret_key,
			response
				.bytes()
				.await
				.map_err(Error::ErrorResponseDownloadReadBytesSyncMessages)?
				.as_ref()
				.into(),
		)
		.map_err(Error::Decrypt)?
	} else {
		let mut reader = StreamReader::new(response.bytes_stream().map_err(|e| {
			error!(?e, "Failed to read sync messages bytes stream");
			io::Error::new(io::ErrorKind::Other, e)
		}));

		let mut nonce = StreamNonce::default();

		reader
			.read_exact(&mut nonce)
			.await
			.map_err(Error::ReadNonceStreamDecryption)?;

		// TODO: Reimplement using async streaming with serde if it ever gets implemented

		let mut plain_text = vec![];

		StreamDecryption::decrypt(&secret_key, &nonce, reader, &mut plain_text)
			.await
			.map_err(Error::Decrypt)?;

		plain_text
	};

	postcard::from_bytes::<CompressedCRDTOperationsPerModel>(&plain_text)
		.map(|compressed_ops| compressed_ops.into_ops(device_pub_id))
		.map_err(Error::DeserializationFailureToPullSyncMessages)
}

#[instrument(skip_all, err)]
async fn extract_messages_unknown_size(
	response: Response,
	secret_key: SecretKey,
	devices::PubId(device_pub_id): devices::PubId,
) -> Result<Vec<CRDTOperation>, Error> {
	let plain_text = match UnknownDownloadKind::new(response).await? {
		UnknownDownloadKind::OneShot(buffer) => {
			OneShotDecryption::decrypt(&secret_key, buffer.as_slice().into())
				.map_err(Error::Decrypt)?
		}

		UnknownDownloadKind::Stream((nonce, reader)) => {
			let mut plain_text = vec![];

			StreamDecryption::decrypt(&secret_key, &nonce, reader, &mut plain_text)
				.await
				.map_err(Error::Decrypt)?;

			plain_text
		}
	};

	postcard::from_bytes::<CompressedCRDTOperationsPerModel>(&plain_text)
		.map(|compressed_ops| compressed_ops.into_ops(device_pub_id))
		.map_err(Error::DeserializationFailureToPullSyncMessages)
}

#[instrument(skip_all, err)]
pub async fn write_cloud_ops_to_db(
	ops: Vec<CRDTOperation>,
	db: &PrismaClient,
) -> Result<(), sd_core_sync::Error> {
	db._batch(
		ops.into_iter()
			.map(|op| cloud_crdt_op_db(&op).map(|op| op.to_query(db)))
			.collect::<Result<Vec<_>, _>>()?,
	)
	.await?;

	Ok(())
}

#[derive(Serialize, Deserialize, Debug)]
struct LastTimestampKeeper {
	timestamps: HashMap<devices::PubId, DateTime<Utc>>,
	file_path: Box<Path>,
}

impl LastTimestampKeeper {
	async fn load(data_dir: &Path) -> Result<Self, Error> {
		let file_path = data_dir.join(CLOUD_SYNC_DATA_KEEPER_FILE).into_boxed_path();

		match fs::read(&file_path).await {
			Ok(bytes) => Ok(Self {
				timestamps: postcard::from_bytes(&bytes)
					.map_err(Error::LastTimestampKeeperSerialization)?,
				file_path,
			}),

			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self {
				timestamps: HashMap::new(),
				file_path,
			}),

			Err(e) => Err(Error::FailedToReadLastTimestampKeeper(e)),
		}
	}

	async fn save(&self) -> Result<(), Error> {
		fs::write(
			&self.file_path,
			&postcard::to_stdvec(&self.timestamps)
				.map_err(Error::LastTimestampKeeperSerialization)?,
		)
		.await
		.map_err(Error::FailedToWriteLastTimestampKeeper)
	}
}

struct UnknownDownloadSizeStreamer {
	stream_reader: Box<dyn AsyncRead + Send + Unpin + 'static>,
	buffer: Vec<u8>,
	was_read: usize,
}

enum UnknownDownloadKind {
	OneShot(Vec<u8>),
	Stream((StreamNonce, UnknownDownloadSizeStreamer)),
}

impl UnknownDownloadKind {
	async fn new(response: Response) -> Result<Self, Error> {
		let mut buffer = Vec::with_capacity(EncryptedBlock::CIPHER_TEXT_SIZE * 2);

		let mut stream = response.bytes_stream();

		while let Some(res) = stream.next().await {
			buffer.extend(res.map_err(Error::ErrorResponseDownloadReadBytesSyncMessages)?);
			if buffer.len() > EncryptedBlock::CIPHER_TEXT_SIZE {
				break;
			}
		}

		if buffer.len() < size_of::<OneShotNonce>() {
			return Err(Error::IncompleteDownloadBytesSyncMessages);
		}

		if buffer.len() <= EncryptedBlock::CIPHER_TEXT_SIZE {
			Ok(Self::OneShot(buffer))
		} else {
			let nonce_size = size_of::<StreamNonce>();

			Ok(Self::Stream((
				StreamNonce::try_from(&buffer[..nonce_size]).expect("passing the right nonce size"),
				UnknownDownloadSizeStreamer {
					stream_reader: Box::new(StreamReader::new(stream.map_err(|e| {
						error!(?e, "Failed to read sync messages bytes stream");
						io::Error::new(io::ErrorKind::Other, e)
					}))),
					buffer,
					was_read: nonce_size,
				},
			)))
		}
	}
}

impl AsyncRead for UnknownDownloadSizeStreamer {
	fn poll_read(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
		buf: &mut ReadBuf<'_>,
	) -> Poll<io::Result<()>> {
		if buf.remaining() == 0 {
			return Poll::Ready(Ok(()));
		}

		if self.was_read == self.buffer.len() {
			Pin::new(&mut self.stream_reader).poll_read(cx, buf)
		} else {
			let len = std::cmp::min(self.buffer.len() - self.was_read, buf.remaining());
			buf.put_slice(&self.buffer[self.was_read..(self.was_read + len)]);
			self.was_read += len;

			Poll::Ready(Ok(()))
		}
	}
}