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
import com.mydomain.android.core.ffi
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
        /** Failed tries of the chunk at the head of [out]. */
        var retries = 0
        /** The chunk sent and not yet confirmed by onNotificationSent. */
        var inFlight: ByteArray? = null
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
        // Set before addService: onServiceAdded can come (on a Binder
        // thread) before addService returns, and it found nothing to start,
        // so the phone never advertised.
        pendingAdvertiser.set { advertise(manager) }
        if (!s.addService(service)) { // advertising starts in onServiceAdded
            Log.w(TAG, "can't add the service; not advertising")
            pendingAdvertiser.set(null)
        }
    }

    fun stop() {
        // Not started yet (the service is still being added): never start.
        pendingAdvertiser.set(null)
        advertising?.let { cb -> runCatching { advertiser?.stopAdvertising(cb) } }
        advertising = null
        advertiser = null
        ResourceLedger.release(ResourceBook.Kind.BLUETOOTH, "BLE advertising (low power)")
        conns.values.forEach { c -> c.linkId?.let { ffi("bleClosed") { Core.engine()?.bleClosed(it.toULong()) } } }
        conns.clear()
        runCatching { server?.close() }
        server = null
    }

    /** Starts advertising once the service is added (taken once). */
    private val pendingAdvertiser = java.util.concurrent.atomic.AtomicReference<(() -> Unit)?>(null)
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
            val start = pendingAdvertiser.getAndSet(null)
            if (status == BluetoothGatt.GATT_SUCCESS && server != null) start?.invoke()
            else Log.w(TAG, "service not added ($status); not advertising")
        }

        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            when (newState) {
                BluetoothProfile.STATE_CONNECTED -> {
                    conns[device.address] = Conn(device)
                    Log.i(TAG, "computer connected")
                }
                BluetoothProfile.STATE_DISCONNECTED -> {
                    val c = conns.remove(device.address) ?: return
                    c.linkId?.let { id -> toCore.execute { ffi("bleClosed") { Core.engine()?.bleClosed(id.toULong()) } } }
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
            val failed = synchronized(c) {
                val chunk = c.inFlight
                c.inFlight = null
                if (status == BluetoothGatt.GATT_SUCCESS || chunk == null) {
                    c.sending = false
                    c.retries = 0
                    null
                } else {
                    chunk
                }
            }
            // It didn't arrive: the same chunk again, not the next one.
            if (failed != null) sendFailed(c, failed) else pump(c)
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
        c.linkId = ffi("bleOpened") { engine.bleOpened(out) }?.toLong()
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
        toCore.execute { ffi("bleFrame") { Core.engine()?.bleFrame(id.toULong(), frame) } }
    }

    /** Send the next queued chunk, one notification at a time. A chunk
     *  that couldn't go is put back and tried again shortly: dropping it
     *  (as before) cut a piece out of the frame and corrupted it. */
    private fun pump(c: Conn) {
        val chunk = synchronized(c) {
            if (c.sending || !c.subscribed || c.out.isEmpty()) return
            c.sending = true
            c.out.removeFirst().also { c.inFlight = it }
        }
        val s = server
        val ch = fromPhone
        if (s == null || ch == null || conns[c.device.address] !== c) {
            // Gone (stopped, or the computer left): nothing more goes to it.
            synchronized(c) { c.out.clear(); c.inFlight = null; c.sending = false }
            return
        }
        val sent = runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                s.notifyCharacteristicChanged(c.device, ch, false, chunk) == BluetoothGatt.GATT_SUCCESS
            } else {
                @Suppress("DEPRECATION")
                ch.value = chunk
                @Suppress("DEPRECATION")
                s.notifyCharacteristicChanged(c.device, ch, false)
            }
        }.getOrDefault(false)
        // Sent: onNotificationSent says whether it arrived.
        if (sent) return
        val wasInFlight = synchronized(c) { (c.inFlight === chunk).also { if (it) c.inFlight = null } }
        // Already answered by onNotificationSent (it failed there too).
        if (wasInFlight) sendFailed(c, chunk)
    }

    /** [chunk] didn't go: back at the head of the queue, tried again soon;
     *  past [MAX_RETRIES] the connection is closed. */
    private fun sendFailed(c: Conn, chunk: ByteArray) {
        val s = server
        val giveUp = synchronized(c) {
            c.sending = false
            c.retries++
            if (c.retries > MAX_RETRIES) {
                // The link is stuck. Dropping the rest would leave the
                // computer half a frame, so the connection goes: both sides
                // start clean when it connects again.
                c.out.clear()
                c.retries = 0
                true
            } else {
                c.out.addFirst(chunk)
                false
            }
        }
        if (giveUp) {
            Log.w(TAG, "notifications keep failing: closing the connection")
            runCatching { s?.cancelConnection(c.device) }
        } else {
            retry.postDelayed({ pump(c) }, RETRY_MS)
        }
    }

    private val retry = android.os.Handler(android.os.Looper.getMainLooper())
    /** A chunk that didn't go is tried again this soon, this many times. */
    private const val RETRY_MS = 30L
    private const val MAX_RETRIES = 200

    private fun split(frame: ByteArray, packet: Int): List<ByteArray> {
        val body = (packet - 1).coerceAtLeast(1)
        if (frame.isEmpty()) return listOf(byteArrayOf(1))
        val parts = (frame.indices step body).map { frame.copyOfRange(it, minOf(it + body, frame.size)) }
        return parts.mapIndexed { i, p -> byteArrayOf(if (i == parts.lastIndex) 1 else 0) + p }
    }
}
