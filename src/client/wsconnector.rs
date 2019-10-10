// Buttplug Client Websocket Connector
//
// The big thing to understand here is that we'll only ever need one connection.
// Just one. No more, no less. So there's no real reason to futz with trying to
// get async clients going here other than to lose us a thread, which means we
// shouldn't really need to wait for any network library to update to futures
// 0.3. For now, we can:
//
// - Create a futures channel, retain the receiver in the main thread.
// - Create a ws channel, retain a sender in the main thread
// - Create a thread (for the ws), hand it a sender from the futures channel
// - In ws thread, spin up the connection, waiting on success response in
//   our main thread as a future.
// - Continue on our way with the two channels, happy to know we don't have to
//   wait for networking libraries to get on our futures 0.3 level.

use super::connector::{ButtplugClientConnector,
                       ButtplugClientConnectorError,
                       ButtplugRemoteClientConnectorHelper,
                       ButtplugRemoteClientConnectorMessage,
                       ButtplugRemoteClientConnectorSender};
use super::messagesorter::{ClientConnectorMessageFuture};
use super::client::ButtplugClientError;
use std::thread;
use async_std::task;
use crate::core::messages::ButtplugMessageUnion;
use futures_channel::mpsc;
use async_trait::async_trait;
use ws::{Handler, Handshake, Message, CloseCode};

// TODO Should probably let users pass in their own addresses
const CONNECTION: &'static str = "ws://127.0.0.1:12345";

struct InternalClient {
    buttplug_out: mpsc::UnboundedSender<ButtplugRemoteClientConnectorMessage>,
}

impl Handler for InternalClient {
    fn on_open(&mut self, _: Handshake) -> ws::Result<()> {
        println!("Opened websocket");
        Ok(())
    }

    fn on_message(&mut self, msg: Message) -> ws::Result<()> {
        println!("Got message: {}", msg);
        self.buttplug_out.unbounded_send(ButtplugRemoteClientConnectorMessage::Text(msg.to_string()));
        ws::Result::Ok(())
    }

    fn on_close(&mut self, code: CloseCode, reason: &str) {
        println!("Closed!");
    }

    fn on_error(&mut self, err: ws::Error) {
        println!("The server encountered an error: {:?}", err);
    }
}

pub struct ButtplugWebsocketClientConnector
{
    helper: ButtplugRemoteClientConnectorHelper,
    ws_thread: Option<thread::JoinHandle<()>>,
}

impl ButtplugWebsocketClientConnector {
    pub fn new() -> ButtplugWebsocketClientConnector {
        ButtplugWebsocketClientConnector {
            helper: ButtplugRemoteClientConnectorHelper::new(),
            ws_thread: None,
        }
    }
}

pub struct ButtplugWebsocketWrappedSender {
    sender: ws::Sender
}

unsafe impl Send for ButtplugWebsocketWrappedSender {}
unsafe impl Sync for ButtplugWebsocketWrappedSender {}

impl ButtplugWebsocketWrappedSender {
    pub fn new(send: ws::Sender) -> ButtplugWebsocketWrappedSender {
        ButtplugWebsocketWrappedSender {
            sender: send
        }
    }
}

impl ButtplugRemoteClientConnectorSender for ButtplugWebsocketWrappedSender {
    fn send(&self, msg: ButtplugMessageUnion) {
        let m = "[".to_owned() + &serde_json::to_string(&msg).unwrap() + "]";
        self.sender.send(m);
    }

    fn close(&self) {
        self.sender.close(CloseCode::Normal);
    }
}

#[async_trait]
impl ButtplugClientConnector for ButtplugWebsocketClientConnector {
    async fn connect(&mut self) -> Option<ButtplugClientConnectorError> {
        let send = self.helper.get_remote_send().clone();
        self.ws_thread = Some(thread::spawn(|| {
            ws::connect(CONNECTION, move |out| {
                // Get our websocket sender back to the main thread
                send.unbounded_send(ButtplugRemoteClientConnectorMessage::Sender(
                    Box::new(ButtplugWebsocketWrappedSender::new(out.clone())))).unwrap();
                // Go ahead and create our internal client
                InternalClient {
                    buttplug_out: send.clone()
                }
            });
        }));

        let read_future = self.helper.get_recv_future();
        task::spawn(async {
            read_future.await;
        });
        None
    }

    fn disconnect(&mut self) -> Option<ButtplugClientConnectorError> {
        None
    }

    async fn send(&mut self, msg: &ButtplugMessageUnion) -> Result<ButtplugMessageUnion, ButtplugClientError> {
        self.helper.send(msg).await
    }
}

#[cfg(test)]
mod test {
    use crate::client::client::ButtplugClient;
    use super::ButtplugClientConnector;
    use super::ButtplugWebsocketClientConnector;
    use async_std::task;

    #[test]
    fn test_websocket() {
        task::block_on(async {
            assert!(ButtplugWebsocketClientConnector::new().connect().await.is_none());
        })
    }

    #[test]
    fn test_client_websocket() {
        task::block_on(async {
            println!("connecting");
            let mut client = ButtplugClient::new("test client");
            client.connect(ButtplugWebsocketClientConnector::new()).await;
            println!("connected");
        })
    }
}
