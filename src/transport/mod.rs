use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

use http::Uri;

use crate::proxy::Proxy;
use crate::resolver::Resolver;
use crate::{AgentConfig, Error};

mod tcp;

pub trait Connector: Debug + Send + Sync + 'static {
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
    fn borrow_buffers(&mut self) -> Buffers;
    fn transmit_output(&mut self, amount: usize, timeout: Duration) -> Result<(), Error>;
    fn await_input(&mut self, timeout: Duration, is_body: bool) -> Result<Buffers, Error>;
    fn consume_input(&mut self, amount: usize);
}

pub struct Buffers<'a> {
    pub input: &'a mut [u8],
    pub output: &'a mut [u8],
}

impl Buffers<'_> {
    pub(crate) fn empty() -> Buffers<'static> {
        Buffers {
            input: &mut [],
            output: &mut [],
        }
    }
}

pub struct LazyBuffers {
    input_size: usize,
    output_size: usize,
    input: Vec<u8>,
    output: Vec<u8>,

    // We have two modes. One where input is filled with some incoming data,
    // and one where we can use it freely. These are represented by
    // Some/None in this Option respectively.
    input_filled: Option<usize>,

    // If we have input_filled: Some(value), this is the amount of that value
    // we have consumed.
    input_consumed: usize,
}

impl LazyBuffers {
    pub fn new(input_size: usize, output_size: usize) -> Self {
        assert!(input_size > 0);
        assert!(output_size > 0);

        LazyBuffers {
            input_size,
            output_size,
            // Vectors don't allocate until they get a size.
            input: vec![],
            output: vec![],

            input_filled: None,
            input_consumed: 0,
        }
    }

    /// Borrow the buffers.
    ///
    /// This allocates first time it's used.
    ///
    /// The input buffer might be scaled to what's left unconsumed if we are in "fill mode".
    pub fn borrow_mut(&mut self) -> Buffers<'_> {
        if self.input.is_empty() {
            self.input.resize(self.input_size, 0);
        }
        if self.output.is_empty() {
            self.output.resize(self.output_size, 0);
        }

        // Unput is scaled to whatever is unconsumed.
        let input = if let Some(filled) = self.input_filled {
            &mut self.input[self.input_consumed..filled]
        } else {
            &mut self.input[..]
        };

        Buffers {
            input,
            output: &mut self.output,
        }
    }

    /// Query how much input is unconsumed.
    pub fn unconsumed(&self) -> usize {
        if let Some(filled) = self.input_filled {
            filled
                .checked_sub(self.input_consumed)
                // This is an error condition. Something in the buffer handling
                // has consumed more than is possible.
                .expect("consumed is greater than filled")
        } else {
            0
        }
    }

    /// Switch mode to "filled input" by setting how much of the input was filled.
    ///
    /// There cannot be a previous set_input_filled that hasn't been entirely consumed.
    pub fn set_input_filled(&mut self, input_filled: usize) {
        // Assert there isn't unconsumed input.
        self.assert_and_clear_input_filled();

        self.input_filled = Some(input_filled);
    }

    /// Switch mode to "free input" by unsetting the filled value. This checks the
    /// entire input was consumed.
    pub fn assert_and_clear_input_filled(&mut self) {
        let unconsumed = self.unconsumed();

        if unconsumed > 0 {
            // This is a hard error. It indicates a state bug higher up in ureq. Ignoring
            // it would be a security risk because we would silently discard input sent
            // by the remote server potentially opening for request smuggling
            // attacks etc.
            panic!("input contains {} unconsumed bytes", unconsumed);
        }

        self.input_filled = None;
        self.input_consumed = 0;
    }

    /// Mark some input as consumed.
    ///
    /// This ensure we are in the correct "fill mode" and that there are bytes left to consume.
    fn consume_input(&mut self, amount: usize) {
        // This indicates the order of calls is not correct. We must
        // first set_input_fileld(), then consume_input()
        assert!(
            self.input_filled.is_some(),
            "consume without a filled buffer"
        );

        // This indicates some state bug where the caller tries to consume
        // more than is filled.
        if amount > self.unconsumed() {
            panic!(
                "consume more than unconsumed {} > {}",
                amount,
                self.unconsumed()
            );
        }

        self.input_consumed += amount;
    }
}

#[derive(Debug)]
pub struct DefaultConnector;

impl Connector for DefaultConnector {
    fn connect(
        &self,
        _details: &ConnectionDetails,
        _previous: Option<Box<dyn Transport>>,
    ) -> Result<Option<Box<dyn Transport>>, Error> {
        todo!()
    }
}
