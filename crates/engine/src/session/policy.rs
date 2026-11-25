//! A host's command policy: an application embedding the engine (a read-only viewer, a kiosk)
//! decides which commands may run. Every way of starting a command asks it: programmatic
//! execution ([`Session::execute`]), the interactive start ([`Session::start`]: typed names,
//! aliases, menus, shortcuts, Enter repeating the last command), grip edits (asked as `grip`)
//! and AutoLISP (asked as `lisp`: its `entmake`, `entmod` and `entdel` change the drawing
//! without a command). A UI asks it for its own commands through [`Session::check_command`].

use super::{EngineError, Result, Session, Value};
use crate::CommandPolicy;

impl Session {
    /// Sets the host's command policy, or removes it with `None` (every command may run, the
    /// default). The policy gets the command id (after alias resolution) and its JSON
    /// parameters (`null` for an interactive start). A refused command is an error and changes
    /// nothing; a command already running is left running.
    pub fn set_command_policy(&mut self, policy: Option<CommandPolicy>) {
        self.command_policy = policy;
    }

    /// Whether the command policy lets `id` run with `params`. True without a policy.
    pub fn command_allowed(&self, id: &str, params: &Value) -> bool {
        self.command_policy.as_ref().is_none_or(|allowed| allowed(id, params))
    }

    /// [`Session::command_allowed`] as the error a refused command reports.
    pub fn check_command(&self, id: &str, params: &Value) -> Result<()> {
        if self.command_allowed(id, params) { Ok(()) } else { Err(EngineError::Other(format!("{} is not available here", id.to_ascii_uppercase()))) }
    }
}
