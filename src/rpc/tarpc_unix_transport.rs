use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};
use tarpc::serde_transport::Transport;
use tokio::net::{UnixListener, UnixStream};
use tokio_serde::{Deserializer, Serializer};

#[pin_project::pin_project]
pub(super) struct Incoming<Item, SinkItem, Codec, CodecFn> {
    listener: UnixListener,
    codec_fn: CodecFn,
    _marker: PhantomData<(Codec, Item, SinkItem)>,
}

pub(super) fn incoming<Item, SinkItem, Codec, CodecFn>(
    listener: UnixListener,
    codec_fn: CodecFn,
) -> Incoming<Item, SinkItem, Codec, CodecFn>
where
    Item: for<'de> Deserialize<'de>,
    Codec: Serializer<SinkItem> + Deserializer<Item>,
    CodecFn: Fn() -> Codec,
{
    Incoming {
        listener,
        codec_fn,
        _marker: PhantomData,
    }
}

impl<Item, SinkItem, Codec, CodecFn> Stream for Incoming<Item, SinkItem, Codec, CodecFn>
where
    Item: for<'de> Deserialize<'de>,
    SinkItem: Serialize,
    Codec: Serializer<SinkItem> + Deserializer<Item>,
    CodecFn: Fn() -> Codec,
{
    type Item = std::io::Result<Transport<UnixStream, Item, SinkItem, Codec>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next = futures_util::ready!(
            Pin::new(&mut self.as_mut().project().listener.incoming()).poll_next(cx)?
        );
        Poll::Ready(next.map(|conn| Ok(new(conn, (self.codec_fn)()))))
    }
}

pub(super) fn new<Item, SinkItem, Codec>(
    io: UnixStream,
    codec: Codec,
) -> Transport<UnixStream, Item, SinkItem, Codec>
where
    Item: for<'de> Deserialize<'de>,
    SinkItem: Serialize,
    Codec: Serializer<SinkItem> + Deserializer<Item>,
{
    Transport::from((io, codec))
}
