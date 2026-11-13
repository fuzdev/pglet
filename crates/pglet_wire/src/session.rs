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
    #[expect(
        clippy::expect_used,
        reason = "without OS entropy there is no secure session key, so fail fast rather than use a predictable value"
    )]
    getrandom::fill(&mut buf).expect("getrandom failed");
    i32::from_ne_bytes(buf)
}
