//! A demonstration of an offchain worker that sends onchain callbacks

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod tests;

use core::{convert::TryInto, fmt};
use frame_support::{
	debug, decl_error, decl_event, decl_module, decl_storage, dispatch::DispatchResult, traits::Get,
};
use parity_scale_codec::{Decode, Encode};

use frame_system::{
	self as system, ensure_none, ensure_signed,
	offchain::{
		AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer, SubmitTransaction,
	},
};
use sp_core::crypto::KeyTypeId;
use sp_runtime::{
	offchain as rt_offchain,
	offchain::storage::StorageValueRef,
	transaction_validity::{
		InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity,
		ValidTransaction,
	},
};
use sp_std::prelude::*;
use sp_std::str;

// We use `alt_serde`, and Xanewok-modified `serde_json` so that we can compile the program
//   with serde(features `std`) and alt_serde(features `no_std`).
use alt_serde::{Deserialize, Deserializer};

/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"demo");
pub const NUM_VEC_LEN: usize = 10;

// We are fetching information from github public API about organisation `substrate-developer-hub`.
pub const HTTP_REMOTE_REQUEST_BYTES: &[u8] = b"https://api.github.com/orgs/substrate-developer-hub";
pub const HTTP_HEADER_USER_AGENT: &[u8] = b"jimmychu0807";

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
pub mod crypto {
	use crate::KEY_TYPE;
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner,
	};

	app_crypto!(sr25519, KEY_TYPE);

	pub struct TestAuthId;
	// implemented for ocw-runtime
	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}

	// implemented for mock runtime in test
	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
		for TestAuthId
	{
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}

// How to implement ocw executed by extrinsic instead of arbitraty block number
// i would implement a task queue as on-chain storage, storing any needed parameters the ocw needed inside. 
// Then when the needed extrinsic is called, it adds a new object (with params/info the ocw needed) in the taskqueue. 
//  need to implement a boolean storage value to track if task queue has object or not. 
// Then everytime in the ocw callback, just check if the task queue has any object. If yes, process it. If no, return.

// TaskQueue, needs an extrinsic used to populate these fields
#[serde(crate = "alt_serde")]
#[derive(Deserialize, Encode, Decode, Default,Debug)]
pub struct TaskQueue {
	#[serde(deserialize_with = "de_string_to_bytes")]
	http_remote_reqst: Vec<u8>,
	#[serde(deserialize_with = "de_string_to_bytes")]
	http_header_usr: Vec<u8>,
}

// Specifying serde path as `alt_serde`
// ref: https://serde.rs/container-attrs.html#crate
#[serde(crate = "alt_serde")]
#[derive(Deserialize, Encode, Decode, Default)]
struct GithubInfo {
	// Specify our own deserializing function to convert JSON string to vector of bytes
	#[serde(deserialize_with = "de_string_to_bytes")]
	login: Vec<u8>,
	#[serde(deserialize_with = "de_string_to_bytes")]
	blog: Vec<u8>,
	public_repos: u32,
}

pub fn de_string_to_bytes<'de, D>(de: D) -> Result<Vec<u8>, D::Error>
where
	D: Deserializer<'de>,
{
	let s: &str = Deserialize::deserialize(de)?;
	Ok(s.as_bytes().to_vec())
}

impl fmt::Debug for GithubInfo {
	// `fmt` converts the vector of bytes inside the struct back to string for
	//   more friendly display.
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{{ login: {}, blog: {}, public_repos: {} }}",
			str::from_utf8(&self.login).map_err(|_| fmt::Error)?,
			str::from_utf8(&self.blog).map_err(|_| fmt::Error)?,
			&self.public_repos
		)
	}
}

/// This is the pallet's configuration trait
pub trait Trait: system::Trait + CreateSignedTransaction<Call<Self>> {
	/// The identifier type for an offchain worker.
	type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
	/// The overarching dispatch call type.
	type Call: From<Call<Self>>;
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
	/// The type to sign and send transactions.
	type UnsignedPriority: Get<TransactionPriority>;
}

// Custom data type
#[derive(Debug)]
enum TransactionType {
	SignedSubmitNumber,
	UnsignedSubmitNumber,
	//HttpFetching,
	None,
}

decl_storage! {
	trait Store for Module<T: Trait> as Example {
		/// A vector of recently submitted numbers. Should be bounded
		Numbers get(fn numbers): Vec<u64>;
		/// A map of TasksQueues to numbers
		TaskQueueByNumber get(fn task_queue_by_number):
			map hasher(blake2_128_concat) u32 => TaskQueue;
		// A bool to track if there is a task in the queue to be fetched via HTTP
		QueueAvailable get(fn queue_available): bool;
		// Another bool to track if there is some data in the offchain worker ready to be submitted onchain
		//DataAvailable get (fn data_available): bool;
		UserAgentOnChain get(fn user_agent_on_chain): Vec<u8>;
	}
}

decl_event!(
	/// Events generated by the module.
	pub enum Event<T>
	where
		AccountId = <T as system::Trait>::AccountId,
	{
		/// Event generated when a new number is accepted to contribute to the average.
		NewNumber(Option<AccountId>, u64),
	}
);

decl_error! {
	pub enum Error for Module<T: Trait> {
		// Error returned when making signed transactions in off-chain worker
		SignedSubmitNumberError,
		// Error returned when making unsigned transactions in off-chain worker
		UnsignedSubmitNumberError,
		// Error returned when making remote http fetching
		HttpFetchingError0,
		HttpFetchingError1,
		HttpFetchingError2,
		HttpFetchingError3,
		HttpFetchingError4,
		HttpFetchingError5,
		HttpFetchingError6,
		HttpFetchingError7,
		HttpFetchingError8,
		HttpFetchingError9,
		// Error returned when gh-info has already been fetched
		AlreadyFetched,
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		/// Adds a new task to the TaskQueue
		#[weight = 0]
		pub fn insert_new_task(origin, task_number: u32, http_remote_reqst: Vec<u8>, http_header_usr: Vec<u8>) -> DispatchResult {
			let _ = ensure_signed(origin)?;
			let task_queue = TaskQueue {
				http_remote_reqst,
				http_header_usr,
			};
			<TaskQueueByNumber>::insert(task_number, task_queue);
			QueueAvailable::put(true);
			Ok(())
		}

		#[weight = 0]
		pub fn empty_tasks(origin) -> DispatchResult {
			QueueAvailable::put(false);
			Ok(())
		}

		#[weight = 0]
		pub fn submit_agent_signed(origin, agent: Vec<u8>) -> DispatchResult {
			debug::info!("submit_agent_signed: {:?}", agent);
			let who = ensure_signed(origin)?;
			Self::update_agent(Some(who), agent)
		}

		#[weight = 0]
		pub fn submit_number_signed(origin, number: u64) -> DispatchResult {
			debug::info!("submit_number_signed: {:?}", number);
			let who = ensure_signed(origin)?;
			Self::append_or_replace_number(Some(who), number)
		}

		#[weight = 0]
		pub fn submit_number_unsigned(origin, number: u64) -> DispatchResult {
			debug::info!("submit_number_unsigned: {:?}", number);
			let _ = ensure_none(origin)?;
			Self::append_or_replace_number(None, number)
		}

		fn offchain_worker(block_number: T::BlockNumber) {
			debug::info!("Entering off-chain workers");

//			let result = match Self::choose_tx_type(block_number) {
//				TransactionType::SignedSubmitNumber => Self::signed_submit_number(block_number),
//				TransactionType::UnsignedSubmitNumber => Self::unsigned_submit_number(block_number),
//				TransactionType::HttpFetching => Self::fetch_if_needed(),
//				TransactionType::None => Ok(())
//			};

			let result = 
				if Self::queue_available() == true {
					debug::info!("there is a task in the queue");
					QueueAvailable::put(false);
					debug::info!("the task status is {:?}", Self::queue_available());
					Self::fetch_if_needed()
				//DataAvailable::put(true);
				} else {
					debug::info!("executing signed extrinsic");
					Self::signed_submit_agent()
					//if let Err(e) = result { debug::error!("Error: {:?}", e); }
			};
		}
	}
}

impl<T: Trait> Module<T> {
	/// Add a new number to the list.
	fn append_or_replace_number(who: Option<T::AccountId>, number: u64) -> DispatchResult {
		Numbers::mutate(|numbers| {
			// The append or replace logic. The `numbers` vector is at most `NUM_VEC_LEN` long.
			let num_len = numbers.len();

			if num_len < NUM_VEC_LEN {
				numbers.push(number);
			} else {
				numbers[num_len % NUM_VEC_LEN] = number;
			}

			// displaying the average
			let num_len = numbers.len();
			let average = match num_len {
				0 => 0,
				_ => numbers.iter().sum::<u64>() / (num_len as u64),
			};

			debug::info!("Current average of numbers is: {}", average);
		});

		// Raise the NewNumber event
		Self::deposit_event(RawEvent::NewNumber(who, number));
		Ok(())
	}

	fn update_agent(who: Option<T::AccountId>, agent: Vec<u8>) -> DispatchResult {
		debug::info!("some agent ---> {:?}",agent);
		UserAgentOnChain::put(agent);
		Ok(())
	}

	fn choose_tx_type(block_number: T::BlockNumber) -> TransactionType {

		//let task_queue_thing = Self::task_queue_by_number(1);
		//let task_queue_bytes = task_queue_thing.http_header_usr;

		//let task_alias2 = str::from_utf8(&task_queue_bytes).map_err(|_| <Error<T>>::HttpFetchingError3);
		//debug::info!("WHY?{:?}ABC",task_alias2);

		//let task_alias = StorageValueRef::persistent(b"offchain-demo::task-item");
		//let task_alias3 = task_alias.get::<TaskQueue>();
		//debug::info!("compare this --> {:?}",task_alias3);


		//if let Some(Some(task_queue_thing)) = task_alias.get::<TaskQueue>() {
			// task has already been completed, Return None as transaction type
		//	debug::info!("cached task-info: {:?}", task_queue_thing.http_header_usr);
		//	return TransactionType::None;
		//}

		// Decide what type of transaction to send based on block number.
		// Each block the offchain worker will send one type of transaction back to the chain.
		// First a signed transaction, then an unsigned transaction, then an http fetch and json parsing.
		match block_number.try_into().ok().unwrap() % 3 {
			0 => TransactionType::SignedSubmitNumber,
			1 => TransactionType::UnsignedSubmitNumber,
			//2 => TransactionType::HttpFetching,
			_ => TransactionType::None,
		}
	}

	/// Check if we have fetched github info before. If yes, we use the cached version that is
	///   stored in off-chain worker storage `storage`. If no, we fetch the remote info and then
	///   write the info into the storage for future retrieval.
	fn fetch_if_needed() -> Result<(), Error<T>> {
		// Start off by creating a reference to Local Storage value.
		// Since the local storage is common for all offchain workers, it's a good practice
		// to prepend our entry with the pallet name.
		let s_info = StorageValueRef::persistent(b"offchain-demo::gh-info");
		let s_lock = StorageValueRef::persistent(b"offchain-demo::lock");

		// The local storage is persisted and shared between runs of the offchain workers,
		// and offchain workers may run concurrently. We can use the `mutate` function, to
		// write a storage entry in an atomic fashion.
		//
		// It has a similar API as `StorageValue` that offer `get`, `set`, `mutate`.
		// If we are using a get-check-set access pattern, we likely want to use `mutate` to access
		// the storage in one go.
		//
		// Ref: https://substrate.dev/rustdocs/v2.0.0-rc3/sp_runtime/offchain/storage/struct.StorageValueRef.html
		if let Some(Some(gh_info)) = s_info.get::<GithubInfo>() {
			// gh-info has already been fetched. Return early.
			debug::info!("cached gh-info: {:?}", gh_info);
			return Ok(());
		}

		// We are implementing a mutex lock here with `s_lock`
		let res: Result<Result<bool, bool>, Error<T>> = s_lock.mutate(|s: Option<Option<bool>>| {
			match s {
				// `s` can be one of the following:
				//   `None`: the lock has never been set. Treated as the lock is free
				//   `Some(None)`: unexpected case, treated it as AlreadyFetch
				//   `Some(Some(false))`: the lock is free
				//   `Some(Some(true))`: the lock is held

				// If the lock has never been set or is free (false), return true to execute `fetch_n_parse`
				None | Some(Some(false)) => Ok(true),

				// Otherwise, someone already hold the lock (true), we want to skip `fetch_n_parse`.
				// Covering cases: `Some(None)` and `Some(Some(true))`
				_ => Err(<Error<T>>::AlreadyFetched),
			}
		});

		// Cases of `res` returned result:
		//   `Err(<Error<T>>)` - lock is held, so we want to skip `fetch_n_parse` function.
		//   `Ok(Err(true))` - Another ocw is writing to the storage while we set it,
		//                     we also skip `fetch_n_parse` in this case.
		//   `Ok(Ok(true))` - successfully acquire the lock, so we run `fetch_n_parse`
		if let Ok(Ok(true)) = res {
			match Self::fetch_n_parse() {
				Ok(gh_info) => {
					// set gh-info into the storage and release the lock
					s_info.set(&gh_info);
					s_lock.set(&false);

					debug::info!("fetched gh-info: {:?}", gh_info);
				}
				Err(err) => {
					// release the lock
					s_lock.set(&false);
					return Err(err);
				}
			}
		}
		Ok(())
	}

	/// Fetch from remote and deserialize the JSON to a struct
	fn fetch_n_parse() -> Result<GithubInfo, Error<T>> {
		let resp_bytes = Self::fetch_from_remote().map_err(|e| {
			debug::error!("fetch_from_remote error: {:?}", e);
			<Error<T>>::HttpFetchingError0
		})?;

		let resp_str = str::from_utf8(&resp_bytes).map_err(|_| <Error<T>>::HttpFetchingError1)?;
		// Print out our fetched JSON string
		debug::info!("{}", resp_str);

		// Deserializing JSON to struct, thanks to `serde` and `serde_derive`
		let gh_info: GithubInfo =
			serde_json::from_str(&resp_str).map_err(|_| <Error<T>>::HttpFetchingError2)?;
		Ok(gh_info)
	}

	/// This function uses the `offchain::http` API to query the remote github information,
	///   and returns the JSON response as vector of bytes.
	fn fetch_from_remote() -> Result<Vec<u8>, Error<T>> {
		let remote_url_bytes = HTTP_REMOTE_REQUEST_BYTES.to_vec();
		//let user_agent = HTTP_HEADER_USER_AGENT.to_vec();
		let task_queue_thing = Self::task_queue_by_number(1);
		let user_agent_bytes = task_queue_thing.http_header_usr;
		let user_agent = str::from_utf8(&user_agent_bytes).map_err(|_| <Error<T>>::HttpFetchingError3)?;
		debug::info!("from the task queue --> {}", user_agent);

		let remote_url =
			str::from_utf8(&remote_url_bytes).map_err(|_| <Error<T>>::HttpFetchingError4)?;

		debug::info!("sending request to: {}", remote_url);

		// Initiate an external HTTP GET request. This is using high-level wrappers from `sp_runtime`.
		let request = rt_offchain::http::Request::get(remote_url);

		// Keeping the offchain worker execution time reasonable, so limiting the call to be within 3s.
		let timeout = sp_io::offchain::timestamp().add(rt_offchain::Duration::from_millis(3000));

		// For github API request, we also need to specify `user-agent` in http request header.
		//   See: https://developer.github.com/v3/#user-agent-required
		let pending = request
			.add_header(
				"User-Agent",
				str::from_utf8(&user_agent_bytes).map_err(|_| <Error<T>>::HttpFetchingError5)?,
			)
			.deadline(timeout) // Setting the timeout time
			.send() // Sending the request out by the host
			.map_err(|_| <Error<T>>::HttpFetchingError6)?;

		// By default, the http request is async from the runtime perspective. So we are asking the
		//   runtime to wait here.
		// The returning value here is a `Result` of `Result`, so we are unwrapping it twice by two `?`
		//   ref: https://substrate.dev/rustdocs/v2.0.0-rc3/sp_runtime/offchain/http/struct.PendingRequest.html#method.try_wait
		let response = pending
			.try_wait(timeout)
			.map_err(|_| <Error<T>>::HttpFetchingError7)?
			.map_err(|_| <Error<T>>::HttpFetchingError8)?;

		if response.code != 200 {
			debug::error!("Unexpected http request status code: {}", response.code);
			return Err(<Error<T>>::HttpFetchingError9);
		}

		// Next we fully read the response body and collect it to a vector of bytes.
		Ok(response.body().collect::<Vec<u8>>())
	}

	fn signed_submit_agent() -> Result<(), Error<T>> {
		let signer = Signer::<T, T::AuthorityId>::all_accounts();
		if !signer.can_sign() {
			debug::error!("No local account available -- boi"); // HELP HERE
			return Err(<Error<T>>::SignedSubmitNumberError);
		}
		let s_info = StorageValueRef::persistent(b"offchain-demo::gh-info");
		if let Some(Some(gh_info)) = s_info.get::<GithubInfo>() {
			debug::info!("cached gh-info in submit function: {:?}", gh_info);
			let agent_y = gh_info.login;
			let results = signer.send_signed_transaction(|_acct| {
				Call::submit_agent_signed(agent_y.clone())
			});
			for (acc, res) in &results {
				match res {
					Ok(()) => {
						debug::native::info!(
							"off-chain send_signed: acc: {:?}| number: {:#?}",
							acc.id,
							agent_y.clone()
						);
					}
					Err(e) => {
						debug::error!("[{:?}] Failed in signed_submit_number: {:?}", acc.id, e);
						return Err(<Error<T>>::SignedSubmitNumberError);
					}
				};
			}
		};

		Ok(())
	}



	fn signed_submit_number(block_number: T::BlockNumber) -> Result<(), Error<T>> {
		let signer = Signer::<T, T::AuthorityId>::all_accounts();
		if !signer.can_sign() {
			debug::error!("No local account available"); // HELP HERE
			return Err(<Error<T>>::SignedSubmitNumberError);
		}

		// Using `SubmitSignedTransaction` associated type we create and submit a transaction
		// representing the call, we've just created.
		// Submit signed will return a vector of results for all accounts that were found in the
		// local keystore with expected `KEY_TYPE`.
		let submission: u64 = block_number.try_into().ok().unwrap() as u64;
		
		let s_info = StorageValueRef::persistent(b"offchain-demo::gh-info");
		if let Some(Some(gh_info)) = s_info.get::<GithubInfo>() {
			debug::info!("cached gh-info in submit function: {:?}", gh_info);
		};
		let test_info = s_info.get::<GithubInfo>();
		//let agent_x = test_info.login; HELP HERE
		//let submission2: Vec<u8> = agent_x; HELP HERE
		
		
		let results = signer.send_signed_transaction(|_acct| {
			// We are just submitting the current block number back on-chain
			Call::submit_number_signed(submission)
			//Call::submit_agent_signed(submission2.clone()) HELP HERE
		});

		for (acc, res) in &results {
			match res {
				Ok(()) => {
					debug::native::info!(
						"off-chain send_signed: acc: {:?}| number: {}",
						acc.id,
						submission
					);
				}
				Err(e) => {
					debug::error!("[{:?}] Failed in signed_submit_number: {:?}", acc.id, e);
					return Err(<Error<T>>::SignedSubmitNumberError);
				}
			};
		}
		Ok(())
	}

	fn unsigned_submit_number(block_number: T::BlockNumber) -> Result<(), Error<T>> {
		let submission: u64 = block_number.try_into().ok().unwrap() as u64;
		// Submitting the current block number back on-chain.
		let call = Call::submit_number_unsigned(submission);

		SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into()).map_err(|e| {
			debug::error!("Failed in unsigned_submit_number: {:?}", e);
			<Error<T>>::UnsignedSubmitNumberError
		})
	}
}

impl<T: Trait> frame_support::unsigned::ValidateUnsigned for Module<T> {
	type Call = Call<T>;

	fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
		#[allow(unused_variables)]
		if let Call::submit_number_unsigned(number) = call {
			debug::native::info!("off-chain send_unsigned: number: {}", number);

			ValidTransaction::with_tag_prefix("offchain-demo")
				.priority(T::UnsignedPriority::get())
				.and_provides([b"submit_number_unsigned"])
				.longevity(3)
				.propagate(true)
				.build()
		} else {
			InvalidTransaction::Call.into()
		}
	}
}
