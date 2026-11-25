use std::marker::PhantomData;
use std::rc::Rc;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

const RPC_E_CHANGED_MODE: i32 = 0x80010106u32 as i32;

pub(crate) struct Apartment {
    initialized: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Apartment {
    pub(crate) fn open() -> Result<Self, String> {
        let initialized = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => true,
            Err(error) if error.code().0 == RPC_E_CHANGED_MODE => false,
            Err(error) => return Err(format!("RoInitialize 失败：0x{:08X}", error.code().0 as u32)),
        };
        Ok(Self { initialized, _thread: PhantomData })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { RoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED;

    #[test]
    fn repeated_initialization_on_the_same_thread_succeeds() {
        std::thread::spawn(|| {
            let first = Apartment::open().unwrap();
            let second = Apartment::open().unwrap();
            assert!(first.initialized);
            assert!(second.initialized);
            drop(second);
            drop(first);
            let fresh = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) };
            assert!(fresh.is_ok(), "initialization references must be balanced: {fresh:?}");
            unsafe { RoUninitialize() };
        }).join().unwrap();
    }

    #[test]
    fn existing_single_threaded_apartment_is_not_uninitialized() {
        std::thread::spawn(|| {
            unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.unwrap();
            let borrowed = Apartment::open().unwrap();
            assert!(!borrowed.initialized);
            drop(borrowed);
            let changed = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            assert_eq!(changed.unwrap_err().code().0, RPC_E_CHANGED_MODE);
            unsafe { RoUninitialize() };
        }).join().unwrap();
    }

}
