use tower::{
    Layer,
    limit::{ConcurrencyLimit, ConcurrencyLimitLayer},
};

/// 限流中间件层
#[derive(Clone)]
pub struct RateLimitLayer {
    max_concurrent: usize,
}

impl RateLimitLayer {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent: max_concurrent.max(1),
        }
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

impl<S> Layer<S> for RateLimitLayer
where
    S: Send + 'static,
{
    type Service = ConcurrencyLimit<S>;

    fn layer(&self, service: S) -> Self::Service {
        ConcurrencyLimitLayer::new(self.max_concurrent).layer(service)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        future::{Future, poll_fn},
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use tokio::{sync::Notify, time::timeout};
    use tower::{Layer, Service};

    use super::RateLimitLayer;

    #[derive(Clone)]
    struct BlockingService {
        started: Arc<AtomicUsize>,
        release: Arc<Notify>,
    }

    impl Service<()> for BlockingService {
        type Response = ();
        type Error = Infallible;
        type Future = Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: ()) -> Self::Future {
            let started = self.started.clone();
            let release = self.release.clone();

            Box::pin(async move {
                started.fetch_add(1, Ordering::SeqCst);
                release.notified().await;
                Ok(())
            })
        }
    }

    #[test]
    fn zero_limit_is_normalized_to_one() {
        let layer = RateLimitLayer::new(0);

        assert_eq!(layer.max_concurrent(), 1);
    }

    #[tokio::test]
    async fn enforces_concurrent_request_limit() {
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let mut service = RateLimitLayer::new(1).layer(BlockingService {
            started: started.clone(),
            release: release.clone(),
        });

        poll_fn(|cx| service.poll_ready(cx)).await.unwrap();
        let first = tokio::spawn(service.call(()));

        while started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        let second_ready = timeout(
            Duration::from_millis(25),
            poll_fn(|cx| service.poll_ready(cx)),
        )
        .await;
        assert!(second_ready.is_err());

        release.notify_waiters();
        first.await.unwrap().unwrap();

        timeout(
            Duration::from_millis(100),
            poll_fn(|cx| service.poll_ready(cx)),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
