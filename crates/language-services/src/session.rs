//! Generation-safe language-server session lifecycle contracts.
//!
//! Services own child processes; this module models only the transitions and
//! identity that let app/controllers reject output from an old process after a
//! restart or workspace change.
use crate::{LanguageId, SessionGeneration};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub language: LanguageId,
    pub workspace_uri: String,
    pub configuration_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    Stopped,
    Starting,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionTransitionError {
    NotStarting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSession {
    pub key: SessionKey,
    pub generation: SessionGeneration,
    pub state: SessionState,
}

impl ServerSession {
    #[must_use]
    pub fn new(key: SessionKey) -> Self {
        Self {
            key,
            generation: SessionGeneration::default(),
            state: SessionState::Stopped,
        }
    }

    /// Begin a fresh process generation. All events tagged with the previous
    /// generation must be discarded by the app/service boundary.
    pub fn start(&mut self) {
        self.generation = self.generation.next();
        self.state = SessionState::Starting;
    }

    pub fn initialized(&mut self) -> Result<(), SessionTransitionError> {
        if self.state != SessionState::Starting {
            return Err(SessionTransitionError::NotStarting);
        }
        self.state = SessionState::Ready;
        Ok(())
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        self.state = SessionState::Failed(message.into());
    }

    pub fn stop(&mut self) {
        self.state = SessionState::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SessionKey {
        SessionKey {
            language: LanguageId::RUST,
            workspace_uri: "file:///workspace".into(),
            configuration_fingerprint: "default".into(),
        }
    }

    #[test]
    fn lifecycle_transitions_and_restart_generation_are_explicit() {
        let mut session = ServerSession::new(key());
        assert_eq!(session.state, SessionState::Stopped);
        assert_eq!(
            session.initialized(),
            Err(SessionTransitionError::NotStarting)
        );
        session.start();
        assert_eq!(session.generation, SessionGeneration(1));
        assert_eq!(session.state, SessionState::Starting);
        session.initialized().unwrap();
        assert_eq!(session.state, SessionState::Ready);
        session.fail("server exited");
        assert_eq!(session.state, SessionState::Failed("server exited".into()));
        session.start();
        assert_eq!(session.generation, SessionGeneration(2));
        assert_eq!(session.state, SessionState::Starting);
        session.stop();
        assert_eq!(session.state, SessionState::Stopped);
    }
}
