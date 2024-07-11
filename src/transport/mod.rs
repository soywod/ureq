use std::fmt::Debug;
use std::net::SocketAddr;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::time::Duration;
use crate::{AgentConfig, Error};

use self::tcp::TcpConnector;

mod buf;
pub use buf::{Buffers, LazyBuffers, NoBuffers};

mod tcp;

mod io;
pub use io::TransportAdapter;

mod chain;
pub use chain::ChainedConnector;

#[cfg(feature = "socks-proxy")]
mod socks;
#[cfg(feature = "socks-proxy")]
pub use self::socks::SocksConnector;

pub trait Connector: Debug + Send + Sync + 'static {
    fn boxed(self) -> Box<dyn Connector>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error>;
}

pub struct ConnectionDetails<'a> {
    pub uri: &'a Uri,
    pub addr: SocketAddr,
    pub proxy: &'a Option<Proxy>,
    pub resolver: &'a dyn Resolver,
    pub config: &'a AgentConfig,

    // TODO(martin): Make mechanism to lower duration for each step in the connector chain.
    pub timeout: Duration,
}

pub trait Transport: Debug + Send + Sync {
    fn buffers(&mut self) -> &mut dyn Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn await_input(&mut self, timeout: Duration) -> Result<(), Error>;
    fn consume_input(&mut self, amount: usize);
    fn is_tls(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct DefaultConnector {
    chain: ChainedConnector,
}

impl DefaultConnector {
    pub fn new() -> Self {
        let chain = ChainedConnector::new([
            //
            // If we are using socks-proxy, that takes precedence over TcpConnector.
            #[cfg(feature = "socks-proxy")]
            SocksConnector::default().boxed(),
            //
            // If the config indicates we ought to use a socks proxy
            // and the feature flag isn't enabled, we should warn the user.
            #[cfg(not(feature = "socks-proxy"))]
            WarnOnNoSocksConnector.boxed(),
            //
            // If we didn't get a socks-proxy, open a Tcp connection
            TcpConnector.boxed(),
            //
            // If rustls is enabled, prefer that
            #[cfg(feature = "rustls")]
            crate::tls::RustlsConnector::default().boxed(),
            //
            // As a fallback if rustls isn't enabled, use native-tls
            #[cfg(feature = "native-tls")]
            crate::tls::NativeTlsConnector::default().boxed(),
        ]);

        DefaultConnector { chain }
    }
}

impl Connector for DefaultConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        self.chain.connect(details, chained)
    }
}

#[derive(Debug)]
pub struct WarnOnNoSocksConnector;

impl Connector for WarnOnNoSocksConnector {
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        if chained.is_none() {
            if let Some(proxy) = &details.proxy {
                if proxy.proto().is_socks() {
                    if proxy.is_from_env() {
                        warn!("Enable feature socks-proxy to use proxy configured by environment variables");
                    } else {
                        // If a user bothered to manually create a AgentConfig.proxy setting, and it's
                        // not honored, assume it's a serious error.
                        panic!("Enable feature socks-proxy to use manually configured proxy");
                    }
                }
            }
        }
        return Ok(chained);
    }
}
