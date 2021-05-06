#![no_std]

use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::Poll;
use futures::task::AtomicWaker;

pub(crate) use core::sync::atomic::AtomicBool;

#[derive(Debug)]
pub struct MVar<T> {
    item: UnsafeCell<MaybeUninit<T>>,
    is_full: AtomicBool,
    take_waker: AtomicWaker,
    put_waker: AtomicWaker,
}

unsafe impl<T: Sync> Sync for MVar<T> {}

impl<T> MVar<T> {
    pub const fn new_empty() -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::uninit()),
            is_full: AtomicBool::new(false),
            take_waker: AtomicWaker::new(),
            put_waker: AtomicWaker::new(),
        }
    }

    pub const fn new(item: T) -> Self {
        Self {
            item: UnsafeCell::new(MaybeUninit::new(item)),
            is_full: AtomicBool::new(true),
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
        let can_take =
            self.is_full
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst);

        if let Ok(true) = can_take {
            self.put_waker.wake();

            let mut value = MaybeUninit::uninit();

            self.item
                .with_mut(|ptr| core::mem::swap(unsafe { &mut *ptr }, &mut value));

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
        self.mvar.take_waker.register(cx.waker());

        match self.mvar._take() {
            Some(value) => Poll::Ready(value),
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
        self.mvar.put_waker.register(cx.waker());

        if let Ok(false) =
            self.mvar
                .is_full
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        {
            let reference = Pin::into_inner(self);
            reference.mvar.take_waker.wake();

            let mut opt_value = None;

            core::mem::swap(&mut reference.item, &mut opt_value);

            match opt_value {
                Some(value) => {
                    let mut muvalue = MaybeUninit::new(value);
                    reference
                        .mvar
                        .item
                        .with_mut(|ptr| core::mem::swap(unsafe { &mut *ptr }, &mut muvalue));
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
