use dbus::{
    arg::{RefArg, Variant},
    nonblock::{Proxy, SyncConnection},
    Path,
};
use futures::{channel::mpsc, SinkExt, Stream, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};
use uuid::Uuid;

use crate::{
    adapter, Address, AddressType, Error, Modalias, PropertyEvent, Result, SERVICE_NAME, TIMEOUT,
};

pub(crate) const INTERFACE: &str = "org.bluez.Device1";

/// Interface to a Bluetooth device.
#[derive(Clone)]
pub struct Device {
    connection: Arc<SyncConnection>,
    dbus_path: Path<'static>,
    adapter_name: Arc<String>,
    address: Address,
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Device {{ adapter_name: {}, address: {} }}",
            self.adapter_name(),
            self.address()
        )
    }
}

impl Device {
    /// Create Bluetooth device interface for device of specified address connected to specified adapater.
    pub(crate) fn new(
        connection: Arc<SyncConnection>,
        adapter_name: Arc<String>,
        address: Address,
    ) -> Result<Self> {
        Ok(Self {
            connection,
            dbus_path: Self::dbus_path(&*adapter_name, address)?,
            adapter_name,
            address,
        })
    }

    fn proxy(&self) -> Proxy<'_, &SyncConnection> {
        Proxy::new(SERVICE_NAME, &self.dbus_path, TIMEOUT, &*self.connection)
    }

    pub(crate) fn dbus_path(adapter_name: &str, address: Address) -> Result<Path<'static>> {
        Path::new(format!(
            "{}{}/dev_{}",
            adapter::PREFIX,
            adapter_name,
            address.to_string().replace(':', "_")
        ))
        .map_err(|_| Error::InvalidName((*adapter_name).to_string()))
    }

    pub(crate) fn parse_dbus_path(path: &Path<'static>) -> Option<(String, Address)> {
        match path.strip_prefix(adapter::PREFIX) {
            Some(adapter_dev) => match adapter_dev.split("/dev_").collect::<Vec<_>>().as_slice() {
                &[adapater, dev] => match dev.replace('_', ":").parse() {
                    Ok(addr) => (Some((adapater.to_string(), addr))),
                    Err(_) => None,
                },
                _ => None,
            },
            None => None,
        }
    }

    /// The Bluetooth adapter name.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// The Bluetooth device address of the remote device.
    pub fn address(&self) -> Address {
        self.address
    }

    //     pub async fn get_addata(&self) -> Result<BluetoothAdvertisingData<'_>> {
    //         let addata = bluetooth_utils::list_addata_2(&self.session.connection(), &self.object_path).await?;
    //
    //         if addata.is_empty() {
    //             return Err(Box::from("No addata found."));
    //         }
    //         Ok(BluetoothAdvertisingData::new(&self.session, &addata[0]))
    //     }

    // pub async fn get_gatt_services(&self) -> Result<Vec<String>> {
    //     bluetooth_utils::list_services(&self.session.connection(), &self.object_path).await
    // }

    /// Streams device property changes.
    pub async fn changes(&self) -> Result<impl Stream<Item = DeviceChanged>> {
        let mut events =
            PropertyEvent::stream(self.connection.clone(), self.dbus_path.clone()).await?;

        let (mut tx, rx) = mpsc::unbounded();
        let address = self.address;
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                for property in DeviceProperty::from_prop_map(event.changed) {
                    if tx.send(DeviceChanged { address, property }).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    dbus_interface!(INTERFACE);

    // ===========================================================================================
    // Methods
    // ===========================================================================================

    /// This is a generic method to connect any profiles
    /// the remote device supports that can be connected
    /// to and have been flagged as auto-connectable on
    /// our side.
    ///
    /// If only subset of profiles is already
    /// connected it will try to connect currently disconnected
    /// ones.
    ///
    /// If at least one profile was connected successfully this
    /// method will indicate success.
    ///
    /// For dual-mode devices only one bearer is connected at
    /// time, the conditions are in the following order:
    ///
    ///     1. Connect the disconnected bearer if already
    ///     connected.
    ///
    ///     2. Connect first the bonded bearer. If no
    ///     bearers are bonded or both are skip and check
    ///     latest seen bearer.
    ///
    ///     3. Connect last seen bearer, in case the
    ///     timestamps are the same BR/EDR takes
    ///     precedence.
    pub async fn connect(&self) -> Result<()> {
        self.call_method("Connect", ()).await
    }

    /// This method gracefully disconnects all connected
    /// profiles and then terminates low-level ACL connection.
    ///
    /// ACL connection will be terminated even if some profiles
    /// were not disconnected properly e.g. due to misbehaving
    /// device.
    ///
    /// This method can be also used to cancel a preceding
    /// Connect call before a reply to it has been received.
    ///
    /// For non-trusted devices connected over LE bearer calling
    /// this method will disable incoming connections until
    /// Connect method is called again.
    pub async fn disconnect(&self) -> Result<()> {
        self.call_method("Disconnect", ()).await
    }

    /// This method connects a specific profile of this
    /// device. The UUID provided is the remote service
    /// UUID for the profile.
    pub async fn connect_profile(&self, uuid: &Uuid) -> Result<()> {
        self.call_method("ConnectProfile", (uuid.to_string(),))
            .await
    }

    /// This method disconnects a specific profile of
    /// this device.
    ///
    /// The profile needs to be registered
    /// client profile.
    ///
    /// There is no connection tracking for a profile, so
    /// as long as the profile is registered this will always
    /// succeed.
    pub async fn disconnect_profile(&self, uuid: &Uuid) -> Result<()> {
        self.call_method("DisconnectProfile", (uuid.to_string(),))
            .await
    }

    /// This method will connect to the remote device,
    /// initiate pairing and then retrieve all SDP records
    /// (or GATT primary services).
    ///
    /// If the application has registered its own agent,
    /// then that specific agent will be used. Otherwise
    /// it will use the default agent.
    ///
    /// Only for applications like a pairing wizard it
    /// would make sense to have its own agent. In almost
    /// all other cases the default agent will handle
    /// this just fine.
    ///
    /// In case there is no application agent and also
    /// no default agent present, this method will fail.
    pub async fn pair(&self) -> Result<()> {
        self.call_method("Pair", ()).await
    }

    /// This method can be used to cancel a pairing
    /// operation initiated by the Pair method.
    pub async fn cancel_pairing(&self) -> Result<()> {
        self.call_method("CancelPairing", ()).await
    }
}

define_properties!(
    Device, DeviceProperty => {
        /// The Bluetooth remote name.
        ///
        /// This value can not be
        ///	changed. Use the Alias property instead.
        ///
        ///	This value is only present for completeness. It is
        ///	better to always use the Alias property when
        ///	displaying the devices name.
        ///
        ///	If the Alias property is unset, it will reflect
        ///	this value which makes it more convenient.
        property(
            Name, String,
            dbus: ("Name", String, OPTIONAL),
            get: (name, v => {v.to_owned()}),
        );

        /// The Bluetooth device Address Type.
        ///
        /// For dual-mode and
        /// BR/EDR only devices this defaults to "public". Single
        /// mode LE devices may have either value. If remote device
        /// uses privacy than before pairing this represents address
        /// type used for connection and Identity Address after
        /// pairing.
        property(
            AddressType, AddressType,
            dbus: ("AddressType", String, MANDATORY),
            get: (address_type, v => {v.parse()?}),
        );

        /// Proposed icon name according to the freedesktop.org
        /// icon naming specification.
        property(
            Icon, String,
            dbus: ("Icon", String, OPTIONAL),
            get: (icon, v => {v.to_owned()}),
        );

        ///	The Bluetooth class of device of the remote device.
        property(
            Class, u32,
            dbus: ("Class", u32, OPTIONAL),
            get: (class, v => {v.to_owned()}),
        );

        ///	External appearance of device, as found on GAP service.
        property(
            Appearance, u32,
            dbus: ("Appearance", u32, OPTIONAL),
            get: (appearance, v => {v.to_owned()}),
        );

        ///	List of 128-bit UUIDs that represents the available
        /// remote services.
        property(
            Uuids, HashSet<Uuid>,
            dbus: ("UUIDs", Vec<String>, OPTIONAL),
            get: (uuids, v => {
                v
                .into_iter()
                .map(|uuid| {
                    uuid.parse()
                        .map_err(|_| Error::InvalidUuid(uuid.to_string()))
                })
                .collect::<Result<HashSet<Uuid>>>()?
            }),
        );

        ///	Indicates if the remote device is paired.
        property(
            Paired, bool,
            dbus: ("Paired", bool, MANDATORY),
            get: (is_paired, v => {v.to_owned()}),
        );

        ///	Indicates if the remote device is paired.
        property(
            Connected, bool,
            dbus: ("Connected", bool, MANDATORY),
            get: (is_connected, v => {v.to_owned()}),
        );

        ///	Indicates if the remote is seen as trusted. This
        /// setting can be changed by the application.
        property(
            Trusted, bool,
            dbus: ("Trusted", bool, MANDATORY),
            get: (is_trusted, v => {v.to_owned()}),
            set: (set_trusted, v => {v}),
        );

        /// If set to true any incoming connections from the
        /// device will be immediately rejected.
        ///
        /// Any device
        /// drivers will also be removed and no new ones will
        /// be probed as long as the device is blocked.
        property(
            Blocked, bool,
            dbus: ("Blocked", bool, MANDATORY),
            get: (is_blocked, v => {v.to_owned()}),
            set: (set_blocked, v => {v}),
        );

        /// If set to true this device will be allowed to wake the
        /// host from system suspend.
        property(
            WakeAllowed, bool,
            dbus: ("WakeAllowed", bool, MANDATORY),
            get: (is_wake_allowed, v => {v.to_owned()}),
            set: (set_wake_allowed, v => {v}),
        );

        /// The name alias for the remote device.
        ///
        /// The alias can
        /// be used to have a different friendly name for the
        /// remote device.
        ///
        /// In case no alias is set, it will return the remote
        /// device name. Setting an empty string as alias will
        /// convert it back to the remote device name.
        ///
        /// When resetting the alias with an empty string, the
        /// property will default back to the remote name.
        property(
            Alias, String,
            dbus: ("Alias", String, MANDATORY),
            get: (alias, v => {v.to_owned()}),
            set: (set_alias, v => {v}),
        );

        /// Set to true if the device only supports the pre-2.1
        /// pairing mechanism.
        ///
        /// This property is useful during
        /// device discovery to anticipate whether legacy or
        /// simple pairing will occur if pairing is initiated.
        ///
        /// Note that this property can exhibit false-positives
        /// in the case of Bluetooth 2.1 (or newer) devices that
        /// have disabled Extended Inquiry Response support.
        property(
            LegacyPairing, bool,
            dbus: ("LegacyPairing", bool, MANDATORY),
            get: (is_legacy_pairing, v => {v.to_owned()}),
        );

        /// Remote Device ID information in modalias format
        /// used by the kernel and udev.
        property(
            Modalias, Modalias,
            dbus: ("Modalias", String, OPTIONAL),
            get: (modalias, v => { v.parse()? }),
        );

        /// Received Signal Strength Indicator of the remote
        ///	device (inquiry or advertising).
        property(
            Rssi, i16,
            dbus: ("RSSI", i16, OPTIONAL),
            get: (rssi, v => {v.to_owned()}),
        );

        /// Advertised transmitted power level (inquiry or
        /// advertising).
        property(
            TxPower, i16,
            dbus: ("TxPower", i16, OPTIONAL),
            get: (tx_power, v => {v.to_owned()}),
        );

        /// Manufacturer specific advertisement data.
        ///
        /// Keys are
        /// 16 bits Manufacturer ID followed by its byte array
        /// value.
        property(
            ManufacturerData, HashMap<u16, Vec<u8>>,
            dbus: ("ManufacturerData", HashMap<u16, Variant<Box<dyn RefArg  + 'static>>>, OPTIONAL),
            get: (manufacturer_data, m => {
                let mut mt: HashMap<u16, Vec<u8>> = HashMap::new();
                for (k,v) in m {
                    match dbus::arg::cast(&v.0).cloned() {
                        Some(v) => {
                            mt.insert(*k, v);
                        }
                        None => (),
                    }
                }
                mt
            }),
        );

        /// Service advertisement data.
        ///
        /// Keys are the UUIDs followed by its byte array value.
        property(
            ServiceData, HashMap<Uuid, Vec<u8>>,
            dbus: ("ServiceData", HashMap<String, Variant<Box<dyn RefArg  + 'static>>>, OPTIONAL),
            get: (service_data, m => {
                let mut mt: HashMap<Uuid, Vec<u8>> = HashMap::new();
                for (k,v) in m {
                    match (k.parse(), dbus::arg::cast(&v.0).cloned()) {
                        (Ok(k), Some(v)) => {
                            mt.insert(k, v);
                        }
                        _ => (),
                    }
                }
                mt
            }),
        );

        /// Indicate whether or not service discovery has been
        /// resolved.
        property(
            ServicesResolved, bool,
            dbus: ("ServicesResolved", bool, MANDATORY),
            get: (is_services_resolved, v => {v.to_owned()}),
        );

        /// The Advertising Data Flags of the remote device.
        property(
            AdvertisingFlags, Vec<u8>,
            dbus: ("AdvertisingFlags", Vec<u8>, MANDATORY),
            get: (advertising_flags, v => {v.to_owned()}),
        );

        /// The Advertising Data of the remote device.
        ///
        /// Note: Only types considered safe to be handled by
        /// application are exposed.
        property(
            AdvertisingData, HashMap<u8, Vec<u8>>,
            dbus: ("AdvertisingData", HashMap<u8, Vec<u8>>, MANDATORY),
            get: (advertising_data, v => {v.to_owned()}),
        );
    }
);

/// Bluetooth device property change event.
#[derive(Debug, Clone)]
pub struct DeviceChanged {
    /// Bluetooth address of changed Bluetooth device.
    pub address: Address,
    /// Changed property.
    pub property: DeviceProperty,
}
