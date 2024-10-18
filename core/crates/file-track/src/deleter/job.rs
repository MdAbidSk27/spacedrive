use std::{
	hash::Hash,
	marker::PhantomData,
	path::{Path, PathBuf},
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
};

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use futures_concurrency::stream::Merge;
use itertools::Itertools;
use sd_core_file_path_helper::IsolatedFilePathData;
use sd_core_heavy_lifting::{
	job_system::{
		job::{Job, JobReturn, JobTaskDispatcher, ReturnStatus},
		utils::cancel_pending_tasks,
		SerializableJob, SerializedTasks,
	},
	Error, JobContext, JobName, NonCriticalError, OuterContext, ProgressUpdate,
};
use sd_core_prisma_helpers::file_path_with_object;
use sd_prisma::prisma::{file_path, location, PrismaClient};
use sd_task_system::{Task, TaskDispatcher, TaskHandle, TaskOutput, TaskStatus};

use super::{tasks, DeleteBehavior, FileData};

#[derive(Debug)]
pub struct DeleterJob<T> {
	location_id: location::id::Type,
	file_path_ids: Vec<file_path::id::Type>,

	pending_tasks: Option<Vec<TaskHandle<Error>>>,
	shutdown_tasks: Option<Vec<Box<dyn Task<Error>>>>,
	accumulative_errors: Option<Vec<Error>>,

	behavior: PhantomData<fn(T) -> T>, // variance: invariant, inherent Send + Sync
}

impl<B: DeleteBehavior> Hash for DeleterJob<tasks::RemoveTask<B>> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.location_id.hash(state);
		self.file_path_ids.hash(state);
		// self.tasks.hash(state);
		// self.behavior.hash(state);
	}
}

impl<B: DeleteBehavior + Hash> DeleterJob<tasks::RemoveTask<B>> {
	pub const fn new(
		location_id: location::id::Type,
		file_path_ids: Vec<file_path::id::Type>,
	) -> Self {
		Self {
			location_id,
			file_path_ids,

			behavior: PhantomData,

			accumulative_errors: None,
			pending_tasks: None,
			shutdown_tasks: None,
		}
	}
}

impl<B: DeleteBehavior + Hash + Send + 'static> Job for DeleterJob<tasks::RemoveTask<B>> {
	const NAME: JobName = JobName::Delete;

	// TODO(matheus-consoli): tracing
	async fn run<OuterCtx: OuterContext>(
		mut self,
		dispatcher: JobTaskDispatcher,
		ctx: impl JobContext<OuterCtx>,
	) -> Result<ReturnStatus, Error> {
		let location_path = get_location_path_from_location_id(ctx.db(), self.location_id)
			.await
			.map_err(|_e| todo!("?, aka error handling"))
			.unwrap();

		// TODO(matheus-consoli): use a better query (get only the info we need)
		let files = get_many_files_datas(ctx.db(), location_path, &self.file_path_ids)
			.await
			.map_err(|_| todo!("FileSystemJobsError::from"))
			.unwrap();

		ctx.progress([ProgressUpdate::CompletedTaskCount(files.len() as _)])
			.await;

		let mut steps: Vec<Vec<_>> = {
			let temp = files.into_iter();
			let ch = temp.chunks(50);

			ch.into_iter().map(|c| c.collect()).collect()
		};

		let progress_counter = Arc::new(AtomicU64::new(0));

		// TODO(matheus-consoli): make it clear that this is an optimization
		// exec_in_place();
		if steps.len() == 1 {
			tracing::debug!("files to delete fits in a single task, straight up executing it");

			let all = steps.pop().expect("we checked the length");
			let size = all.len() as u64;

			B::delete_all(all).await.unwrap();

			ctx.progress([ProgressUpdate::TaskCount(size)]).await;
			progress_counter.fetch_add(size, Ordering::SeqCst);
		} else {
			let tasks =
				dispatcher
					.dispatch_many(steps.into_iter().map(|step| {
						tasks::RemoveTask::<B>::new(step, Arc::clone(&progress_counter))
					}))
					.await
					.unwrap();

			let mut tasks = FuturesUnordered::from_iter(tasks);

			let c = while let Some(result) = tasks.next().await {
				match result {
					Ok(TaskStatus::Done(_)) => {
						let progress = progress_counter.load(Ordering::Acquire);
						ctx.progress([ProgressUpdate::TaskCount(progress)]).await;
					}
					Ok(TaskStatus::Shutdown(task)) => {
						self.shutdown_tasks.get_or_insert_with(Vec::new).push(task);
					}
					Ok(TaskStatus::Canceled | TaskStatus::ForcedAbortion) => {
						cancel_pending_tasks(&mut tasks).await;
						let _return = ReturnStatus::Canceled(
							JobReturn::builder()
								// .with_non_critical_errors()
								.build(),
						);
						// TODO(matheus-consoli): cancel the job
						todo!()
					}
					Ok(TaskStatus::Error(error)) => {
						cancel_pending_tasks(&mut tasks).await;
						// break Some(Err(error));
					}

					Err(_) => {
						cancel_pending_tasks(&mut tasks).await;
						// break Some(Err(error));
					}
				}
			};
		};

		// TODO(matheus-consoli): inline this later
		// let errors = errors
		// 	.into_iter()
		// 	.map(|_| NonCriticalError::Deleter("TODO handle errors".into()))
		// 	.collect();

		ctx.progress([ProgressUpdate::CompletedTaskCount(
			progress_counter.load(Ordering::Acquire),
		)])
		.await;

		Ok(ReturnStatus::Completed(
			JobReturn::builder()
				// .with_non_critical_errors(errors)
				.build(),
		))
	}
}

// TODO(matheus-consoli): add serialization once we add smart tasks
impl<OuterCtx: OuterContext, B: DeleteBehavior + Hash + 'static> SerializableJob<OuterCtx>
	for DeleterJob<tasks::RemoveTask<B>>
{
	async fn serialize(self) -> Result<Option<Vec<u8>>, rmp_serde::encode::Error> {
		Ok(None)
	}

	async fn deserialize(
		serialized_job: &[u8],
		ctx: &OuterCtx,
	) -> Result<Option<(Self, Option<SerializedTasks>)>, rmp_serde::decode::Error> {
		Ok(None)
	}
}

type TODO = Box<dyn std::error::Error>;

/// Get the [`FileData`] related to every `file_path_id`
async fn get_many_files_datas(
	db: &PrismaClient,
	location_path: impl AsRef<Path>,
	file_path_ids: &[file_path::id::Type],
) -> Result<Vec<FileData>, TODO> {
	let location_path = location_path.as_ref();

	db._batch(
		file_path_ids
			.iter()
			.map(|file_path_id| {
				db.file_path()
					.find_unique(file_path::id::equals(*file_path_id))
					.include(file_path_with_object::include())
			})
			// FIXME:(fogodev -> Brendonovich) this collect is a workaround to a weird higher ranker lifetime error on
			// the _batch function, it should be removed once the error is fixed
			.collect::<Vec<_>>(),
	)
	.await?
	.into_iter()
	.zip(file_path_ids.iter())
	.map(|(maybe_file_path, file_path_id)| {
		maybe_file_path
			// TODO(matheus-consoli): proper error handling
			.ok_or_else(|| todo!())
			// .ok_or(FileSystemJobsError::FilePathIdNotFound(*file_path_id))
			.and_then(|path_data| {
				Ok(FileData {
					full_path: location_path.join(IsolatedFilePathData::try_from(&path_data)?),
					file_path: path_data,
				})
			})
	})
	.collect()
}

pub async fn get_location_path_from_location_id(
	db: &PrismaClient,
	location_id: file_path::id::Type,
) -> Result<PathBuf, TODO> {
	db.location()
		.find_unique(location::id::equals(location_id))
		.exec()
		.await
		.map_err(Into::into)
		.and_then(|maybe_location| {
			maybe_location
				// TODO(matheus-consoli): proper error handling
				.ok_or_else(|| todo!())
				// .ok_or(LocationError::IdNotFound(location_id))
				.and_then(|location| {
					location
						.path
						.map(PathBuf::from)
						// TODO(matheus-consoli): proper error handling
						.ok_or_else(|| todo!())
					// .ok_or(LocationError::MissingPath(location_id))
				})
		})
}