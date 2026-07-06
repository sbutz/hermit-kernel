use alloc::boxed::Box;
use core::arch::asm;
use core::cell::Cell;
use core::ptr;
use core::sync::atomic::Ordering;

use async_executor::StaticLocalExecutor;
use enumset::{EnumSet, EnumSetType};
#[cfg(feature = "smp")]
use hermit_sync::InterruptTicketMutex;
use hermit_sync::{RawRwSpinLock, RawSpinMutex};

use crate::arch::riscv64::kernel::CPU_ONLINE;
use crate::env;
#[cfg(feature = "smp")]
use crate::scheduler::SchedulerInput;
use crate::scheduler::{CoreId, PerCoreScheduler};

#[derive(Debug, EnumSetType)]
pub enum IsaExtension {
	/// Zkr - Entropy Source Extension
	Zkr,
}

pub struct CoreLocal {
	/// ID of the current Core.
	core_id: CoreId,
	/// Scheduler of the current Core.
	scheduler: Cell<*mut PerCoreScheduler>,
	/// Start address of the kernel stack
	pub kernel_stack: Cell<u64>,
	/// The core-local async executor.
	ex: StaticLocalExecutor<RawSpinMutex, RawRwSpinLock>,
	/// Queues to handle incoming requests from the other cores
	#[cfg(feature = "smp")]
	pub scheduler_input: InterruptTicketMutex<SchedulerInput>,
	/// Bitmap of supported ISA extensions
	isa_extensions: EnumSet<IsaExtension>,
}

impl CoreLocal {
	pub fn install() {
		unsafe {
			let raw: *const Self;
			asm!("mv {}, gp", out(reg) raw);
			debug_assert_eq!(raw, ptr::null());

			let core_id = CPU_ONLINE.load(Ordering::Relaxed);

			let this = Self {
				core_id,
				scheduler: Cell::new(ptr::null_mut()),
				kernel_stack: Cell::new(0),
				ex: StaticLocalExecutor::new(),
				#[cfg(feature = "smp")]
				scheduler_input: InterruptTicketMutex::new(SchedulerInput::new()),
				isa_extensions: CoreLocal::lookup_isa_extensions(core_id),
			};
			let this = if core_id == 0 {
				take_static::take_static! {
					static FIRST_CORE_LOCAL: Option<CoreLocal> = None;
				}
				FIRST_CORE_LOCAL.take().unwrap().insert(this)
			} else {
				Box::leak(Box::new(this))
			};

			asm!("mv gp, {}", in(reg) this);
		}
	}

	pub fn lookup_isa_extensions(core_id: CoreId) -> EnumSet<IsaExtension> {
		let fdt = env::fdt().unwrap();
		let cpu = fdt
			.cpus()
			.find(|cpu| cpu.property("reg").unwrap().as_usize().unwrap() as u32 == core_id)
			.expect("Could not find CPU node in FDT for core_id {core_id}");
		let isa_extensions = cpu
			.property("riscv,isa-extensions")
			.unwrap()
			.as_str()
			.unwrap();

		let mut bitmap = EnumSet::empty();
		for chunk in isa_extensions.split('\0') {
			if chunk.is_empty() {
				continue;
			}

			if chunk == "zkr" {
				bitmap |= IsaExtension::Zkr;
			}
		}
		bitmap
	}

	#[inline]
	pub fn get() -> &'static Self {
		unsafe {
			let raw: *const Self;
			asm!("mv {}, gp", out(reg) raw);
			debug_assert_ne!(raw, ptr::null());
			&*raw
		}
	}
}

#[inline]
pub fn core_id() -> CoreId {
	CoreLocal::get().core_id
}

#[inline]
pub fn core_scheduler() -> &'static mut PerCoreScheduler {
	unsafe { CoreLocal::get().scheduler.get().as_mut().unwrap() }
}

#[inline]
pub fn set_core_scheduler(scheduler: *mut PerCoreScheduler) {
	CoreLocal::get().scheduler.set(scheduler);
}

pub(crate) fn ex() -> &'static StaticLocalExecutor<RawSpinMutex, RawRwSpinLock> {
	&CoreLocal::get().ex
}

pub fn supports_isa_extension(extension: IsaExtension) -> bool {
	CoreLocal::get().isa_extensions.contains(extension)
}
