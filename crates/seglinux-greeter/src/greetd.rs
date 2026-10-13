//! greetd's greeter protocol (greetd-ipc(7)): JSON messages over the Unix
//! socket in `$GREETD_SOCK`, each preceded by its length as a native-endian
//! 32-bit number. greetd runs PAM; the greeter only relays its questions
//! (the password prompt) and, once PAM agrees, names the session to start.
//! greetd starts it when the greeter exits.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<String> },
    StartSession { cmd: Vec<String>, env: Vec<String> },
    CancelSession,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    AuthError,
    Error,
}

#[derive(Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: String,
    },
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: String,
    },
}

/// Why signing in did not work, worded for the person signing in.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The password was not accepted.
    WrongPassword,
    /// The account cannot sign in here as it is (an expired password that
    /// must be changed, a second factor...): what PAM said.
    Refused(String),
    /// greetd or the session could not be reached or started.
    Broken(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::WrongPassword => f.write_str("The password is not correct. Try again."),
            Failure::Refused(why) | Failure::Broken(why) => f.write_str(why),
        }
    }
}

fn write_message(stream: &mut impl Write, request: &Request) -> io::Result<()> {
    let body = serde_json::to_vec(request)?;
    let length = u32::try_from(body.len()).map_err(io::Error::other)?;
    stream.write_all(&length.to_ne_bytes())?;
    stream.write_all(&body)?;
    stream.flush()
}

fn read_message(stream: &mut impl Read) -> io::Result<Response> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_ne_bytes(length) as usize;
    if length > 1 << 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "greetd sent an oversized message",
        ));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// One conversation with greetd.
pub struct Client<S> {
    stream: S,
}

impl Client<UnixStream> {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        Ok(Client::new(UnixStream::connect(socket)?))
    }
}

impl<S: Read + Write> Client<S> {
    pub fn new(stream: S) -> Self {
        Client { stream }
    }

    fn ask(&mut self, request: &Request) -> Result<Response, Failure> {
        write_message(&mut self.stream, request)
            .and_then(|()| read_message(&mut self.stream))
            .map_err(|e| Failure::Broken(format!("The sign-in service did not answer: {e}")))
    }

    /// Ends a half-done sign-in, so the next one can begin.
    fn cancel(&mut self) {
        let _ = self.ask(&Request::CancelSession);
    }

    /// Checks `password` for `username`, then has greetd start `cmd` with
    /// `env` as that user once this greeter exits. Messages PAM shows on
    /// the way (such as "your password expires in 3 days") go to `notice`.
    pub fn sign_in(
        &mut self,
        username: &str,
        password: &str,
        cmd: Vec<String>,
        env: Vec<String>,
        mut notice: impl FnMut(&str),
    ) -> Result<(), Failure> {
        let mut answered = false;
        let mut response = self.ask(&Request::CreateSession {
            username: username.to_string(),
        })?;
        loop {
            response = match response {
                Response::Success => break,
                Response::AuthMessage {
                    auth_message_type: AuthMessageType::Secret | AuthMessageType::Visible,
                    auth_message,
                } => {
                    if answered {
                        // A second question, such as "New password:" for an
                        // expired one, needs a text console.
                        self.cancel();
                        return Err(Failure::Refused(format!(
                            "{} Sign in on a text console (Ctrl+Alt+F2) to answer it.",
                            auth_message.trim()
                        )));
                    }
                    answered = true;
                    self.ask(&Request::PostAuthMessageResponse {
                        response: Some(password.to_string()),
                    })?
                }
                Response::AuthMessage { auth_message, .. } => {
                    notice(auth_message.trim());
                    self.ask(&Request::PostAuthMessageResponse { response: None })?
                }
                Response::Error {
                    error_type: ErrorType::AuthError,
                    ..
                } => {
                    self.cancel();
                    return Err(Failure::WrongPassword);
                }
                Response::Error { description, .. } => {
                    self.cancel();
                    return Err(Failure::Refused(description));
                }
            };
        }
        match self.ask(&Request::StartSession { cmd, env })? {
            Response::Success => Ok(()),
            Response::Error { description, .. } => {
                self.cancel();
                Err(Failure::Broken(format!(
                    "The session could not be started: {description}"
                )))
            }
            Response::AuthMessage { .. } => {
                self.cancel();
                Err(Failure::Broken(
                    "The sign-in service asked something unexpected.".into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread;

    /// A stand-in for greetd that answers each request with the next of
    /// `script` and returns the requests it received, as JSON.
    fn fake_greetd(script: Vec<&'static str>) -> (UnixStream, thread::JoinHandle<Vec<String>>) {
        let (greeter, mut server) = UnixStream::pair().unwrap();
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in script {
                let mut length = [0u8; 4];
                if server.read_exact(&mut length).is_err() {
                    break;
                }
                let mut body = vec![0u8; u32::from_ne_bytes(length) as usize];
                server.read_exact(&mut body).unwrap();
                seen.push(String::from_utf8(body).unwrap());
                server
                    .write_all(&(reply.len() as u32).to_ne_bytes())
                    .unwrap();
                server.write_all(reply.as_bytes()).unwrap();
            }
            seen
        });
        (greeter, handle)
    }

    const SUCCESS: &str = r#"{"type":"success"}"#;
    const PASSWORD: &str =
        r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: "}"#;

    fn sign_in(script: Vec<&'static str>) -> (Result<(), Failure>, Vec<String>, Vec<String>) {
        let (stream, server) = fake_greetd(script);
        let mut notices = Vec::new();
        let result = Client::new(stream).sign_in(
            "alice",
            "s3cret",
            vec!["seglinux-session".into()],
            vec!["XDG_SESSION_TYPE=wayland".into()],
            |n| notices.push(n.to_string()),
        );
        (result, server.join().unwrap(), notices)
    }

    #[test]
    fn signs_in_with_the_password() {
        let (result, seen, _) = sign_in(vec![PASSWORD, SUCCESS, SUCCESS]);
        assert_eq!(result, Ok(()));
        assert_eq!(
            seen,
            [
                r#"{"type":"create_session","username":"alice"}"#,
                r#"{"type":"post_auth_message_response","response":"s3cret"}"#,
                r#"{"type":"start_session","cmd":["seglinux-session"],"env":["XDG_SESSION_TYPE=wayland"]}"#,
            ]
        );
    }

    #[test]
    fn passes_on_information_from_pam() {
        let info = r#"{"type":"auth_message","auth_message_type":"info","auth_message":"Your password expires in 3 days. "}"#;
        let (result, seen, notices) = sign_in(vec![PASSWORD, info, SUCCESS, SUCCESS]);
        assert_eq!(result, Ok(()));
        assert_eq!(notices, ["Your password expires in 3 days."]);
        assert_eq!(
            seen[2],
            r#"{"type":"post_auth_message_response","response":null}"#
        );
    }

    #[test]
    fn a_wrong_password_cancels_the_attempt() {
        let denied = r#"{"type":"error","error_type":"auth_error","description":"pam_authenticate: AUTH_ERR"}"#;
        let (result, seen, _) = sign_in(vec![PASSWORD, denied, SUCCESS]);
        assert_eq!(result, Err(Failure::WrongPassword));
        assert_eq!(seen.last().unwrap(), r#"{"type":"cancel_session"}"#);
    }

    #[test]
    fn a_second_question_is_not_answered_with_the_password() {
        let new_password = r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"New password: "}"#;
        let (result, seen, _) = sign_in(vec![PASSWORD, new_password, SUCCESS]);
        assert!(
            matches!(&result, Err(Failure::Refused(m)) if m.starts_with("New password:")),
            "{result:?}"
        );
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2], r#"{"type":"cancel_session"}"#);
    }

    #[test]
    fn a_session_that_cannot_start_is_reported() {
        let error = r#"{"type":"error","error_type":"error","description":"exec failed"}"#;
        let (result, _, _) = sign_in(vec![PASSWORD, SUCCESS, error, SUCCESS]);
        assert_eq!(
            result,
            Err(Failure::Broken(
                "The session could not be started: exec failed".into()
            ))
        );
    }

    #[test]
    fn a_vanished_greetd_is_an_error_not_a_hang() {
        let (result, _, _) = sign_in(vec![]);
        assert!(matches!(result, Err(Failure::Broken(_))), "{result:?}");
    }
}
