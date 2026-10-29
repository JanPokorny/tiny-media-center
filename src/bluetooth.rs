// The BlueZ pairing agent and the Bluetooth connection notices, on the system
// bus. Nobody can type or compare a code with a remote, so pairing requests
// are accepted while the TV itself pairs (the Pair new screen is open, or a
// pair started from it runs) and rejected otherwise.

use crate::overlay;
use std::collections::HashMap;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::Duration;
use zbus::connection::Builder;
use zbus::export::futures_core::Stream;
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream, ObjectPath, OwnedVariant};

const AGENT_PATH: &str = "/org/tinymediacenter/agent";
const BLUEZ: &str = "org.bluez";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

static PAIRING: AtomicUsize = AtomicUsize::new(0);
// A device connected or disconnected (a few seconds ago, so its audio output
// has had time to come or go): the Home audio label may be stale.
pub static CHANGED: AtomicBool = AtomicBool::new(false);

// Pairing requests are accepted while one of these lives.
pub struct Pairing;

impl Pairing {
    pub fn start() -> Pairing {
        PAIRING.fetch_add(1, Relaxed);
        Pairing
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        PAIRING.fetch_sub(1, Relaxed);
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected,
}

fn check(allowed: bool) -> Result<(), Error> {
    if allowed { Ok(()) } else { Err(Error::Rejected) }
}

fn pairing() -> Result<(), Error> {
    check(PAIRING.load(Relaxed) > 0)
}

// Shown for audio and input devices only: other kinds (phones) connect for
// their own reasons.
fn notice(alias: &str, icon: &str, connected: bool) -> Option<String> {
    let state = if connected { "connected" } else { "disconnected" };
    (icon.starts_with("audio-") || icon.starts_with("input-")).then(|| format!("{alias} {state}"))
}

async fn device(conn: &Connection, path: &ObjectPath<'_>) -> zbus::Result<HashMap<String, OwnedVariant>> {
    let reply = conn.call_method(Some(BLUEZ), path, Some(PROPERTIES), "GetAll", &("org.bluez.Device1",)).await?;
    reply.body().deserialize()
}

fn get<'a, T: TryFrom<&'a OwnedVariant>>(props: &'a HashMap<String, OwnedVariant>, key: &str) -> Option<T> {
    props.get(key).and_then(|v| T::try_from(v).ok())
}

struct Agent;

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    fn cancel(&self) {}

    fn request_pin_code(&self, _device: ObjectPath<'_>) -> Result<String, Error> {
        pairing().map(|()| "0000".into())
    }

    fn request_passkey(&self, _device: ObjectPath<'_>) -> Result<u32, Error> {
        pairing().map(|()| 0)
    }

    fn request_confirmation(&self, _device: ObjectPath<'_>, _passkey: u32) -> Result<(), Error> {
        pairing()
    }

    fn request_authorization(&self, _device: ObjectPath<'_>) -> Result<(), Error> {
        pairing()
    }

    fn display_pin_code(&self, _device: ObjectPath<'_>, pincode: String) {
        overlay::notify("Bluetooth", format!("Pairing code {pincode}"));
    }

    fn display_passkey(&self, _device: ObjectPath<'_>, passkey: u32, _entered: u16) {
        overlay::notify("Bluetooth", format!("Pairing code {passkey:06}"));
    }

    async fn authorize_service(
        &self,
        #[zbus(connection)] conn: &Connection,
        device: ObjectPath<'_>,
        _uuid: String,
    ) -> Result<(), Error> {
        let props = self::device(conn, &device).await?;
        check(get(&props, "Paired") == Some(true) || get(&props, "Trusted") == Some(true))
    }
}

// Registers the agent every 10 s: bluetoothd forgets it when it restarts, and
// says AlreadyExists while it remembers. The first failure in a row is logged.
async fn register(conn: Connection) {
    let runtime = zruntime::SharedRuntime::current().expect("runtime");
    let mut failed = false;
    loop {
        let (manager, agent) = (Some("org.bluez.AgentManager1"), ObjectPath::from_static_str_unchecked(AGENT_PATH));
        let result = async {
            conn.call_method(Some(BLUEZ), "/org/bluez", manager, "RegisterAgent", &(&agent, "NoInputNoOutput")).await?;
            conn.call_method(Some(BLUEZ), "/org/bluez", manager, "RequestDefaultAgent", &(&agent,)).await
        };
        match result.await {
            Err(zbus::Error::MethodError(name, ..)) if name == "org.bluez.Error.AlreadyExists" => {}
            Err(e) if !failed => {
                eprintln!("bluetooth agent: {e}");
                failed = true;
            }
            result => failed = result.is_err(),
        }
        runtime.sleep(Duration::from_secs(10)).await;
    }
}

async fn watch(conn: &Connection) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path_namespace("/org/bluez")
        .arg(0, "org.bluez.Device1")
        .build()?;
    let mut stream = MessageStream::for_match_rule(rule, conn, None).await?;
    while let Some(msg) = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
        let Ok(msg) = msg else { continue };
        let Ok((_, changed, _)) = msg.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else {
            continue;
        };
        let header = msg.header();
        let (Some(connected), Some(path)) = (get(&changed, "Connected"), header.path()) else { continue };
        let Ok(props) = device(conn, path).await else { continue };
        let (alias, icon) = (get(&props, "Alias").unwrap_or(""), get(&props, "Icon").unwrap_or(""));
        if let Some(text) = notice(alias, icon, connected) {
            overlay::notify("Bluetooth", text);
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(3));
                CHANGED.store(true, Relaxed);
            });
        }
    }
    Ok(())
}

// zbus drives the connection from this thread's block_on.
pub fn serve() {
    std::thread::spawn(|| {
        zbus::block_on(async {
            let conn = match Builder::system().serve_at(AGENT_PATH, Agent).build().await {
                Ok(conn) => conn,
                Err(e) => return eprintln!("bluetooth: {e}"),
            };
            zruntime::spawn(register(conn.clone())).detach();
            if let Err(e) = watch(&conn).await {
                eprintln!("bluetooth notices: {e}");
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_requests_need_the_pair_screen() {
        let device = || ObjectPath::from_static_str_unchecked("/org/bluez/hci0/dev_00");
        let agent = Agent;
        assert!(agent.request_confirmation(device(), 123456).is_err());
        assert!(agent.request_pin_code(device()).is_err());
        let (screen, pair) = (Pairing::start(), Pairing::start());
        drop(screen); // the Pair new screen closes while its pair still runs
        assert!(agent.request_confirmation(device(), 123456).is_ok());
        assert!(agent.request_authorization(device()).is_ok());
        assert_eq!(agent.request_pin_code(device()).ok().as_deref(), Some("0000"));
        assert_eq!(agent.request_passkey(device()).ok(), Some(0));
        drop(pair);
        assert!(matches!(agent.request_authorization(device()), Err(Error::Rejected)));
        assert!(agent.request_passkey(device()).is_err());
    }

    #[test]
    fn notices_for_audio_and_input_devices() {
        assert_eq!(notice("JBL Charge 5", "audio-card", true).as_deref(), Some("JBL Charge 5 connected"));
        assert_eq!(notice("MX Anywhere 3", "input-mouse", false).as_deref(), Some("MX Anywhere 3 disconnected"));
        assert_eq!(notice("Pixel", "phone", true), None);
        assert_eq!(notice("Unknown", "", true), None);
    }
}
