#![no_std]
#![forbid(unsafe_code)]

use core::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::{future::FusedFuture, ready};
use pin_project::pin_project;

/// Future for [`with_try_background`][FutureExt::with_try_background]
#[pin_project]
#[derive(Debug, Clone, Default)]
pub struct TryForegroundBackground<F, B> {
    #[pin]
    foreground: F,

    #[pin]
    background: B,
}

impl<F, B, E> Future for TryForegroundBackground<F, B>
where
    F: Future,
    B: Future<Output = Result<(), E>> + FusedFuture,
{
    type Output = Result<F::Output, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match this.foreground.poll(cx) {
            Poll::Ready(value) => Poll::Ready(Ok(value)),
            Poll::Pending if this.background.is_terminated() => Poll::Pending,
            Poll::Pending => match ready!(this.background.poll(cx)) {
                Ok(()) => Poll::Pending,
                Err(err) => Poll::Ready(Err(err)),
            },
        }
    }
}

#[pin_project]
#[derive(Debug, Clone, Default)]
struct NeverError<F> {
    #[pin]
    fut: F,
}

impl<F: Future> Future for NeverError<F> {
    type Output = Result<F::Output, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx).map(Ok)
    }
}

impl<F: FusedFuture> FusedFuture for NeverError<F> {
    fn is_terminated(&self) -> bool {
        self.fut.is_terminated()
    }
}

/// Future for [`with_background`][FutureExt::with_background]
#[pin_project]
#[derive(Debug, Clone, Default)]
pub struct ForegroundBackground<F, B> {
    #[pin]
    inner: TryForegroundBackground<F, NeverError<B>>,
}

impl<F, B> Future for ForegroundBackground<F, B>
where
    F: Future,
    B: Future<Output = ()> + FusedFuture,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|res| match res {
            Ok(value) => value,
            Err(infallible) => match infallible {},
        })
    }
}

impl<F, B> FusedFuture for ForegroundBackground<F, B>
where
    F: FusedFuture,
    B: Future<Output = ()> + FusedFuture,
{
    fn is_terminated(&self) -> bool {
        self.inner.foreground.is_terminated()
    }
}

/**
Extension trait for [`Future`] providing adapters to run other futures
concurrently in the background.
*/
pub trait FutureExt: Sized + Future {
    /**
    Arrange for a future to run concurrently with a background future.

    This method returns a new future that waits for `self` to complete
    while also concurrently running `background`. If `background` finishes
    before `self`, it will still wait for `self` to complete. If `self`
    finishes first, the future will resolve immediately and no longer poll
    `background`.

    # Example

    ```rust
    use std::time::Duration;

    use futures::future::FutureExt as _;
    use futures::executor::block_on;
    use async_channel::{bounded, Receiver, Sender};

    use foreback::FutureExt as _;

    /// Send pings forever to a channel
    async fn ticker(sender: Sender<()>) {
        while let Ok(()) = sender.send(()).await {}
    }

    block_on(async {
        let (send, recv) = bounded(1);

        let result = async move {
            let mut counter = 0;

            for _ in 0..5 {
                let () = recv.recv().await.unwrap();
                counter += 1;
            }

            counter
        }
            .with_background(ticker(send).fuse())
            .await;

        assert_eq!(result, 5);
    });
    ```
    */
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    fn with_background<B>(self, background: B) -> ForegroundBackground<Self, B>
    where
        B: Future<Output = ()> + FusedFuture,
    {
        ForegroundBackground {
            inner: self.with_try_background(NeverError { fut: background }),
        }
    }

    /**
    Arrange for a future to run concurrently with a fallible background
    future.

    This method returns a new future that waits for `self` to complete
    while also concurrently running `background`. If `background` finishes
    before `self`, it will still wait for `self` to complete. If `self`
    finishes first, the future will resolve immediately and no longer poll
    `background`. If `background` finishes with an error, that error will
    be returned immediately, and `self` will no longer be polled.

    # Example

    ```rust
    use std::time::Duration;

    use futures::future::{FutureExt as _, pending};
    use futures::executor::block_on;
    use async_channel::{bounded, Receiver, Sender, SendError};

    use foreback::FutureExt as _;

    /// Send pings forever to a channel. Return an error if the channel closes.
    async fn ticker(sender: Sender<()>) -> Result<(), SendError<()>> {
        loop {
            match sender.send(()).await {
                Ok(()) => {},
                Err(err) => return Err(err),
            }
        }
    }

    block_on(async {
        let (send, recv) = bounded(1);

        let result = async move {
            let mut counter = 0;

            for _ in 0..5 {
                let () = recv.recv().await.unwrap();
                counter += 1;
            }

            counter
        }
            .with_try_background(ticker(send).fuse())
            .await
            .unwrap();

        assert_eq!(result, 5);
    });

    block_on(async {
        let (send, recv) = bounded(1);

        let result = async move {
            let mut counter = 0;

            for _ in 0..5 {
                let () = recv.recv().await.unwrap();
                counter += 1;
            }

            drop(recv);
            let () = pending().await;
            counter
        }
            .with_try_background(ticker(send).fuse())
            .await
            .unwrap_err();
    });
    ```
    */
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    fn with_try_background<B, E>(self, background: B) -> TryForegroundBackground<Self, B>
    where
        B: Future<Output = Result<(), E>>,
    {
        TryForegroundBackground {
            foreground: self,
            background,
        }
    }
}

impl<F: Future> FutureExt for F {}
