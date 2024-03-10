use std::future::Future;
use std::marker::PhantomData;
use std::mem;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::future::BoxFuture;
use futures_util::FutureExt;

use crate::chan::ActorMessage;
use crate::envelope::Shutdown;
use crate::mailbox::Mailbox;
use crate::Message;

impl<A> Message<A> {
    /// Dispatches this message to the given actor.
    pub fn dispatch_to(self, actor: &mut A) -> DispatchFuture<'_, A> {
        DispatchFuture::new(
            self.inner,
            actor,
            Mailbox::from_parts(self.channel, self.broadcast_mailbox),
        )
    }
}

/// Represents the dispatch of a message to an actor.
///
/// This future is **not** cancellation-safe. Dropping it will interrupt the execution of
/// [`Handler::handle`](crate::Handler::handle) which may leave the actor in an inconsistent state.
#[must_use = "Futures do nothing unless polled"]
pub struct DispatchFuture<'a, A> {
    state: State<'a, A>,
}

impl<'a, A> DispatchFuture<'a, A> {
    fn running(msg: ActorMessage<A>, act: &'a mut A, mailbox: Mailbox<A>) -> DispatchFuture<'a, A> {
        let fut = match msg {
            ActorMessage::ToOneActor(msg) => msg.handle(act, mailbox),
            ActorMessage::ToAllActors(msg) => msg.handle(act, mailbox),
            ActorMessage::Shutdown => Shutdown::<A>::handle(),
        };

        DispatchFuture {
            state: State::Running {
                fut,
                phantom: PhantomData,
            },
        }
    }
}

enum State<'a, A> {
    New {
        msg: ActorMessage<A>,
        act: &'a mut A,
        mailbox: Mailbox<A>,
    },
    Running {
        fut: BoxFuture<'a, ControlFlow<()>>,
        phantom: PhantomData<fn(&'a A)>,
    },
    Done,
}

impl<'a, A> DispatchFuture<'a, A> {
    pub fn new(msg: ActorMessage<A>, act: &'a mut A, mailbox: Mailbox<A>) -> Self {
        DispatchFuture {
            state: State::New { msg, act, mailbox },
        }
    }
}

impl<'a, A> Future for DispatchFuture<'a, A> {
    type Output = ControlFlow<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match mem::replace(&mut self.state, State::Done) {
            State::New { msg, act, mailbox } => {
                *self = DispatchFuture::running(msg, act, mailbox);
                self.poll(cx)
            }
            State::Running { mut fut, phantom } => {
                match fut.poll_unpin(cx) {
                    Poll::Ready(flow) => Poll::Ready(flow),
                    Poll::Pending => {
                        self.state = State::Running { fut, phantom };
                        Poll::Pending
                    }
                }
            }
            State::Done => panic!("Polled after completion"),
        }
    }
}
