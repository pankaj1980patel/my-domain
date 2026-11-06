package com.mydomain.android

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import com.mydomain.android.core.Core
import com.mydomain.android.ffi.BleOut
import java.io.ByteArrayOutputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * my-domain over Bluetooth LE, phone side: advertise the my-domain service
 * and carry the core's frames for computers that connect (the computer's
 * side is crates/md-ble). Used when there's no shared network or no
 * internet; the core prefers any direct link.
 *
 * The service (must match crates/md-ble/src/lib.rs): [SERVICE] with
 * [TO_PHONE] (the computer writes) and [FROM_PHONE] (we notify). A frame
 * goes as chunks: one header byte (1 = last chunk, 0 = more follow), then
 * up to the packet size minus one bytes.
 *
 * Needs BLUETOOTH_ADVERTISE and BLUETOOTH_CONNECT (Android 12+); without
 * them, or with Bluetooth off, [start] does nothing.
 */
@SuppressLint("MissingPermission") // checked in start()
object BleLink {
    private const val TAG = "mydomain_ble"
    val SERVICE: UUID = UUID.fromString("7a3e0001-5f3c-4b8e-9d1a-6d79646f6d61")
    val TO_PHONE: UUID = UUID.fromString("7a3e0002-5f3c-4b8e-9d1a-6d79646f6d61")
    val FROM_PHONE: UUID = UUID.fromString("7a3e0003-5f3c-4b8e-9d1a-6d79646f6d61")
    private val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    private const val MAX_FRAME = 4 * 1024 * 1024

    /** One connected computer. */
    private class Conn(val device: BluetoothDevice) {
        var linkId: Long? = null
        var mtu = 23
        var subscribed = false
        var sending = false
        val out = ArrayDeque<ByteArray>()
        val frame = ByteArrayOutputStream()
        /** A long write (prepare/execute) being put together. */
        val prepared = ByteArrayOutputStream()
    }

    private val conns = ConcurrentHashMap<String, Conn>()
    /** Frames to the core in arrival order, off the Bluetooth threads. */
    private val toCore = Executors.newSingleThreadExecutor()
    private var server: BluetoothGattServer? = null
    private var fromPhone: BluetoothGattCharacteristic? = null
    private var advertising: AdvertiseCallback? = null

    fun start(ctx: Context) {
        if (server != null) return
        if (Sdk.rules(Build.VERSION_CODES.S) &&
            listOf(Manifest.permission.BLUETOOTH_ADVERTISE, Manifest.permission.BLUETOOTH_CONNECT)
                .any { ctx.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        ) {
            Log.i(TAG, "no Bluetooth permission; not advertising")
            return
        }
        val manager = ctx.getSystemService(BluetoothManager::class.java) ?: return
        if (manager.adapter?.isEnabled != true) {
            Log.i(TAG, "Bluetooth is off")
            return
        }
        val s = runCatching { manager.openGattServer(ctx, callback) }.getOrNull() ?: return
        server = s
        val notify = BluetoothGattCharacteristic(
            FROM_PHONE,
            BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ,
        ).apply {
            addDescriptor(
                BluetoothGattDescriptor(
                    CCCD,
                    BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE,
                ),
            )
        }
        fromPhone = notify
        val write = BluetoothGattCharacteristic(
            TO_PHONE,
            BluetoothGattCharacteristic.PROPERTY_WRITE,
            BluetoothGattCharacteristic.PERMISSION_WRITE,
        )
        val service = BluetoothGattService(SERVICE, BluetoothGattService.SERVICE_TYPE_PRIMARY).apply {
            addCharacteristic(write)
            addCharacteristic(notify)
        }
        s.addService(service) // advertising starts in onServiceAdded
        pendingAdvertiser = { advertise(manager) }
    }

    fun stop() {
        // Not started yet (the service is still being added): never start.
        pendingAdvertiser = null
        advertising?.let { cb -> runCatching { advertiser?.stopAdvertising(cb) } }
        advertising = null
        advertiser = null
        ResourceLedger.release(ResourceBook.Kind.BLUETOOTH, "BLE advertising (low power)")
        conns.values.forEach { c -> c.linkId?.let { Core.engine()?.bleClosed(it.toULong()) } }
        conns.clear()
        runCatching { server?.close() }
        server = null
    }

    private var pendingAdvertiser: (() -> Unit)? = null
    private var advertiser: android.bluetooth.le.BluetoothLeAdvertiser? = null

    private fun advertise(manager: BluetoothManager) {
        val adv = manager.adapter?.bluetoothLeAdvertiser ?: return
        advertiser = adv
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true)
            .setTimeout(0)
            .build()
        val data = AdvertiseData.Builder()
            .addServiceUuid(ParcelUuid(SERVICE))
            .setIncludeDeviceName(false)
            .build()
        val cb = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                Log.i(TAG, "advertising")
                ResourceLedger.hold(ResourceBook.Kind.BLUETOOTH, "BLE advertising (low power)")
            }

            override fun onStartFailure(errorCode: Int) {
                Log.w(TAG, "can't advertise: $errorCode")
            }
        }
        advertising = cb
        adv.startAdvertising(settings, data, cb)
    }

    private val callback = object : BluetoothGattServerCallback() {
        override fun onServiceAdded(status: Int, service: BluetoothGattService) {
            if (status == BluetoothGatt.GATT_SUCCESS && server != null) pendingAdvertiser?.invoke()
            pendingAdvertiser = null
        }

        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            when (newState) {
                BluetoothProfile.STATE_CONNECTED -> {
                    conns[device.address] = Conn(device)
                    Log.i(TAG, "computer connected")
                }
                BluetoothProfile.STATE_DISCONNECTED -> {
                    val c = conns.remove(device.address) ?: return
                    c.linkId?.let { id -> toCore.execute { Core.engine()?.bleClosed(id.toULong()) } }
                    Log.i(TAG, "computer disconnected")
                }
            }
        }

        override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
            conns[device.address]?.mtu = mtu
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray,
        ) {
            val c = conns[device.address]
            val ok = c != null && characteristic.uuid == TO_PHONE
            if (ok) {
                if (preparedWrite) c!!.prepared.write(value) else chunkIn(c!!, value)
            }
            if (responseNeeded) {
                val status = if (ok) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_FAILURE
                server?.sendResponse(device, requestId, status, offset, value)
            }
        }

        override fun onExecuteWrite(device: BluetoothDevice, requestId: Int, execute: Boolean) {
            conns[device.address]?.let { c ->
                if (execute) chunkIn(c, c.prepared.toByteArray())
                c.prepared.reset()
            }
            server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray,
        ) {
            val c = conns[device.address]
            if (c != null && descriptor.uuid == CCCD) {
                c.subscribed = value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
                if (c.subscribed && c.linkId == null) open(c)
            }
            if (responseNeeded) {
                server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value)
            }
            pump(c ?: return)
        }

        override fun onNotificationSent(device: BluetoothDevice, status: Int) {
            val c = conns[device.address] ?: return
            synchronized(c) { c.sending = false }
            pump(c)
        }
    }

    /** The computer subscribed: hand the connection to the core. */
    private fun open(c: Conn) {
        val engine = Core.engine() ?: return
        val out = object : BleOut {
            override fun send(frame: ByteArray) {
                val packet = (c.mtu - 3).coerceAtLeast(20)
                synchronized(c) { split(frame, packet).forEach(c.out::addLast) }
                pump(c)
            }
        }
        c.linkId = engine.bleOpened(out).toLong()
    }

    private fun chunkIn(c: Conn, chunk: ByteArray) {
        if (chunk.isEmpty()) return
        if (c.frame.size() + chunk.size - 1 > MAX_FRAME) {
            c.frame.reset()
            return
        }
        c.frame.write(chunk, 1, chunk.size - 1)
        if (chunk[0].toInt() != 1) return
        val frame = c.frame.toByteArray()
        c.frame.reset()
        val id = c.linkId ?: return
        toCore.execute { Core.engine()?.bleFrame(id.toULong(), frame) }
    }

    /** Send the next queued chunk, one notification at a time. */
    private fun pump(c: Conn) {
        val chunk = synchronized(c) {
            if (c.sending || !c.subscribed || c.out.isEmpty()) return
            c.sending = true
            c.out.removeFirst()
        }
        val s = server
        val ch = fromPhone
        val sent = if (s == null || ch == null) {
            false
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            s.notifyCharacteristicChanged(c.device, ch, false, chunk) == BluetoothGatt.GATT_SUCCESS
        } else {
            @Suppress("DEPRECATION")
            ch.value = chunk
            @Suppress("DEPRECATION")
            s.notifyCharacteristicChanged(c.device, ch, false)
        }
        if (!sent) synchronized(c) { c.sending = false }
    }

    private fun split(frame: ByteArray, packet: Int): List<ByteArray> {
        val body = (packet - 1).coerceAtLeast(1)
        if (frame.isEmpty()) return listOf(byteArrayOf(1))
        val parts = (frame.indices step body).map { frame.copyOfRange(it, minOf(it + body, frame.size)) }
        return parts.mapIndexed { i, p -> byteArrayOf(if (i == parts.lastIndex) 1 else 0) + p }
    }
}
