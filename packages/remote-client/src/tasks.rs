use futures::{
    future::{AbortHandle, Abortable, BoxFuture},
    FutureExt,
};
use tokio::sync::watch;

/// Cancellation and completion belong to the controller instance. The host
/// decides where the wrapped future runs and can also cancel it during exit.
#[derive(Clone)]
pub struct TaskHandle {
    abort: AbortHandle,
    done: watch::Receiver<Outcome>,
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

impl std::future::IntoFuture for TaskHandle {
    type Output = Result<(), crate::Error>;
    type IntoFuture = BoxFuture<'static, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let outcome = self.done.clone();
            self.completed().await;
            let outcome = *outcome.borrow();
            match outcome {
                Outcome::Completed => Ok(()),
                Outcome::Panicked => {
                    Err(crate::Error::Message("remote_client_task_panicked".into()))
                }
                _ => Err(crate::Error::Message("remote_client_task_cancelled".into())),
            }
        }
        .boxed()
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
    (
        TaskHandle {
            abort,
            done: completed,
        },
        wrapped,
    )
}
