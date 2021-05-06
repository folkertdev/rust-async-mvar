#![no_std]

/// # Asynchronous mutable variable
///
/// Based on haskell's [Control.Concurrent.MVar](https://hackage.haskell.org/package/base-4.15.0.0/docs/Control-Concurrent-MVar.html). Per its documentation:
///
/// > An `MVar t` is mutable location that is either empty or contains a value of type `t`. It has two fundamental operations: `putMVar` which fills an MVar if it is empty and blocks otherwise, and `takeMVar` which empties an MVar if it is full and blocks otherwise.
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::Poll;
use futures::task::AtomicWaker;

use core::sync::atomic::AtomicU8;

// (empty, filling, emptying, full)
const MVAR_EMPTY: u8 = 1;
const MVAR_FILLING: u8 = 2;
const MVAR_EMPTYING: u8 = 4;
const MVAR_FULL: u8 = 8;

#[derive(Debug)]
pub struct MVar<T> {
    item: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
    take_waker: AtomicWaker,
    put_waker: AtomicWaker,
}

unsafe impl<T: Sync> Sync for MVar<T> {}

impl<T> MVar<T> {
    pub const fn new_empty() -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(MVAR_EMPTY),
            take_waker: AtomicWaker::new(),
            put_waker: AtomicWaker::new(),
        }
    }

    pub const fn new(item: T) -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::new(item)),
            state: AtomicU8::new(MVAR_FULL),
            take_waker: AtomicWaker::new(),
            put_waker: AtomicWaker::new(),
        }
    }

    pub const fn take(&self) -> TakeFuture<T> {
        TakeFuture { mvar: self }
    }

    pub const fn put(&self, item: T) -> PutFuture<T> {
        PutFuture {
            mvar: self,
            item: Some(item),
        }
    }

    fn _take(&self) -> Option<T> {
        let can_take = self.state.compare_exchange(
            MVAR_FULL,
            MVAR_EMPTYING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        if let Ok(MVAR_FULL) = can_take {
            let mut value = MaybeUninit::uninit();

            self.item
                .with_mut(|ptr| core::mem::swap(unsafe { &mut *ptr }, &mut value));

            self.state.store(MVAR_EMPTY, Ordering::SeqCst);

            unsafe { Some(value.assume_init()) }
        } else {
            None
        }
    }
}

#[must_use]
pub struct TakeFuture<'a, T> {
    mvar: &'a MVar<T>,
}

impl<'a, T> Future for TakeFuture<'a, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
        // register before the computation
        self.mvar.take_waker.register(cx.waker());

        match self.mvar._take() {
            Some(value) => {
                // wake after the computation
                self.mvar.put_waker.wake();

                Poll::Ready(value)
            }
            None => Poll::Pending,
        }
    }
}

#[must_use]
pub struct PutFuture<'a, T> {
    item: Option<T>,
    mvar: &'a MVar<T>,
}

impl<'a, T: Unpin> Future for PutFuture<'a, T> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
        // register before the computation
        self.mvar.put_waker.register(cx.waker());

        let can_put = self.mvar.state.compare_exchange(
            MVAR_EMPTY,
            MVAR_FILLING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        if let Ok(MVAR_EMPTY) = can_put {
            let reference = Pin::into_inner(self);

            let opt_value = core::mem::take(&mut reference.item);

            match opt_value {
                Some(value) => {
                    let mut muvalue = MaybeUninit::new(value);
                    reference
                        .mvar
                        .item
                        .with_mut(|ptr| core::mem::swap(unsafe { &mut *ptr }, &mut muvalue));

                    reference.mvar.state.store(MVAR_FULL, Ordering::SeqCst);

                    // wake after the computation
                    reference.mvar.take_waker.wake();
                }
                None => {
                    unreachable!("the same PutFuture is used twice");
                }
            }

            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl<T> Drop for MVar<T> {
    fn drop(&mut self) {
        // make sure to drop the item
        self._take();
    }
}

#[cfg(not(loom))]
#[derive(Debug)]
struct UnsafeCell<T>(core::cell::UnsafeCell<T>);

#[cfg(not(loom))]
impl<T> UnsafeCell<T> {
    const fn new(data: T) -> UnsafeCell<T> {
        UnsafeCell(core::cell::UnsafeCell::new(data))
    }

    //    pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
    //        f(self.0.get())
    //    }

    fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
