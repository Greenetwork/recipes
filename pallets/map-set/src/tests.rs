use crate::*;
use frame_support::{assert_noop, assert_ok, impl_outer_event, impl_outer_origin, parameter_types};
use frame_system as system;
use sp_core::H256;
use sp_io::TestExternalities;
use sp_runtime::{
	testing::Header,
	traits::{BlakeTwo256, IdentityLookup},
	Perbill,
};

impl_outer_origin! {
	pub enum Origin for TestRuntime {}
}

// Workaround for https://github.com/rust-lang/rust/issues/26925 . Remove when sorted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TestRuntime;
parameter_types! {
	pub const BlockHashCount: u64 = 250;
	pub const MaximumBlockWeight: u32 = 1024;
	pub const MaximumBlockLength: u32 = 2 * 1024;
	pub const AvailableBlockRatio: Perbill = Perbill::one();
}
impl system::Trait for TestRuntime {
	type Origin = Origin;
	type Index = u64;
	type Call = ();
	type BlockNumber = u64;
	type Hash = H256;
	type Hashing = BlakeTwo256;
	type AccountId = u64;
	type Lookup = IdentityLookup<Self::AccountId>;
	type Header = Header;
	type Event = TestEvent;
	type BlockHashCount = BlockHashCount;
	type MaximumBlockWeight = MaximumBlockWeight;
	type DbWeight = ();
	type BlockExecutionWeight = ();
	type ExtrinsicBaseWeight = ();
	type MaximumExtrinsicWeight = MaximumBlockWeight;
	type MaximumBlockLength = MaximumBlockLength;
	type AvailableBlockRatio = AvailableBlockRatio;
	type Version = ();
	type ModuleToIndex = ();
	type AccountData = ();
	type OnNewAccount = ();
	type OnKilledAccount = ();
}

mod vec_set {
	pub use crate::Event;
}

impl_outer_event! {
	pub enum TestEvent for TestRuntime {
		vec_set<T>,
		system<T>,
	}
}

impl Trait for TestRuntime {
	type Event = TestEvent;
}

pub type System = system::Module<TestRuntime>;
pub type MapSet = Module<TestRuntime>;

pub struct ExtBuilder;

impl ExtBuilder {
	pub fn build() -> TestExternalities {
		let storage = system::GenesisConfig::default()
			.build_storage::<TestRuntime>()
			.unwrap();
		let mut ext = TestExternalities::from(storage);
		ext.execute_with(|| System::set_block_number(1));
		ext
	}
}

#[test]
fn add_member_works() {
	ExtBuilder::build().execute_with(|| {
		assert_ok!(MapSet::add_member(Origin::signed(1)));

		let expected_event = TestEvent::vec_set(RawEvent::MemberAdded(1));
		assert!(System::events().iter().any(|a| a.event == expected_event));

		assert!(<Members<TestRuntime>>::contains_key(1));
	})
}

#[test]
fn cant_add_duplicate_members() {
	ExtBuilder::build().execute_with(|| {
		assert_ok!(MapSet::add_member(Origin::signed(1)));

		assert_noop!(
			MapSet::add_member(Origin::signed(1)),
			Error::<TestRuntime>::AlreadyMember
		);
	})
}

#[test]
fn cant_exceed_max_members() {
	ExtBuilder::build().execute_with(|| {
		// Add 16 members, reaching the max
		for i in 0..16 {
			assert_ok!(MapSet::add_member(Origin::signed(i)));
		}

		// Try to add the 17th member exceeding the max
		assert_noop!(
			MapSet::add_member(Origin::signed(16)),
			Error::<TestRuntime>::MembershipLimitReached
		);
	})
}

#[test]
fn remove_member_works() {
	ExtBuilder::build().execute_with(|| {
		assert_ok!(MapSet::add_member(Origin::signed(1)));
		assert_ok!(MapSet::remove_member(Origin::signed(1)));

		// check correct event emission
		let expected_event = TestEvent::vec_set(RawEvent::MemberRemoved(1));
		assert!(System::events().iter().any(|a| a.event == expected_event));

		// check storage changes
		assert!(!<Members<TestRuntime>>::contains_key(1));
	})
}

#[test]
fn remove_member_handles_errors() {
	ExtBuilder::build().execute_with(|| {
		// 2 is NOT previously added as a member
		assert_noop!(
			MapSet::remove_member(Origin::signed(2)),
			Error::<TestRuntime>::NotMember
		);
	})
}
