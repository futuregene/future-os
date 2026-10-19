use futures::{
    future::{AbortHandle, Abortable, BoxFuture, Shared},
    FutureExt,
};
use tokio::sync::watch;

/// Cancellation and completion belong to the controlled-endpoint instance. The host
/// decides where the wrapped future runs and can also cancel it during exit.
#[derive(Clone)]
pub struct TaskHandle {
    abort: AbortHandle,
    done: watch::Receiver<Outcome>,
    completion: Shared<BoxFuture<'static, Outcome>>,
}

#[derive(Clone, Copy)]
enum Outcome {
    Pending,
    Completed,
    Cancelled,
    Panicked,
}

struct Completion(watch::Sender<Outcome>);
impl Drop for Completion {
    fn drop(&mut self) {
        if matches!(*self.0.borrow(), Outcome::Pending) {
            self.0.send_replace(Outcome::Cancelled);
        }
    }
}

impl TaskHandle {
    pub fn abort(&self) {
        self.abort.abort();
    }
    pub fn is_finished(&self) -> bool {
        !matches!(*self.done.borrow(), Outcome::Pending)
    }
    pub async fn completed(mut self) {
        while matches!(*self.done.borrow_and_update(), Outcome::Pending) {
            if self.done.changed().await.is_err() {
                break;
            }
        }
    }
}

impl std::future::Future for TaskHandle {
    type Output = Result<(), crate::Error>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let outcome = std::task::ready!(self.completion.poll_unpin(cx));
        std::task::Poll::Ready(match outcome {
            Outcome::Completed => Ok(()),
            Outcome::Panicked => Err(crate::Error::Message("remote_server_task_panicked".into())),
            _ => Err(crate::Error::Message("remote_server_task_cancelled".into())),
        })
    }
}

pub(crate) fn tracked(future: BoxFuture<'static, ()>) -> (TaskHandle, BoxFuture<'static, ()>) {
    let (abort, registration) = AbortHandle::new_pair();
    let (done, completed) = watch::channel(Outcome::Pending);
    // Construct before host registration: dropping a rejected/unpolled future
    // must signal completion just as cancelling a running future does.
    let completion = Completion(done);
    let wrapped = async move {
        let outcome = std::panic::AssertUnwindSafe(Abortable::new(future, registration))
            .catch_unwind()
            .await;
        match outcome {
            Ok(Ok(())) => {
                completion.0.send_replace(Outcome::Completed);
            }
            Ok(Err(_)) => {
                completion.0.send_replace(Outcome::Cancelled);
            }
            Err(panic) => {
                completion.0.send_replace(Outcome::Panicked);
                std::panic::resume_unwind(panic);
            }
        }
    }
    .boxed();
    let mut status = completed.clone();
    let completion = async move {
        while matches!(*status.borrow_and_update(), Outcome::Pending) {
            if status.changed().await.is_err() {
                break;
            }
        }
        *status.borrow()
    }
    .boxed()
    .shared();
    (
        TaskHandle {
            abort,
            done: completed,
            completion,
        },
        wrapped,
    )
}

/// A native job cannot be aborted once its synchronous body starts. Completion
/// belongs to the job itself, including rejected/unstarted jobs and unwinding.
#[derive(Clone)]
pub(crate) struct NativeHandle {
    done: watch::Receiver<bool>,
}
struct NativeCompletion(watch::Sender<bool>);
impl Drop for NativeCompletion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl NativeHandle {
    pub(crate) fn is_finished(&self) -> bool {
        *self.done.borrow()
    }
    pub(crate) async fn completed(mut self) {
        while !*self.done.borrow_and_update() {
            if self.done.changed().await.is_err() {
                break;
            }
        }
    }
}
pub(crate) fn native(
    job: Box<dyn FnOnce() + Send + 'static>,
) -> (NativeHandle, Box<dyn FnOnce() + Send + 'static>) {
    let (done, receiver) = watch::channel(false);
    let completion = NativeCompletion(done);
    (
        NativeHandle { done: receiver },
        Box::new(move || {
            let _completion = completion;
            job();
        }),
    )
}
