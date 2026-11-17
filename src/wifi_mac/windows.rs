use super::{profile, Backend, Error, Snapshot, Target};
use crate::network::{self, NetworkConnection, NetworkIdentity};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::HANDLE,
        NetworkManagement::{
            IpHelper::{GetIfEntry2, MIB_IF_ROW2},
            WiFi::{
                wlan_interface_state_connected, wlan_intf_opcode_current_connection,
                WlanCloseHandle, WlanFreeMemory, WlanGetProfile, WlanGetProfileCustomUserData,
                WlanOpenHandle, WlanQueryInterface, WlanSetProfile, WLAN_CONNECTION_ATTRIBUTES,
                WLAN_PROFILE_GROUP_POLICY, WLAN_PROFILE_USER,
            },
        },
    },
};

pub(super) struct Native<'a> {
    pub identity: &'a NetworkIdentity,
    pub connection: NetworkConnection,
}

struct Session(HANDLE);
impl Session {
    fn open() -> Result<Self, Error> {
        let mut handle = null_mut();
        let mut version = 0;
        check(unsafe { WlanOpenHandle(2, null(), &mut version, &mut handle) })?;
        Ok(Self(handle))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, null());
        }
    }
}
struct Memory(*mut c_void);
impl Drop for Memory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WlanFreeMemory(self.0);
            }
        }
    }
}

fn check(code: u32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        5 => Err(Error::Permission),
        _ => Err(Error::Query),
    }
}

fn adapter_id(guid: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

struct Profile {
    snapshot: Snapshot,
    xml: String,
    flags: u32,
    guid: GUID,
}

impl Native<'_> {
    fn read(&self, session: &Session) -> Result<Profile, Error> {
        // Both the saved fingerprint and route must still match before touching a profile.
        if network::matched_connection(self.identity).ok().flatten() != Some(self.connection) {
            return Err(Error::Changed);
        }
        unsafe {
            let mut row: MIB_IF_ROW2 = zeroed();
            row.InterfaceIndex = self.connection.interface_index;
            check(GetIfEntry2(&mut row))?;
            if row.Type != 71
                || row.InterfaceAndOperStatusFlags._bitfield & 1 == 0
                || !adapter_id(&row.InterfaceGuid).eq_ignore_ascii_case(&self.identity.adapter_id)
            {
                return Err(Error::Changed);
            }
            if row.PhysicalAddressLength != 6 {
                return Err(Error::UnknownMac);
            }
            let mut data = null_mut();
            let mut size = 0;
            let result = WlanQueryInterface(
                session.0,
                &row.InterfaceGuid,
                wlan_intf_opcode_current_connection,
                null(),
                &mut size,
                &mut data,
                null_mut(),
            );
            let memory = Memory(data);
            check(result)?;
            if memory.0.is_null() || (size as usize) < size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
                return Err(Error::Query);
            }
            let connection = &*(memory.0 as *const WLAN_CONNECTION_ATTRIBUTES);
            let ssid = &connection.wlanAssociationAttributes.dot11Ssid;
            let len = ssid.uSSIDLength as usize;
            if connection.isState != wlan_interface_state_connected
                || len == 0
                || len > 32
                || self.identity.ssid.as_deref() != Some(&ssid.ucSSID[..len])
            {
                return Err(Error::Changed);
            }
            let name_len = connection
                .strProfileName
                .iter()
                .position(|c| *c == 0)
                .ok_or(Error::Query)?;
            if name_len == 0 {
                return Err(Error::Query);
            }
            let name = &connection.strProfileName[..name_len];
            let target = Target {
                adapter_id: self.identity.adapter_id.clone(),
                interface_index: self.connection.interface_index,
                profile: name.to_vec(),
                ssid: ssid.ucSSID[..len].to_vec(),
            };
            let terminated: Vec<u16> = name.iter().copied().chain(Some(0)).collect();
            let mut xml_ptr = null_mut();
            // Never request WLAN_PROFILE_GET_PLAINTEXT_KEY. Windows returns protected keyMaterial.
            let mut flags = 0;
            let result = WlanGetProfile(
                session.0,
                &row.InterfaceGuid,
                terminated.as_ptr(),
                null(),
                &mut xml_ptr,
                &mut flags,
                null_mut(),
            );
            let xml_memory = Memory(xml_ptr.cast());
            check(result)?;
            if xml_memory.0.is_null() {
                return Err(Error::Query);
            }
            let mut xml_len = 0;
            while xml_len <= 524_288 && *xml_ptr.add(xml_len) != 0 {
                xml_len += 1;
            }
            if xml_len > 524_288 {
                return Err(Error::Query);
            }
            let xml = String::from_utf16(std::slice::from_raw_parts(xml_ptr, xml_len))
                .map_err(|_| Error::Query)?;
            let disabled = profile::randomization_disabled(&xml).map_err(|_| Error::Query)?;
            let mut custom_data = false;
            // WlanSetProfile clears custom user data. Refuse an unsafe overwrite rather than
            // copying or logging that data. Already-disabled profiles do not need this query.
            if !disabled {
                let mut custom_size = 0;
                let mut custom_ptr = null_mut();
                let result = WlanGetProfileCustomUserData(
                    session.0,
                    &row.InterfaceGuid,
                    terminated.as_ptr(),
                    null(),
                    &mut custom_size,
                    &mut custom_ptr,
                );
                let _custom_memory = Memory(custom_ptr.cast());
                if result != 2 {
                    check(result)?;
                }
                custom_data = result == 0 && custom_size != 0;
            }
            let mut current = [0; 6];
            let mut permanent = [0; 6];
            current.copy_from_slice(&row.PhysicalAddress[..6]);
            permanent.copy_from_slice(&row.PermanentPhysicalAddress[..6]);
            Ok(Profile {
                snapshot: Snapshot {
                    target,
                    current,
                    permanent,
                    disabled,
                    managed: flags & WLAN_PROFILE_GROUP_POLICY != 0
                        || flags & !(WLAN_PROFILE_USER | WLAN_PROFILE_GROUP_POLICY) != 0,
                    custom_data,
                },
                xml,
                flags,
                guid: row.InterfaceGuid,
            })
        }
    }
}

impl Backend for Native<'_> {
    fn inspect(&mut self) -> Result<Snapshot, Error> {
        self.read(&Session::open()?).map(|profile| profile.snapshot)
    }
    fn disable(&mut self, expected: &Snapshot) -> Result<(), Error> {
        let session = Session::open()?;
        let profile = self.read(&session)?;
        if profile.snapshot.target != expected.target
            || profile.snapshot.current != expected.current
            || profile.snapshot.permanent != expected.permanent
        {
            return Err(Error::Changed);
        }
        if profile.snapshot.disabled {
            return Ok(());
        }
        if profile.snapshot.managed {
            return Err(Error::Managed);
        }
        if profile.snapshot.custom_data {
            return Err(Error::CustomData);
        }
        let xml = profile::disable_randomization(&profile.xml).map_err(|_| Error::Write)?;
        let wide: Vec<u16> = xml.encode_utf16().chain(Some(0)).collect();
        if network::matched_connection(self.identity).ok().flatten() != Some(self.connection) {
            return Err(Error::Changed);
        }
        let mut reason = 0;
        // Exact profile, original per-user/all-user type, existing ACL retained, no reconnect.
        match unsafe {
            WlanSetProfile(
                session.0,
                &profile.guid,
                profile.flags & WLAN_PROFILE_USER,
                wide.as_ptr(),
                null(),
                1,
                null(),
                &mut reason,
            )
        } {
            0 => Ok(()),
            5 => Err(Error::Permission),
            _ => Err(Error::Write),
        }
    }
}
