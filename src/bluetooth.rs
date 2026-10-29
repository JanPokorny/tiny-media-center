// The BlueZ pairing agent and the Bluetooth connection notices, on the system
// bus. Nobody can type a code with a remote, so the agent registers as
// NoInputNoOutput (Just Works pairing needs no answer from it) and only
// accepts the requests left (legacy PIN, authorization) for the device the TV
// itself is pairing.

use crate::overlay;
use std::collections::HashMap;
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;
use zbus::connection::Builder;
use zbus::export::futures_core::Stream;
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream, ObjectPath, OwnedVariant};

const AGENT_PATH: &str = "/org/tinymediacenter/agent";
const BLUEZ: &str = "org.bluez";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

// "/dev_<MAC>" of the device being paired: its object path's end, whichever
// adapter it is on.
static PAIRING: Mutex<Option<String>> = Mutex::new(None);
// A device connected or disconnected (a few seconds ago, so its audio output
// has had time to come or go): the Home audio label may be stale.
pub static CHANGED: AtomicBool = AtomicBool::new(false);

// Pairing requests from the device are accepted while this lives.
pub struct Pairing;

impl Pairing {
    pub fn start(mac: &str) -> Pairing {
        *PAIRING.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("/dev_{}", mac.replace(':', "_")));
        Pairing
    }
}

impl Drop for Pairing {
    fn drop(&mut self) {
        *PAIRING.lock().unwrap_or_else(|e| e.into_inner()) = None;
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

fn pairing(device: &ObjectPath<'_>) -> Result<(), Error> {
    let pairing = PAIRING.lock().unwrap_or_else(|e| e.into_inner());
    check(pairing.as_ref().is_some_and(|end| device.as_str().ends_with(end.as_str())))
}

// Shown for audio devices only: the others (controllers, phones) come and go
// on their own, such as a controller powering off during a film.
fn notice(alias: &str, icon: &str, connected: bool) -> Option<String> {
    let state = if connected { "connected" } else { "disconnected" };
    icon.starts_with("audio-").then(|| format!("{alias} {state}"))
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

    // RequestPasskey, RequestConfirmation and DisplayPasskey never come to a
    // NoInputNoOutput agent; unknown methods are rejections anyway.
    fn request_pin_code(&self, device: ObjectPath<'_>) -> Result<String, Error> {
        pairing(&device).map(|()| "0000".into())
    }

    fn request_authorization(&self, device: ObjectPath<'_>) -> Result<(), Error> {
        pairing(&device)
    }

    fn display_pin_code(&self, _device: ObjectPath<'_>, pincode: String) {
        overlay::notify("Bluetooth", format!("Pairing code {pincode}"));
    }

    async fn authorize_service(
        &self,
        #[zbus(connection)] conn: &Connection,
        device: ObjectPath<'_>,
        _uuid: String,
    ) -> Result<(), Error> {
        let props = self::device(conn, &device).await?;
        check(get(&props, "Paired") == Some(true))
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

// Serves the agent and shows the notices until the connection fails.
async fn run() -> zbus::Result<()> {
    let conn = Builder::system().serve_at(AGENT_PATH, Agent).build().await?;
    // Cancelled when this returns, so the connection closes with it.
    let _register = zruntime::spawn(register(conn.clone()));
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(BLUEZ)
        .interface(PROPERTIES)
        .member("PropertiesChanged")
        .path_namespace("/org/bluez")
        .arg(0, "org.bluez.Device1")
        .build()?;
    let mut stream = MessageStream::for_match_rule(rule, &conn, None).await?;
    while let Some(msg) = poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
        let Ok(msg) = msg else { continue };
        let Ok((_, changed, _)) = msg.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else {
            continue;
        };
        let header = msg.header();
        let (Some(connected), Some(path)) = (get(&changed, "Connected"), header.path()) else { continue };
        let Ok(props) = device(&conn, path).await else { continue };
        let (alias, icon) = (get(&props, "Alias").unwrap_or(""), get(&props, "Icon").unwrap_or(""));
        if let Some(text) = notice(alias, icon, connected) {
            overlay::notify("Bluetooth", text);
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(3));
                CHANGED.store(true, Relaxed);
            });
        }
    }
    Err(zbus::Error::Failure("system bus connection closed".into()))
}

// zbus drives the connection from this thread's block_on. Starts over 10 s
// after a failure.
pub fn serve() {
    std::thread::spawn(|| {
        zbus::block_on(async {
            let runtime = zruntime::SharedRuntime::current().expect("runtime");
            loop {
                if let Err(e) = run().await {
                    eprintln!("bluetooth: {e}");
                }
                runtime.sleep(Duration::from_secs(10)).await;
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_requests_need_the_device_being_paired() {
        let (jbl, other) = ("/org/bluez/hci0/dev_11_22_33_44_55_66", "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF");
        let path = ObjectPath::from_static_str_unchecked;
        let agent = Agent;
        assert!(agent.request_authorization(path(jbl)).is_err());
        assert!(agent.request_pin_code(path(jbl)).is_err());
        let pairing = Pairing::start("11:22:33:44:55:66");
        assert!(agent.request_authorization(path(jbl)).is_ok());
        assert_eq!(agent.request_pin_code(path(jbl)).ok().as_deref(), Some("0000"));
        assert!(matches!(agent.request_authorization(path(other)), Err(Error::Rejected)));
        assert!(agent.request_pin_code(path(other)).is_err());
        drop(pairing);
        assert!(agent.request_authorization(path(jbl)).is_err());
    }

    #[test]
    fn notices_for_audio_devices_only() {
        assert_eq!(notice("JBL Charge 5", "audio-card", true).as_deref(), Some("JBL Charge 5 connected"));
        assert_eq!(notice("CMF Buds", "audio-headset", false).as_deref(), Some("CMF Buds disconnected"));
        assert_eq!(notice("Xbox Wireless Controller", "input-gaming", false), None);
        assert_eq!(notice("Pixel", "phone", true), None);
        assert_eq!(notice("Unknown", "", true), None);
    }
}
