// Windows Bluetooth for the hands-free unit: the paired-device list
// (bluetoothapis.dll) and an RFCOMM socket to the phone's Hands-Free Audio
// Gateway service (Winsock, AF_BTH). Both work for an ordinary desktop
// program: no package identity or capability is involved.
//
// Raw Win32 calls on handles and buffers this module owns (each `unsafe`
// says why).
#![allow(unsafe_code)]

use std::mem::size_of;

use ::windows::core::GUID;
use ::windows::Win32::Devices::Bluetooth::{
    BluetoothEnumerateInstalledServices, BluetoothFindDeviceClose, BluetoothFindFirstDevice,
    BluetoothFindNextDevice, AF_BTH, BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
    BTHPROTO_RFCOMM, SOCKADDR_BTH,
};
use ::windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS};
use ::windows::Win32::Networking::WinSock::{
    closesocket, connect, recv, send, shutdown, socket, WSAGetLastError, WSAStartup, SD_BOTH,
    SEND_RECV_FLAGS, SOCKADDR, SOCKET, SOCKET_ERROR, SOCK_STREAM, WSADATA,
};

use crate::{address_from_u64, offers_calls, Gateway};

/// The Hands-Free Audio Gateway service class (0x111F), the phone's side.
const AUDIO_GATEWAY: GUID = GUID::from_u128(0x0000111f_0000_1000_8000_00805f9b34fb);

/// Winsock 2.2, once per process (it's reference-counted; never undone:
/// the process ending cleans up).
pub fn startup() -> Result<(), String> {
    let mut data = WSADATA::default();
    // SAFETY: `data` is a valid, writable WSADATA.
    let r = unsafe { WSAStartup(0x0202, &mut data) };
    if r == 0 {
        Ok(())
    } else {
        Err(format!(
            "Windows networking (Winsock) didn\u{2019}t start (error {r})."
        ))
    }
}

/// The last Winsock error on this thread.
fn last_error() -> i32 {
    // SAFETY: no arguments; reads this thread's error slot.
    unsafe { WSAGetLastError() }.0
}

/// Paired phones offering calls (see [`offers_calls`]).
pub fn paired_gateways() -> Result<Vec<Gateway>, String> {
    let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
        dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
        fReturnAuthenticated: true.into(),
        fReturnRemembered: true.into(),
        fReturnUnknown: false.into(),
        fReturnConnected: false.into(),
        // The paired list only: no inquiry (no radio scan, no waiting).
        fIssueInquiry: false.into(),
        cTimeoutMultiplier: 0,
        ..Default::default()
    };
    let mut info = blank_info();
    // SAFETY: both structs are initialised with their dwSize; `info` is
    // writable.
    let find = match unsafe { BluetoothFindFirstDevice(&params, &mut info) } {
        Ok(f) => f,
        Err(e) if e.code() == ERROR_NO_MORE_ITEMS.to_hresult() => return Ok(Vec::new()),
        Err(e) => {
            return Err(format!(
                "Bluetooth isn\u{2019}t available on this computer (is it on in Windows Settings \u{2192} Bluetooth & devices?): {}",
                e.message()
            ))
        }
    };
    let mut out = Vec::new();
    loop {
        if info.fRemembered.as_bool() || info.fAuthenticated.as_bool() {
            let services = has_gateway_service(&info);
            if offers_calls(info.ulClassofDevice, services) {
                out.push(Gateway {
                    address: address_of(&info),
                    name: name_of(&info),
                    connected: info.fConnected.as_bool(),
                });
            }
        }
        info = blank_info();
        // SAFETY: `find` is open; `info` is initialised and writable.
        if unsafe { BluetoothFindNextDevice(find, &mut info) }.is_err() {
            break;
        }
    }
    // SAFETY: `find` came from BluetoothFindFirstDevice, closed once.
    let _ = unsafe { BluetoothFindDeviceClose(find) };
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn blank_info() -> BLUETOOTH_DEVICE_INFO {
    BLUETOOTH_DEVICE_INFO {
        dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        ..Default::default()
    }
}

fn address_of(info: &BLUETOOTH_DEVICE_INFO) -> String {
    // SAFETY: both union members are plain bytes; ullLong covers them.
    let raw = unsafe { info.Address.Anonymous.ullLong };
    address_from_u64(raw & 0xFFFF_FFFF_FFFF)
}

fn name_of(info: &BLUETOOTH_DEVICE_INFO) -> String {
    let len = info
        .szName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szName.len());
    let name = String::from_utf16_lossy(&info.szName[..len]);
    if name.trim().is_empty() {
        address_of(info)
    } else {
        name
    }
}

/// The device lists the Audio Gateway among the services Windows knows
/// for it.
fn has_gateway_service(info: &BLUETOOTH_DEVICE_INFO) -> bool {
    let mut guids = [GUID::zeroed(); 64];
    let mut n = guids.len() as u32;
    // SAFETY: `info` is a filled device record; `guids` has room for `n`.
    let r = unsafe {
        BluetoothEnumerateInstalledServices(None, info, &mut n, Some(guids.as_mut_ptr()))
    };
    (r == 0 || r == ERROR_MORE_DATA.0)
        && guids[..(n as usize).min(guids.len())].contains(&AUDIO_GATEWAY)
}

/// An RFCOMM connection to a phone's Audio Gateway service; closed when
/// dropped. One thread may read while another writes (Winsock allows it).
pub struct Rfcomm(SOCKET);

impl Rfcomm {
    /// Connect to the Audio Gateway service of the phone at `addr`
    /// (`BTH_ADDR`); Windows looks the channel up over SDP. Blocks (up to
    /// the radio's page timeout, ~5-20 s). The error is Winsock's code.
    pub fn connect(addr: u64) -> Result<Rfcomm, i32> {
        // SAFETY: plain socket creation; the handle is owned by `sock`.
        let s = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) }
            .map_err(|_| last_error())?;
        let sock = Rfcomm(s);
        let sa = SOCKADDR_BTH {
            addressFamily: AF_BTH,
            btAddr: addr,
            serviceClassId: AUDIO_GATEWAY,
            // 0: resolve the channel from serviceClassId.
            port: 0,
        };
        // SAFETY: `sa` is a valid SOCKADDR_BTH, its size passed with it;
        // the socket is open.
        let r = unsafe {
            connect(
                sock.0,
                (&sa as *const SOCKADDR_BTH).cast::<SOCKADDR>(),
                size_of::<SOCKADDR_BTH>() as i32,
            )
        };
        if r == SOCKET_ERROR {
            // Read before `sock` is dropped (closesocket resets it).
            return Err(last_error());
        }
        Ok(sock)
    }

    /// Write all of `bytes`.
    pub fn send_all(&self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            // SAFETY: the socket is open; `bytes` is a valid slice.
            let n = unsafe { send(self.0, bytes, SEND_RECV_FLAGS(0)) };
            if n == SOCKET_ERROR || n <= 0 {
                return Err(std::io::Error::from_raw_os_error(last_error()));
            }
            bytes = &bytes[(n as usize).min(bytes.len())..];
        }
        Ok(())
    }

    /// Read what's there (blocks); 0 when the phone closed the channel.
    pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: the socket is open; `buf` is a valid, writable slice.
        let n = unsafe { recv(self.0, buf, SEND_RECV_FLAGS(0)) };
        if n == SOCKET_ERROR || n < 0 {
            Err(std::io::Error::from_raw_os_error(last_error()))
        } else {
            Ok(n as usize)
        }
    }

    /// End the connection now (a blocked `recv` returns); the handle is
    /// closed when the last owner drops it.
    pub fn shutdown(&self) {
        // SAFETY: the socket is open until drop.
        let _ = unsafe { shutdown(self.0, SD_BOTH) };
    }
}

impl Drop for Rfcomm {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and closed exactly once, here.
        let _ = unsafe { closesocket(self.0) };
    }
}
