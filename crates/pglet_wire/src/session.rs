/// Session state machine for handling Postgres protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Waiting for startup message.
    AwaitingStartup,
    /// Ready for queries.
    Ready,
    /// In extended query protocol (Parse/Bind/Execute flow).
    Extended,
    /// Session terminated.
    Terminated,
}

/// Connection session state.
#[derive(Debug)]
pub struct Session {
    pub state: SessionState,
    pub process_id: i32,
    pub secret_key: i32,
    pub params: Vec<(String, String)>,
}

impl Session {
    pub fn new(process_id: i32) -> Self {
        Self {
            state: SessionState::AwaitingStartup,
            process_id,
            secret_key: rand_i32(),
            params: Vec::new(),
        }
    }

    pub const fn is_startup(&self) -> bool {
        matches!(self.state, SessionState::AwaitingStartup)
    }
}

fn rand_i32() -> i32 {
    let mut buf = [0u8; 4];
    // Panic is appropriate here - if the OS can't provide entropy, we can't securely generate
    // session keys and should fail fast rather than use predictable values.
    #[allow(clippy::expect_used)]
    getrandom::fill(&mut buf).expect("getrandom failed");
    i32::from_ne_bytes(buf)
}
